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
        if options.tools.iter().flatten().any(|tool| matches!(tool,
            Tool::Provider(tool) if matches!(tool.id.as_str().rsplit('.').next(), Some("web_search" | "web_search_preview"))
        )) {
            return Err(AiMuxError::invalid_argument("deepseekWebSearch cannot be combined with the legacy web_search tool"));
        }
        if body.get("tools").is_none() {
            body["tools"] = json!([]);
        }
        body["tools"]
            .as_array_mut()
            .expect("request tools array")
            .push(json!({"type": TOOL}));
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
    // replayable; `added` items supply identity if a done event omits it.
    added: BTreeMap<u64, Value>,
    output: BTreeMap<u64, Value>,
}

impl DeepSeekStream {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Self::default()
        }
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
                    let mut item = event["item"].clone();
                    if item["type"] == "web_search_call" {
                        if item.get("id").is_none()
                            && let Some(id) = self.added.get(&index).and_then(|item| item.get("id"))
                        {
                            item["id"] = id.clone();
                        }
                        let (call, result) = search_parts(&item, namespace, index);
                        controller.enqueue(StreamPart::ToolCall(call));
                        controller.enqueue(StreamPart::ToolResult(result));
                    }
                    self.output.insert(index, event["item"].clone());
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
                for (index, item) in event["response"]["output"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    self.output
                        .entry(index as u64)
                        .or_insert_with(|| item.clone());
                }
            }
            _ => {}
        }
    }

    pub fn metadata(self, namespace: &str) -> Option<ProviderMetadata> {
        self.enabled
            .then(|| {
                metadata(
                    namespace,
                    json!(self.output.into_values().collect::<Vec<_>>()),
                )
            })
            .flatten()
    }
}
