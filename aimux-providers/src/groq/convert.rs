//! Mirrors `convert-to-groq-chat-messages.ts`.

use base64::Engine;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultPart, UserPart,
};
use aimux_core::shared::{FileBytes, FileData};
use aimux_provider_utils::resolve_full_media_type;

fn is_image(media_type: &str) -> bool {
    media_type.split('/').next() == Some("image")
}

fn image_url_part(media_type: &str, base64: &str) -> Value {
    json!({
        "type": "image_url",
        "image_url": {
            "url": format!("data:{media_type};base64,{base64}"),
        },
    })
}

fn non_image() -> AiMuxError {
    AiMuxError::UnsupportedFunctionality("Non-image file content parts".to_string())
}

fn convert_user_part(part: &UserPart) -> Result<Value, AiMuxError> {
    Ok(match part {
        UserPart::Text(TextPart { text, .. }) => json!({ "type": "text", "text": text }),
        UserPart::File(file) => match &file.data {
            FileData::Reference { .. } => {
                return Err(AiMuxError::UnsupportedFunctionality(
                    "file parts with provider references".to_string(),
                ));
            }
            FileData::Text { .. } => {
                return Err(AiMuxError::UnsupportedFunctionality(
                    "text file parts".to_string(),
                ));
            }
            FileData::Url { url, .. } => {
                if !is_image(&file.media_type) {
                    return Err(non_image());
                }
                json!({ "type": "image_url", "image_url": { "url": url } })
            }
            FileData::Data { data } => {
                if !is_image(&file.media_type) {
                    return Err(non_image());
                }
                let data = match data {
                    FileBytes::Binary(bytes) => {
                        base64::engine::general_purpose::STANDARD.encode(bytes)
                    }
                    FileBytes::Base64(data) => data.clone(),
                };
                image_url_part(&resolve_full_media_type(file)?, &data)
            }
        },
    })
}

fn convert_assistant_message(content: &[AssistantPart]) -> Value {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    for part in content {
        match part {
            // Groq supports reasoning for tool-calls in multi-turn conversations.
            AssistantPart::Text(TextPart {
                text: part_text, ..
            }) => text.push_str(part_text),
            AssistantPart::Reasoning(ReasoningPart { text, .. }) => reasoning.push_str(text),
            AssistantPart::ToolCall(ToolCallPart {
                tool_call_id,
                tool_name,
                input,
                ..
            }) => tool_calls.push(json!({
                "id": tool_call_id,
                "type": "function",
                "function": {
                    "name": tool_name,
                    "arguments": input.to_string(),
                },
            })),
            _ => {}
        }
    }

    let mut message = Map::new();
    message.insert("role".into(), json!("assistant"));
    message.insert("content".into(), json!(text));
    if !reasoning.is_empty() {
        message.insert("reasoning".into(), json!(reasoning));
    }
    if !tool_calls.is_empty() {
        message.insert("tool_calls".into(), Value::Array(tool_calls));
    }
    Value::Object(message)
}

/// The `messages` of the request body.
///
/// # Errors
///
/// `UnsupportedFunctionality` for a non-image file or an undetectable image subtype.
pub(crate) fn convert_to_groq_chat_messages(
    prompt: &LanguageModelPrompt,
) -> Result<Vec<Value>, AiMuxError> {
    let mut messages = Vec::new();

    for message in prompt {
        match message {
            LanguageModelMessage::System { content, .. } => {
                messages.push(json!({ "role": "system", "content": content }));
            }
            LanguageModelMessage::User { content, .. } => {
                if let [UserPart::Text(TextPart { text, .. })] = content.as_slice() {
                    messages.push(json!({ "role": "user", "content": text }));
                    continue;
                }
                let parts = content
                    .iter()
                    .map(convert_user_part)
                    .collect::<Result<Vec<_>, _>>()?;
                messages.push(json!({ "role": "user", "content": parts }));
            }
            LanguageModelMessage::Assistant { content, .. } => {
                messages.push(convert_assistant_message(content))
            }
            LanguageModelMessage::Tool { content, .. } => {
                for part in content {
                    let ToolPart::ToolResult(ToolResultPart {
                        tool_call_id,
                        result,
                        ..
                    }) = part;
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": match result {
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        },
                    }));
                }
            }
        }
    }

    Ok(messages)
}
