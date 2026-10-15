//! Assemble the assistant message replayed to the model on the next turn.
//!
//! Rust port of the AI SDK's `toResponseMessages` and streamed content
//! assembly. Text and reasoning segments retain their first position while
//! deltas update them by ID. Provider metadata is replayed on the next turn.

use std::collections::HashMap;

use base64::Engine;
use serde_json::Value;

use crate::content::{AssistantContentPart, ToolContentPart, ToolResultOutput, ToolResultPart};
use crate::message::ModelMessage;
use crate::result::{
    ContentPart, GeneratedFile, ReasoningFileOutput, ReasoningOutput, ReasoningPart, Source,
    ToolApprovalRequestOutput, TypedToolError, TypedToolResult,
};
use crate::shared::{FileBytes, GeneratedFileData, SharedProviderMetadata};
use crate::types::NamespacedKind;

/// Match the AI SDK's response-message safety rule for invalid tool calls:
/// malformed primitive input must not be replayed as a prompt tool-call input.
/// JavaScript's `typeof value === "object"` includes arrays and null, so those
/// values are intentionally retained here as well.
pub(crate) fn response_tool_call_input(input: &Value, invalid: Option<bool>) -> Value {
    if invalid == Some(true) && !matches!(input, Value::Object(_) | Value::Array(_) | Value::Null) {
        Value::Object(serde_json::Map::new())
    } else {
        input.clone()
    }
}

/// Accumulates generated content in provider order, updating streamed segments by ID.
#[derive(Default)]
pub(crate) struct ResponseMessageBuilder {
    content: Vec<ContentPart>,
    text_indexes: HashMap<String, usize>,
    reasoning_indexes: HashMap<String, usize>,
}

pub(crate) struct ResponseMessages {
    pub messages: Vec<ModelMessage>,
    pub content: Vec<ContentPart>,
    pub reasoning: Vec<ReasoningPart>,
}

impl ResponseMessageBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text_start(&mut self, id: String, metadata: Option<SharedProviderMetadata>) {
        self.text_delta(id, "", metadata);
    }

    pub fn text_delta(
        &mut self,
        id: String,
        delta: &str,
        metadata: Option<SharedProviderMetadata>,
    ) {
        let index = *self.text_indexes.entry(id).or_insert_with(|| {
            let index = self.content.len();
            self.content.push(ContentPart::Text {
                text: String::new(),
                provider_metadata: None,
            });
            index
        });
        if let ContentPart::Text {
            text,
            provider_metadata,
        } = &mut self.content[index]
        {
            text.push_str(delta);
            if metadata.is_some() {
                *provider_metadata = metadata;
            }
        }
    }

    pub fn text_end(&mut self, id: String, metadata: Option<SharedProviderMetadata>) {
        self.text_delta(id.clone(), "", metadata);
        self.text_indexes.remove(&id);
    }

    pub fn reasoning_start(&mut self, id: String, metadata: Option<SharedProviderMetadata>) {
        self.reasoning_delta(id, "", metadata);
    }

    pub fn reasoning_delta(
        &mut self,
        id: String,
        delta: &str,
        metadata: Option<SharedProviderMetadata>,
    ) {
        let index = *self.reasoning_indexes.entry(id).or_insert_with(|| {
            let index = self.content.len();
            self.content.push(ContentPart::Reasoning(ReasoningOutput {
                text: String::new(),
                provider_metadata: None,
            }));
            index
        });
        if let ContentPart::Reasoning(reasoning) = &mut self.content[index] {
            reasoning.text.push_str(delta);
            if metadata.is_some() {
                reasoning.provider_metadata = metadata;
            }
        }
    }

    pub fn reasoning_end(&mut self, id: String, metadata: Option<SharedProviderMetadata>) {
        self.reasoning_delta(id.clone(), "", metadata);
        self.reasoning_indexes.remove(&id);
    }

    pub fn text(&mut self, text: &str, metadata: Option<&SharedProviderMetadata>) {
        self.content.push(ContentPart::Text {
            text: text.to_owned(),
            provider_metadata: metadata.cloned(),
        });
    }

    pub fn reasoning(&mut self, reasoning: &ReasoningOutput) {
        self.content.push(ContentPart::Reasoning(reasoning.clone()));
    }

    pub fn custom(
        &mut self,
        kind: NamespacedKind,
        provider_metadata: Option<SharedProviderMetadata>,
    ) {
        self.content.push(ContentPart::Custom {
            kind,
            provider_metadata,
        });
    }

    /// A generated file, wrapped as `{ file, providerMetadata }`
    /// (`convert-language-model-content.ts:55-73`).
    pub fn file(&mut self, file: &GeneratedFile, reasoning: bool) {
        self.content.push(if reasoning {
            ContentPart::ReasoningFile(ReasoningFileOutput {
                file: file.clone(),
                provider_metadata: file.provider_metadata.clone(),
            })
        } else {
            ContentPart::File {
                file: file.clone(),
                provider_metadata: file.provider_metadata.clone(),
            }
        });
    }

    pub fn approval(&mut self, approval: &ToolApprovalRequestOutput) {
        self.content
            .push(ContentPart::ToolApprovalRequest(approval.clone()));
    }

    pub fn tool_call(&mut self, call: &crate::tool::ToolCall) {
        self.content.push(ContentPart::ToolCall(call.clone()));
    }

    /// A provider-executed tool result: a tool result, or a tool error when
    /// the provider marks it as one, with the input of the call it answers
    /// (`convert-language-model-content.ts:87-176`). A result with no call
    /// in this response is a deferred result and has no input.
    pub fn tool_result(&mut self, result: crate::tool::ToolResult) {
        if result.preliminary == Some(true) {
            return;
        }
        let call = self.content.iter().find_map(|part| match part {
            ContentPart::ToolCall(call) if call.tool_call_id == result.tool_call_id => Some(call),
            _ => None,
        });
        let input = call.map(|call| call.input.clone());
        let dynamic = match call {
            Some(call) => call.dynamic,
            None => result.dynamic,
        };
        self.content.push(if result.is_error == Some(true) {
            ContentPart::ToolError(TypedToolError {
                tool_call_id: result.tool_call_id,
                tool_name: result.tool_name,
                input,
                error: result.result,
                provider_executed: Some(true),
                provider_metadata: result.provider_metadata,
                dynamic,
            })
        } else {
            ContentPart::ToolResult(TypedToolResult {
                tool_call_id: result.tool_call_id,
                tool_name: result.tool_name,
                input,
                output: result.result,
                provider_executed: Some(true),
                provider_metadata: result.provider_metadata,
                dynamic,
                preliminary: None,
            })
        });
    }

    /// A tool error produced by the call itself, such as the failure of a
    /// tool call that could not be parsed.
    pub fn tool_error(&mut self, error: TypedToolError) {
        self.content.push(ContentPart::ToolError(error));
    }

    pub fn source(&mut self, source: Source) {
        self.content.push(ContentPart::Source(source));
    }

    /// The response messages, as upstream's `toResponseMessages` builds them
    /// (`ai` `generate-text/to-response-messages.ts:17-228`): one assistant
    /// message with the model's content and its provider-executed tool
    /// results, then one tool message with the other tool results, tool
    /// errors and approval responses, ordered by tool call.
    pub fn finish(self) -> ResponseMessages {
        let mut parts = Vec::new();
        let mut reasoning = Vec::new();
        let mut tool_call_order = HashMap::new();
        for content in &self.content {
            parts.push(match content {
                ContentPart::Text {
                    text,
                    provider_metadata,
                } => {
                    if text.is_empty() {
                        continue;
                    }
                    AssistantContentPart::Text {
                        text: text.clone(),
                        provider_options: provider_metadata.clone(),
                    }
                }
                ContentPart::Reasoning(output) => {
                    reasoning.push(ReasoningPart::Text(output.clone()));
                    AssistantContentPart::Reasoning {
                        text: output.text.clone(),
                        provider_options: output.provider_metadata.clone(),
                    }
                }
                ContentPart::Custom {
                    kind,
                    provider_metadata,
                } => AssistantContentPart::Custom {
                    kind: kind.clone(),
                    provider_options: provider_metadata.clone(),
                },
                ContentPart::File {
                    file,
                    provider_metadata,
                } => match &file.data {
                    GeneratedFileData::Data { data } => AssistantContentPart::FileBase64 {
                        data: file_base64(data),
                        media_type: file.media_type.clone(),
                        filename: None,
                        provider_options: provider_metadata.clone(),
                    },
                    GeneratedFileData::Url { url, .. } => AssistantContentPart::FileUrl {
                        url: url.clone(),
                        media_type: file.media_type.clone(),
                        provider_options: provider_metadata.clone(),
                    },
                },
                ContentPart::ReasoningFile(output) => {
                    reasoning.push(ReasoningPart::File(output.clone()));
                    AssistantContentPart::ReasoningFile {
                        data: match &output.file.data {
                            GeneratedFileData::Data { data } => GeneratedFileData::Data {
                                data: FileBytes::Base64(file_base64(data)),
                            },
                            url => url.clone(),
                        },
                        media_type: output.file.media_type.clone(),
                        provider_options: output.provider_metadata.clone(),
                    }
                }
                ContentPart::ToolApprovalRequest(approval) => {
                    AssistantContentPart::ToolApprovalRequest {
                        approval_id: approval.approval_id.clone(),
                        tool_call_id: approval.tool_call.tool_call_id.clone(),
                        reason: approval.reason.clone(),
                        is_automatic: approval.is_automatic,
                        signature: approval.signature.clone(),
                        input_schema_input: None,
                    }
                }
                ContentPart::ToolCall(call) => {
                    let order = tool_call_order.len();
                    tool_call_order
                        .entry(call.tool_call_id.clone())
                        .or_insert(order);
                    AssistantContentPart::ToolCall {
                        tool_call_id: call.tool_call_id.clone(),
                        tool_name: call.tool_name.clone(),
                        input: response_tool_call_input(&call.input, call.invalid),
                        provider_executed: call.provider_executed,
                        provider_options: call.provider_metadata.clone(),
                    }
                }
                // A provider-executed result replays as upstream's
                // `createToolModelOutput` builds it: `errorMode: 'none'` for
                // a result, `'json'` for an error (`:99-131`).
                ContentPart::ToolResult(result) if result.provider_executed == Some(true) => {
                    AssistantContentPart::ToolResult(ToolResultPart {
                        tool_call_id: result.tool_call_id.clone(),
                        tool_name: Some(result.tool_name.clone()),
                        output: ToolResultOutput::from_result(result.output.clone(), false),
                        provider_options: result.provider_metadata.clone(),
                    })
                }
                ContentPart::ToolError(error) if error.provider_executed == Some(true) => {
                    AssistantContentPart::ToolResult(ToolResultPart {
                        tool_call_id: error.tool_call_id.clone(),
                        tool_name: Some(error.tool_name.clone()),
                        output: ToolResultOutput::ErrorJson {
                            value: error.error.clone(),
                            provider_options: None,
                        },
                        provider_options: error.provider_metadata.clone(),
                    })
                }
                ContentPart::Source(_)
                | ContentPart::ToolResult(_)
                | ContentPart::ToolError(_)
                | ContentPart::ToolApprovalResponse(_) => continue,
            });
        }
        let mut messages = Vec::new();
        if !parts.is_empty() {
            messages.push(ModelMessage::assistant_parts(parts));
        }
        let tool_content = tool_message_content(&self.content, &tool_call_order);
        if !tool_content.is_empty() {
            messages.push(ModelMessage::tool(tool_content));
        }
        ResponseMessages {
            messages,
            content: self.content,
            reasoning,
        }
    }
}

/// The tool message of the response (`to-response-messages.ts:158-225`):
/// approval responses (a denied one followed by an `execution-denied`
/// result), then the results and errors of tools the provider did not
/// execute, an error as error text. The tool results are put in tool call
/// order, results of unknown calls last, the rest keeping their places
/// (`sortToolResultContentByToolCallOrder`, `:230-265`).
fn tool_message_content(
    content: &[ContentPart],
    tool_call_order: &HashMap<String, usize>,
) -> Vec<ToolContentPart> {
    let mut parts = Vec::new();
    for part in content {
        match part {
            ContentPart::ToolApprovalResponse(response) => {
                parts.push(ToolContentPart::ToolApprovalResponse {
                    approval_id: response.approval_id.clone(),
                    approved: response.approved,
                    reason: response.reason.clone(),
                    provider_executed: response.provider_executed,
                });
                if !response.approved {
                    parts.push(ToolContentPart::ToolResult(ToolResultPart {
                        tool_call_id: response.tool_call.tool_call_id.clone(),
                        tool_name: Some(response.tool_call.tool_name.clone()),
                        output: ToolResultOutput::ExecutionDenied {
                            reason: response.reason.clone(),
                            provider_options: None,
                        },
                        provider_options: None,
                    }));
                }
            }
            ContentPart::ToolResult(result) if result.provider_executed != Some(true) => {
                parts.push(ToolContentPart::ToolResult(ToolResultPart {
                    tool_call_id: result.tool_call_id.clone(),
                    tool_name: Some(result.tool_name.clone()),
                    output: ToolResultOutput::from_result(result.output.clone(), false),
                    provider_options: result.provider_metadata.clone(),
                }));
            }
            ContentPart::ToolError(error) if error.provider_executed != Some(true) => {
                parts.push(ToolContentPart::ToolResult(ToolResultPart {
                    tool_call_id: error.tool_call_id.clone(),
                    tool_name: Some(error.tool_name.clone()),
                    output: ToolResultOutput::from_result(error.error.clone(), true),
                    provider_options: error.provider_metadata.clone(),
                }));
            }
            _ => {}
        }
    }

    let mut results: Vec<(usize, ToolContentPart)> = parts
        .iter()
        .filter(|part| matches!(part, ToolContentPart::ToolResult(_)))
        .cloned()
        .enumerate()
        .collect();
    let order = |part: &ToolContentPart| match part {
        ToolContentPart::ToolResult(result) => tool_call_order.get(&result.tool_call_id).copied(),
        ToolContentPart::ToolApprovalResponse { .. } => None,
    };
    results.sort_by(|(a_index, a), (b_index, b)| match (order(a), order(b)) {
        (None, None) => a_index.cmp(b_index),
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(a_order), Some(b_order)) => a_order.cmp(&b_order).then(a_index.cmp(b_index)),
    });
    let mut sorted = results.into_iter().map(|(_, part)| part);
    parts
        .into_iter()
        .map(|part| match part {
            ToolContentPart::ToolResult(_) => sorted.next().expect("one sorted result per result"),
            part => part,
        })
        .collect()
}

fn file_base64(data: &FileBytes) -> String {
    match data {
        FileBytes::Base64(data) => data.clone(),
        FileBytes::Binary(data) => base64::engine::general_purpose::STANDARD.encode(data),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // The AI SDK's `ReasoningPart` is `{ type, text, providerOptions? }`
    // (`@ai-sdk/provider-utils@5.0.53` `types/content-part.ts:107`), and
    // `toResponseMessages` replays a reasoning output as
    // `{ type: 'reasoning', text, providerOptions: part.providerMetadata }`
    // (`ai@7.0.127` `generate-text/to-response-messages.ts:62`). A provider's
    // signature stays in its namespace; there is no top-level signature.
    #[test]
    fn reasoning_signature_is_replayed_only_in_provider_options() {
        let metadata =
            crate::shared::provider_namespace("anthropic", json!({ "signature": "sig-1" }))
                .unwrap();
        let mut builder = ResponseMessageBuilder::new();
        builder.reasoning(&ReasoningOutput {
            text: "thinking".into(),
            provider_metadata: Some(metadata),
        });
        let messages = builder.finish().messages;
        assert_eq!(
            serde_json::to_value(&messages).unwrap(),
            json!([{
                "role": "assistant",
                "content": [{
                    "type": "reasoning",
                    "text": "thinking",
                    "provider_options": { "anthropic": { "signature": "sig-1" } }
                }]
            }])
        );
    }

    // A part's provider metadata is replayed whole into the response
    // message's providerOptions (RFC-0042): unknown and empty namespaces and
    // nested nulls survive, in order, on the one-shot text path and on the
    // streamed reasoning path, where the metadata arrives on the end event.
    // Comparing serialized text pins key order, which `Value` equality on an
    // order-preserving map does not.
    #[test]
    fn part_metadata_replays_whole_into_provider_options() {
        let metadata: SharedProviderMetadata = serde_json::from_value(json!({
            "zz-gateway": { "trace": { "id": null, "hops": [1, null, { "x": null }] } },
            "anthropic": { "signature": "sig-1" },
            "acme": {}
        }))
        .unwrap();
        let mut builder = ResponseMessageBuilder::new();
        builder.text("hi", Some(&metadata));
        builder.reasoning_start("r1".into(), None);
        builder.reasoning_delta("r1".into(), "thinking", None);
        builder.reasoning_end("r1".into(), Some(metadata.clone()));
        let messages = builder.finish().messages;

        let expected = serde_json::to_string(&metadata).unwrap();
        let replayed = serde_json::to_string(&messages).unwrap();
        assert_eq!(replayed.matches(expected.as_str()).count(), 2, "{replayed}");
    }
}
