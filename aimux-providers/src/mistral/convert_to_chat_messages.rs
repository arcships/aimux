//! `convert-to-mistral-chat-messages.ts`: `LanguageModelPrompt` to the
//! Mistral `messages` array.

use aimux_core::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultPart, UserPart,
};
use aimux_core::shared::{FileBytes, FileData};
use aimux_provider_utils::{get_top_level_media_type, resolve_full_media_type};
use serde_json::{Value, json};

// ── Message conversion ──────────────────────────────────────────────────────

/// Convert provider prompt parts using the upstream Mistral message format.
///
/// # Errors
/// Rejects file formats and assistant content unsupported by Mistral.
pub fn convert_prompt_to_mistral_messages(
    prompt: &LanguageModelPrompt,
) -> Result<Vec<Value>, AiMuxError> {
    let mut messages = Vec::new();
    for (index, message) in prompt.iter().enumerate() {
        match message {
            LanguageModelMessage::System { content, .. } => {
                messages.push(json!({"role":"system", "content": content}))
            }
            LanguageModelMessage::User { content, .. } => {
                let parts = content
                    .iter()
                    .map(convert_user_part)
                    .collect::<Result<Vec<_>, _>>()?;
                messages.push(json!({"role":"user", "content":parts}));
            }
            LanguageModelMessage::Assistant { content, .. } => {
                let mut text = String::new();
                let mut parts = Vec::new();
                let mut calls = Vec::new();
                let mut has_reasoning = false;
                for part in content {
                    match part {
                        AssistantPart::Text(part) => {
                            text.push_str(&part.text);
                            parts.push(json!({"type":"text", "text":part.text}));
                        }
                        AssistantPart::Reasoning(ReasoningPart { text, .. }) => {
                            has_reasoning = true;
                            parts.push(json!({"type":"thinking", "thinking":[{"type":"text", "text":text}], "closed":true}));
                        }
                        AssistantPart::ToolCall(ToolCallPart { tool_call_id, tool_name, input, .. }) => calls.push(json!({
                            "id":tool_call_id, "type":"function", "function":{"name":tool_name,"arguments":input.to_string()}
                        })),
                        _ => return Err(AiMuxError::UnsupportedFunctionality("Unsupported content type in assistant message".into())),
                    }
                }
                let content = if has_reasoning {
                    json!(parts)
                } else {
                    json!(text)
                };
                let mut value = json!({"role":"assistant", "content":content});
                if index + 1 == prompt.len() {
                    value["prefix"] = json!(true);
                }
                if !calls.is_empty() {
                    value["tool_calls"] = json!(calls);
                }
                messages.push(value);
            }
            LanguageModelMessage::Tool { content, .. } => {
                for part in content {
                    let ToolPart::ToolResult(ToolResultPart {
                        tool_call_id,
                        tool_name,
                        output,
                        ..
                    }) = part
                    else {
                        continue;
                    };
                    messages.push(json!({"role":"tool", "name":tool_name, "tool_call_id":tool_call_id, "content":aimux_provider_utils::tool_result_to_content(output)}));
                }
            }
        }
    }
    Ok(messages)
}

fn convert_user_part(part: &UserPart) -> Result<Value, AiMuxError> {
    use base64::Engine;
    let file = match part {
        UserPart::Text(TextPart { text, .. }) => return Ok(json!({"type":"text", "text":text})),
        UserPart::File(file) => file,
    };
    let url = match &file.data {
        FileData::Reference { .. } => {
            return Err(AiMuxError::UnsupportedFunctionality(
                "file parts with provider references".into(),
            ));
        }
        FileData::Text { .. } => {
            return Err(AiMuxError::UnsupportedFunctionality(
                "text file parts".into(),
            ));
        }
        FileData::Data { data } => {
            let full = resolve_full_media_type(file)?;
            let data = match data {
                FileBytes::Binary(bytes) => base64::engine::general_purpose::STANDARD.encode(bytes),
                FileBytes::Base64(data) => data.clone(),
            };
            format!("data:{full};base64,{data}")
        }
        FileData::Url { url, .. } => url.clone(),
    };
    if get_top_level_media_type(&file.media_type) == "image" {
        Ok(json!({"type":"image_url", "image_url":url}))
    } else {
        let full = if matches!(file.data, FileData::Url { .. }) {
            file.media_type.clone()
        } else {
            resolve_full_media_type(file)?
        };
        if full != "application/pdf" {
            return Err(AiMuxError::UnsupportedFunctionality(
                "Only images and PDF file parts are supported".into(),
            ));
        }
        Ok(json!({"type":"document_url", "document_url":url}))
    }
}
