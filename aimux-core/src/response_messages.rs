//! Assemble the assistant message replayed to the model on the next turn.
//!
//! Rust port of the AI SDK's `toResponseMessages` and streamed content
//! assembly. Text and reasoning segments retain their first position while
//! deltas update them by ID. Provider metadata is replayed on the next turn.

use std::collections::HashMap;

use base64::Engine;
use serde_json::Value;

use crate::content::ContentPart;
use crate::message::{MessageContent, ModelMessage, Role};
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

/// Reasoning signature echoed back on the next turn (Anthropic:
/// `provider_metadata.anthropic.signature`; Bedrock: `.bedrock.signature` /
/// `.amazonBedrock.signature`).
pub(crate) fn extract_reasoning_signature(
    provider_metadata: Option<&SharedProviderMetadata>,
) -> Option<String> {
    let metadata = provider_metadata?;
    ["anthropic", "bedrock", "amazonBedrock"]
        .into_iter()
        .find_map(|ns| metadata.get(ns)?.get("signature")?.as_str())
        .map(str::to_owned)
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
                    ContentPart::Text {
                        text: text.clone(),
                        provider_options: provider_metadata.clone(),
                    }
                }
                GenerateContent::Reasoning(output) => {
                    reasoning.push(ReasoningPart::Text(output.clone()));
                    ContentPart::Reasoning {
                        text: output.text.clone(),
                        signature: extract_reasoning_signature(output.provider_metadata.as_ref()),
                        provider_options: output.provider_metadata.clone(),
                    }
                }
                GenerateContent::Custom {
                    kind,
                    provider_metadata,
                } => ContentPart::Custom {
                    kind: kind.clone(),
                    provider_options: provider_metadata.clone(),
                },
                GenerateContent::File(file) => match &file.data {
                    GeneratedFileData::Data { data } => ContentPart::FileBase64 {
                        data: file_base64(data),
                        media_type: file.media_type.clone(),
                        filename: None,
                        provider_options: file.provider_metadata.clone(),
                    },
                    GeneratedFileData::Url { url, .. } => ContentPart::FileUrl {
                        url: url.clone(),
                        media_type: file.media_type.clone(),
                        provider_options: file.provider_metadata.clone(),
                    },
                },
                GenerateContent::ReasoningFile(output) => {
                    reasoning.push(ReasoningPart::File(output.clone()));
                    ContentPart::ReasoningFile {
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
                    ContentPart::ToolApprovalRequest {
                        approval_id: approval.approval_id.clone(),
                        tool_call_id: approval.tool_call.tool_call_id.clone(),
                        reason: approval.reason.clone(),
                        is_automatic: approval.is_automatic,
                        signature: approval.signature.clone(),
                        input_schema_input: None,
                    }
                }
                GenerateContent::ToolCall(call) => ContentPart::ToolCall {
                    tool_call_id: call.tool_call_id.clone(),
                    tool_name: call.tool_name.clone(),
                    input: response_tool_call_input(&call.input, call.invalid),
                    provider_executed: call.provider_executed,
                    provider_options: call.provider_metadata.clone(),
                },
                GenerateContent::ToolResult(result) => ContentPart::ToolResult {
                    tool_call_id: result.tool_call_id.clone(),
                    tool_name: Some(result.tool_name.clone()),
                    result: result.result.clone(),
                    is_error: result.is_error,
                    preliminary: result.preliminary,
                    dynamic: result.dynamic,
                    provider_options: result.provider_metadata.clone(),
                },
                GenerateContent::Source(_) => continue,
            });
        }
        let messages = if parts.is_empty() {
            Vec::new()
        } else {
            vec![ModelMessage {
                role: Role::Assistant,
                content: MessageContent::Parts(parts),
            }]
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
