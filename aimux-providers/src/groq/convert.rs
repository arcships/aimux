//! Mirrors `convert-to-groq-chat-messages.ts`.

use base64::Engine;
use serde_json::{Map, Value, json};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::message::Role;

fn text_of(content: &[ContentPart]) -> String {
    content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn is_image(media_type: &str) -> bool {
    media_type.split('/').next() == Some("image")
}

/// The media type of image data: a bare `image` or `image/*` is detected from
/// the first bytes of the base64 data (`resolveFullMediaType`).
fn resolve_full_media_type(media_type: &str, base64: &str) -> String {
    if media_type != "image" && !media_type.ends_with("/*") {
        return media_type.to_string();
    }
    if base64.starts_with("/9j/") {
        "image/jpeg"
    } else if base64.starts_with("R0lGOD") {
        "image/gif"
    } else if base64.starts_with("UklGR") {
        "image/webp"
    } else {
        "image/png"
    }
    .to_string()
}

fn image_url_part(media_type: &str, base64: &str) -> Value {
    json!({
        "type": "image_url",
        "image_url": {
            "url": format!("data:{};base64,{base64}", resolve_full_media_type(media_type, base64)),
        },
    })
}

fn non_image() -> AiMuxError {
    AiMuxError::UnsupportedFunctionality("Non-image file content parts".to_string())
}

fn convert_user_part(part: &ContentPart) -> Result<Option<Value>, AiMuxError> {
    let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(match part {
        ContentPart::Text { text, .. } => Some(json!({ "type": "text", "text": text })),
        ContentPart::FileReference { .. } => {
            return Err(AiMuxError::UnsupportedFunctionality(
                "file parts with provider references".to_string(),
            ));
        }
        ContentPart::Image {
            image, media_type, ..
        } => Some(image_url_part(media_type, &encode(image))),
        ContentPart::File {
            data, media_type, ..
        } if is_image(media_type) => Some(image_url_part(media_type, &encode(data))),
        ContentPart::FileBase64 {
            data, media_type, ..
        } if is_image(media_type) => Some(image_url_part(media_type, data)),
        ContentPart::FileUrl {
            url, media_type, ..
        } if is_image(media_type) => Some(json!({
            "type": "image_url",
            "image_url": { "url": url },
        })),
        ContentPart::File { .. } | ContentPart::FileBase64 { .. } | ContentPart::FileUrl { .. } => {
            return Err(non_image());
        }
        // Reasoning and tool traffic have no user-content form.
        ContentPart::Reasoning { .. }
        | ContentPart::ToolCall { .. }
        | ContentPart::ToolResult { .. } => None,
    })
}

fn convert_assistant_message(content: &[ContentPart]) -> Value {
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    for part in content {
        match part {
            // Groq supports reasoning for tool-calls in multi-turn conversations.
            ContentPart::Reasoning { text, .. } => reasoning.push_str(text),
            ContentPart::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => tool_calls.push(json!({
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
    message.insert("content".into(), json!(text_of(content)));
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
/// `UnsupportedFunctionality` for a file part that is not an image.
pub(crate) fn convert_to_groq_chat_messages(
    prompt: &LanguageModelPrompt,
) -> Result<Vec<Value>, AiMuxError> {
    let mut messages = Vec::new();

    for message in prompt {
        let content = &message.content;
        match message.role {
            Role::System => {
                messages.push(json!({ "role": "system", "content": text_of(content) }));
            }
            Role::User => {
                if let [ContentPart::Text { text, .. }] = content.as_slice() {
                    messages.push(json!({ "role": "user", "content": text }));
                    continue;
                }
                let parts = content
                    .iter()
                    .filter_map(|part| convert_user_part(part).transpose())
                    .collect::<Result<Vec<_>, _>>()?;
                messages.push(json!({ "role": "user", "content": parts }));
            }
            Role::Assistant => messages.push(convert_assistant_message(content)),
            Role::Tool => {
                for part in content {
                    if let ContentPart::ToolResult {
                        tool_call_id,
                        result,
                        ..
                    } = part
                    {
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
    }

    Ok(messages)
}
