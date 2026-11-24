//! DeepSeek's stateless Responses extension. Raw output is opaque replay data.

use std::collections::BTreeMap;

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::result::Source;
use aimux_core::shared::provider_namespace;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{RawToolCall, Tool, ToolResult};
use aimux_core::types::ProviderMetadata;
use serde_json::{Value, json};

use super::convert_to_input::convert_to_open_responses_input_with_namespace;
use crate::stream::Controller;

const ENDPOINT: &str = "https://api.deepseek.com/responses";
const MODEL: &str = "deepseek-flash";
const TOOL: &str = "deepseek_web_search";

/// Validate opt-in and replay even when search is off; never infer activation
/// from a provider name alone (another server may use the same name).
pub(super) fn prepare(
    model: &str,
    endpoint: &str,
    namespace: &str,
    options: &CallOptions,
    body: &mut Value,
) -> Result<bool, AiMuxError> {
    let compatible = model == MODEL && endpoint == ENDPOINT;
    let option = options
        .provider_options
        .as_ref()
        .and_then(|options| options.get(namespace))
        .and_then(|options| options.get("deepseekWebSearch"));
    let enabled = match option {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return Err(AiMuxError::invalid_argument(
                "deepseekWebSearch must be a boolean",
            ));
        }
    };
    if enabled {
        if !compatible {
            return Err(AiMuxError::invalid_argument(
                "DeepSeek web search requires deepseek-flash at https://api.deepseek.com/responses",
            ));
        }
        let legacy_web_search = options
            .tools
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|tool| matches!(tool,
                Tool::Provider(tool) if matches!(tool.id.as_str().rsplit('.').next(), Some("web_search" | "web_search_preview"))
            ));
        if legacy_web_search {
            return Err(AiMuxError::invalid_argument(
                "deepseekWebSearch cannot be combined with the legacy web_search tool",
            ));
        }
        if body.get("tools").is_none() {
            body["tools"] = json!([]);
        }
        // A caller-supplied `tools` value that is not an array is an argument
        // error, not a reason to panic.
        let tools = body
            .get_mut("tools")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| {
                AiMuxError::invalid_argument(
                    "deepseekWebSearch requires request tools to be an array",
                )
            })?;
        tools.push(json!({"type": TOOL}));
    }

    // Each replay belongs at one assistant message's position, rather than
    // being prepended to the conversation or appended after the new question.
    if !options
        .prompt
        .iter()
        .any(|message| response_output(message, namespace).is_some())
    {
        return Ok(compatible);
    }
    let mut input = Vec::new();
    for message in &options.prompt {
        if let Some(replay) = response_output(message, namespace) {
            if !compatible || replay["modelId"] != MODEL || replay["endpoint"] != ENDPOINT {
                return Err(AiMuxError::invalid_argument(
                    "responseOutput must come from a compatible DeepSeek Responses request",
                ));
            }
            let items = replay["items"].as_array().ok_or_else(|| {
                AiMuxError::invalid_argument("responseOutput.items must be an array")
            })?;
            for item in items {
                if !item.is_object()
                    || !matches!(
                        item["type"].as_str(),
                        Some("reasoning" | "message" | "function_call" | "web_search_call")
                    )
                    || (item["type"] == "message" && item["role"] != "assistant")
                {
                    return Err(AiMuxError::invalid_argument(
                        "responseOutput contains an invalid assistant output item",
                    ));
                }
            }
            input.extend(items.iter().cloned());
        } else {
            let (converted, _, _) =
                convert_to_open_responses_input_with_namespace(&vec![message.clone()], namespace);
            input.extend(converted.as_array().into_iter().flatten().cloned());
        }
    }
    body["input"] = Value::Array(input);
    Ok(compatible)
}

fn response_output<'a>(message: &'a LanguageModelMessage, namespace: &str) -> Option<&'a Value> {
    match message {
        LanguageModelMessage::Assistant {
            provider_options, ..
        } => provider_options
            .as_ref()
            .and_then(|options| options.get(namespace))
            .and_then(|options| options.get("responseOutput")),
        _ => None,
    }
}

/// A completed server search is a provider-executed call and result. Returning
/// the original item preserves status, sources, page and error extensions.
pub(super) fn search_parts(item: &Value, namespace: &str, index: u64) -> (RawToolCall, ToolResult) {
    let id = item["id"]
        .as_str()
        .map_or_else(|| format!("deepseek-search-{index}"), str::to_owned);
    let metadata = provider_namespace(namespace, json!({"outputIndex": index})).ok();
    (
        RawToolCall {
            tool_call_id: id.clone(),
            tool_name: TOOL.into(),
            input: item.get("action").unwrap_or(&json!({})).to_string(),
            provider_executed: Some(true),
            dynamic: Some(true),
            provider_metadata: metadata.clone(),
        },
        ToolResult {
            tool_call_id: id,
            tool_name: TOOL.into(),
            result: item.clone(),
            is_error: Some(
                item["status"] == "failed"
                    || item.get("error").is_some_and(|value| !value.is_null()),
            ),
            preliminary: None,
            dynamic: Some(true),
            provider_metadata: metadata,
        },
    )
}

/// A finished item can omit its id, or send `null`, while
/// `response.output_item.added` carries it. Patch the item before it is both
/// published and stored, so the emitted call id and the persisted replay
/// record agree.
fn recover_id(item: &mut Value, index: u64, added: &BTreeMap<u64, Value>) {
    if !item.is_object() {
        return;
    }
    if item["id"].as_str().is_none()
        && let Some(id) = added.get(&index).and_then(|added| added["id"].as_str())
    {
        item["id"] = Value::String(id.to_owned());
    }
}

pub(super) fn metadata(namespace: &str, items: Value) -> Option<ProviderMetadata> {
    provider_namespace(
        namespace,
        json!({"responseOutput": {
            "modelId": MODEL, "endpoint": ENDPOINT, "items": items,
        }}),
    )
    .ok()
}

#[derive(Default)]
pub(super) struct DeepSeekStream {
    pub enabled: bool,
    // Completion order can differ from output order. Only completed items are
    // replayable; `added` items supply identity if a done or terminal item
    // omits it.
    added: BTreeMap<u64, Value>,
    // The first item reported for each output index. It decides publication
    // only; the replay record uses it solely when the stream carries no
    // terminal `output` array.
    reported: BTreeMap<u64, Value>,
    // The terminal `response.output` array in provider order. When present,
    // including when it is empty, it alone is the replay record.
    terminal: Option<Vec<Value>>,
}

impl DeepSeekStream {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Self::default()
        }
    }

    /// Publish one finished item once per output index, and return it with its
    /// id recovered, so a replay record stores the same identity it emitted.
    /// Both `response.output_item.done` and the terminal `response.output`
    /// report items, so one that only appears in the latter still reaches the
    /// caller. The first report of an index wins for publication; a repeated
    /// one is the same item, not a second call. The replay record does not use
    /// this map, because the terminal array is authoritative for it.
    fn report(
        &mut self,
        index: u64,
        mut item: Value,
        namespace: &str,
        controller: &mut Controller,
    ) -> Value {
        recover_id(&mut item, index, &self.added);
        self.reported.entry(index).or_insert_with(|| {
            if item["type"] == "web_search_call" {
                let (call, result) = search_parts(&item, namespace, index);
                controller.enqueue(StreamPart::ToolCall(call));
                controller.enqueue(StreamPart::ToolResult(result));
            }
            item.clone()
        });
        item
    }

    /// Match every terminal item with the added item it belongs to. A filtered
    /// array keeps the provider order but not the earlier `output_index`
    /// positions, so items are consumed in order, preferring an equal id, and
    /// fall back to their own position when no added item is left.
    fn align(&self, items: &[Value]) -> Vec<u64> {
        let added = self.added.iter().collect::<Vec<_>>();
        let mut cursor = 0;
        let mut indexes = Vec::with_capacity(items.len());
        for (position, item) in items.iter().enumerate() {
            let remaining = &added[cursor..];
            let found = remaining.iter().position(|(_, candidate)| {
                candidate["type"] == item["type"]
                    && match (item["id"].as_str(), candidate["id"].as_str()) {
                        (Some(item_id), Some(added_id)) => item_id == added_id,
                        _ => true,
                    }
            });
            if let Some(offset) = found {
                indexes.push(*remaining[offset].0);
                cursor += offset + 1;
            } else {
                indexes.push(position as u64);
            }
        }
        indexes
    }

    pub fn observe(&mut self, event: &Value, namespace: &str, controller: &mut Controller) {
        if !self.enabled {
            return;
        }
        match event["type"].as_str() {
            Some("response.output_item.added") => {
                if let Some(index) = event["output_index"].as_u64() {
                    self.added.insert(index, event["item"].clone());
                }
            }
            Some("response.output_item.done") => {
                if let Some(index) = event["output_index"].as_u64() {
                    self.report(index, event["item"].clone(), namespace, controller);
                }
            }
            Some("response.output_text.annotation.added") => {
                let citation = &event["annotation"];
                if citation["type"] == "url_citation"
                    && let Some(url) = citation["url"].as_str()
                {
                    let index = event["output_index"].as_u64().unwrap_or(0);
                    let content = event["content_index"].as_u64().unwrap_or(0);
                    let annotation = event["annotation_index"].as_u64().unwrap_or(0);
                    controller.enqueue(StreamPart::Source(Source::Url {
                        id: format!("deepseek-{index}-{content}-{annotation}"),
                        url: url.to_owned(), title: citation["title"].as_str().map(str::to_owned),
                        provider_metadata: provider_namespace(namespace, json!({
                            "itemId": event["item_id"], "outputIndex": index,
                            "contentIndex": content, "annotationIndex": annotation,
                            "startIndex": citation["start_index"], "endIndex": citation["end_index"],
                            "offsetUnit": "unicode-code-point",
                        })).ok(),
                    }));
                }
            }
            Some("response.completed" | "response.incomplete") => {
                let Some(items) = event["response"]["output"].as_array() else {
                    return;
                };
                // The terminal array is the replay record, even when a filtered
                // array drops items and its positions no longer match the
                // earlier output indexes.
                let indexes = self.align(items);
                let mut replay = Vec::with_capacity(items.len());
                for (position, item) in items.iter().enumerate() {
                    replay.push(self.report(
                        indexes[position],
                        item.clone(),
                        namespace,
                        controller,
                    ));
                }
                self.terminal = Some(replay);
            }
            _ => {}
        }
    }

    pub fn metadata(self, namespace: &str) -> Option<ProviderMetadata> {
        if !self.enabled {
            return None;
        }
        // A terminal array is authoritative for replay, including when the
        // provider emptied it; only a stream that never carries one falls back
        // to the items reported along the way.
        let items = match self.terminal {
            Some(items) => items,
            None => self.reported.into_values().collect(),
        };
        metadata(namespace, json!(items))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_options() -> CallOptions {
        let mut options = CallOptions::new(vec![LanguageModelMessage::user_text("q")]);
        options.provider_options =
            Some(serde_json::from_value(json!({"deepseek": {"deepseekWebSearch": true}})).unwrap());
        options
    }

    // A request whose `tools` value is not an array is an argument error, not
    // a panic.
    #[test]
    fn non_array_request_tools_is_an_invalid_argument() {
        let mut body = json!({"model": MODEL, "tools": "deepseek_web_search"});
        let error = prepare(MODEL, ENDPOINT, "deepseek", &enabled_options(), &mut body)
            .expect_err("a non-array tools value is rejected");
        assert!(
            matches!(&error, AiMuxError::InvalidArgument { message, .. }
                if message == "deepseekWebSearch requires request tools to be an array"),
            "unexpected error: {error:?}"
        );
    }
}
