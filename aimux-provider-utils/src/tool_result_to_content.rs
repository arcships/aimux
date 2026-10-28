//! The string a chat-style provider sends as a tool message's content.
//!
//! Upstream has no shared function for this: each chat converter
//! (`convert-to-openai-chat-messages.ts`, `convert-to-mistral-chat-messages.ts`,
//! `convert-to-groq-chat-messages.ts`, …) repeats the same `switch` over the
//! tool result output. Eight vendors here share one copy, so it lives in
//! provider-utils (RFC-0046 §3.2).

use aimux_core::language_model_message::{ToolResultContent, ToolResultOutput};
use aimux_core::shared::{FileBytes, FileData};
use serde_json::{Value, json};

/// The tool message content upstream's chat converters build: the text of a
/// text or error-text output, the denial reason (or `Tool call execution
/// denied.`), or `JSON.stringify` of a JSON, error-JSON or content output.
#[must_use]
pub fn tool_result_to_content(output: &ToolResultOutput) -> Value {
    Value::String(match output {
        ToolResultOutput::Text { value, .. } | ToolResultOutput::ErrorText { value, .. } => {
            value.clone()
        }
        ToolResultOutput::ExecutionDenied { reason, .. } => reason
            .clone()
            .unwrap_or_else(|| "Tool call execution denied.".to_string()),
        ToolResultOutput::Json { value, .. } | ToolResultOutput::ErrorJson { value, .. } => {
            value.to_string()
        }
        ToolResultOutput::Content { value } => tool_result_content_value(value).to_string(),
    })
}

/// The content parts of a `content` output as upstream serializes them with
/// `JSON.stringify`: camelCase keys, and binary file data as an object keyed by
/// byte index, the way a `Uint8Array` serializes.
#[must_use]
pub fn tool_result_content_value(content: &[ToolResultContent]) -> Value {
    json!(
        content
            .iter()
            .map(|part| {
                let (mut item, provider_options) = match part {
                    ToolResultContent::Text(part) => (
                        json!({"type":"text", "text":part.text}),
                        &part.provider_options,
                    ),
                    ToolResultContent::File(part) => {
                        let data = match &part.data {
                            FileData::Data { data } => {
                                let data = match data {
                                    FileBytes::Base64(data) => json!(data),
                                    FileBytes::Binary(bytes) => Value::Object(
                                        bytes
                                            .iter()
                                            .enumerate()
                                            .map(|(i, byte)| (i.to_string(), json!(byte)))
                                            .collect(),
                                    ),
                                };
                                json!({"type":"data", "data":data})
                            }
                            FileData::Url { url, original_url } => {
                                let mut data = json!({"type":"url", "url":url});
                                if let Some(original_url) = original_url {
                                    data["originalUrl"] = json!(original_url);
                                }
                                data
                            }
                            FileData::Reference { reference } => {
                                json!({"type":"reference", "reference":reference})
                            }
                            FileData::Text { text } => json!({"type":"text", "text":text}),
                        };
                        let mut item =
                            json!({"type":"file", "data":data, "mediaType":part.media_type});
                        if let Some(filename) = &part.filename {
                            item["filename"] = json!(filename);
                        }
                        (item, &part.provider_options)
                    }
                    ToolResultContent::Custom { provider_options } => {
                        (json!({"type":"custom"}), provider_options)
                    }
                };
                if let Some(provider_options) = provider_options {
                    item["providerOptions"] = json!(provider_options);
                }
                item
            })
            .collect::<Vec<_>>()
    )
}
