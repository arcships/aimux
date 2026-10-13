//! Assemble the assistant message replayed to the model on the next turn.
//!
//! Rust port of the AI SDK's `toResponseMessages` and streamed content
//! assembly. Text and reasoning segments retain their first position while
//! deltas update them by ID. Provider metadata is replayed on the next turn.

use std::collections::HashMap;

use base64::Engine;
use serde_json::Value;

use crate::content::{AssistantContentPart, ToolResultOutput, ToolResultPart};
use crate::message::ModelMessage;
use crate::result::{
    GenerateContent, GeneratedFile, ReasoningFileOutput, ReasoningOutput, ReasoningPart, Source,
    TextContent, ToolApprovalRequestOutput,
};
use crate::shared::{FileBytes, GeneratedFileData, SharedProviderMetadata};

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
    content: Vec<TextContent>,
    text_indexes: HashMap<String, usize>,
    reasoning_indexes: HashMap<String, usize>,
}

pub(crate) struct ResponseMessages {
    pub messages: Vec<ModelMessage>,
    pub content: Vec<TextContent>,
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
            self.content.push(GenerateContent::Text {
                text: String::new(),
                provider_metadata: None,
            });
            index
        });
        if let GenerateContent::Text {
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
            self.content
                .push(GenerateContent::Reasoning(ReasoningOutput {
                    text: String::new(),
                    provider_metadata: None,
                }));
            index
        });
        if let GenerateContent::Reasoning(reasoning) = &mut self.content[index] {
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
        self.content.push(GenerateContent::Text {
            text: text.to_owned(),
            provider_metadata: metadata.cloned(),
        });
    }

    pub fn reasoning(&mut self, reasoning: &ReasoningOutput) {
        self.content
            .push(GenerateContent::Reasoning(reasoning.clone()));
    }

    pub fn custom(&mut self, kind: String, provider_metadata: Option<SharedProviderMetadata>) {
        self.content.push(GenerateContent::Custom {
            kind,
            provider_metadata,
        });
    }

    pub fn file(&mut self, file: &GeneratedFile, reasoning: bool) {
        self.content.push(if reasoning {
            GenerateContent::ReasoningFile(ReasoningFileOutput {
                file: file.clone(),
                provider_metadata: file.provider_metadata.clone(),
            })
        } else {
            GenerateContent::File(file.clone())
        });
    }

    pub fn approval(&mut self, approval: &ToolApprovalRequestOutput) {
        self.content
            .push(GenerateContent::ToolApprovalRequest(approval.clone()));
    }

    pub fn tool_call(&mut self, call: &crate::tool::ToolCall) {
        self.content.push(GenerateContent::ToolCall(call.clone()));
    }

    pub fn tool_result(&mut self, result: crate::tool::ToolResult) {
        if result.preliminary != Some(true) {
            self.content.push(GenerateContent::ToolResult(result));
        }
    }

    pub fn source(&mut self, source: Source) {
        self.content.push(GenerateContent::Source(source));
    }

    pub fn finish(self) -> ResponseMessages {
        let mut parts = Vec::new();
        let mut reasoning = Vec::new();
        for content in &self.content {
            parts.push(match content {
                GenerateContent::Text {
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
                GenerateContent::Reasoning(output) => {
                    reasoning.push(ReasoningPart::Text(output.clone()));
                    AssistantContentPart::Reasoning {
                        text: output.text.clone(),
                        provider_options: output.provider_metadata.clone(),
                    }
                }
                GenerateContent::Custom {
                    kind,
                    provider_metadata,
                } => AssistantContentPart::Custom {
                    kind: kind.clone(),
                    provider_options: provider_metadata.clone(),
                },
                GenerateContent::File(file) => match &file.data {
                    GeneratedFileData::Data { data } => AssistantContentPart::FileBase64 {
                        data: file_base64(data),
                        media_type: file.media_type.clone(),
                        filename: None,
                        provider_options: file.provider_metadata.clone(),
                    },
                    GeneratedFileData::Url { url, .. } => AssistantContentPart::FileUrl {
                        url: url.clone(),
                        media_type: file.media_type.clone(),
                        provider_options: file.provider_metadata.clone(),
                    },
                },
                GenerateContent::ReasoningFile(output) => {
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
                GenerateContent::ToolApprovalRequest(approval) => {
                    AssistantContentPart::ToolApprovalRequest {
                        approval_id: approval.approval_id.clone(),
                        tool_call_id: approval.tool_call.tool_call_id.clone(),
                        reason: approval.reason.clone(),
                        is_automatic: approval.is_automatic,
                        signature: approval.signature.clone(),
                        input_schema_input: None,
                    }
                }
                GenerateContent::ToolCall(call) => AssistantContentPart::ToolCall {
                    tool_call_id: call.tool_call_id.clone(),
                    tool_name: call.tool_name.clone(),
                    input: response_tool_call_input(&call.input, call.invalid),
                    provider_executed: call.provider_executed,
                    provider_options: call.provider_metadata.clone(),
                },
                // A provider-executed result replays as upstream's
                // `createToolModelOutput` builds it: `errorMode: 'none'` for
                // a result, `'json'` for an error
                // (`ai` `generate-text/to-response-messages.ts:103-127`).
                GenerateContent::ToolResult(result) => {
                    AssistantContentPart::ToolResult(ToolResultPart {
                        tool_call_id: result.tool_call_id.clone(),
                        tool_name: Some(result.tool_name.clone()),
                        output: if result.is_error == Some(true) {
                            ToolResultOutput::ErrorJson {
                                value: result.result.clone(),
                                provider_options: None,
                            }
                        } else {
                            ToolResultOutput::from_result(result.result.clone(), false)
                        },
                        provider_options: result.provider_metadata.clone(),
                    })
                }
                GenerateContent::Source(_) => continue,
            });
        }
        let messages = if parts.is_empty() {
            Vec::new()
        } else {
            vec![ModelMessage::assistant_parts(parts)]
        };
        ResponseMessages {
            messages,
            content: self.content,
            reasoning,
        }
    }
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
}
