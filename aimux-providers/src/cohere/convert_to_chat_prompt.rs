//! `convert-to-cohere-chat-prompt.ts`: `LanguageModelPrompt` to the Cohere
//! `messages` and `documents`.

use aimux_core::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, LanguageModelMessage, LanguageModelPrompt, TextPart, ToolCallPart, ToolPart,
    ToolResultPart, UserPart,
};
use aimux_core::shared::{FileBytes, FileData, SharedProviderOptions};
use aimux_provider_utils::{get_top_level_media_type, resolve_full_media_type};
use serde_json::{Value, json};

// ── Prompt conversion ───────────────────────────────────────────────────────

/// The result of converting a prompt to Cohere format.
pub struct ConvertedPrompt {
    pub messages: Vec<Value>,
    pub documents: Vec<Value>,
}

/// Convert a `LanguageModelPrompt` to Cohere `messages` + `documents`.
///
/// Differences from OpenAI:
/// - System content is a plain string.
/// - User content (no images) is a plain string; with images, an array of parts.
/// - Non-image file parts are extracted as `documents` (RAG), not in the message.
/// - Assistant content is a plain string; with tool calls, `content` is omitted.
/// - Tool messages are flat `{role:"tool", content, tool_call_id}`.
///
/// # Errors
/// Rejects provider references, non-image file URLs and invalid image options.
pub fn convert_prompt_to_cohere(
    prompt: &LanguageModelPrompt,
) -> Result<ConvertedPrompt, AiMuxError> {
    let mut messages = Vec::new();
    let mut documents = Vec::new();

    for msg in prompt {
        match msg {
            LanguageModelMessage::System { content, .. } => {
                messages.push(json!({ "role": "system", "content": content }));
            }
            LanguageModelMessage::User { content, .. } => {
                let mut parts: Vec<Value> = Vec::new();
                let mut has_image = false;

                for part in content {
                    match part {
                        UserPart::Text(TextPart { text, .. }) => {
                            if !text.is_empty() {
                                parts.push(json!({ "type": "text", "text": text }));
                            }
                        }
                        UserPart::File(file) => {
                            if get_top_level_media_type(&file.media_type) == "image" {
                                let url = match &file.data {
                                    FileData::Data { data } => {
                                        use base64::Engine;
                                        let b64 = match data {
                                            FileBytes::Binary(bytes) => {
                                                base64::engine::general_purpose::STANDARD
                                                    .encode(bytes)
                                            }
                                            FileBytes::Base64(data) => data.clone(),
                                        };
                                        format!(
                                            "data:{};base64,{}",
                                            resolve_full_media_type(file)?,
                                            b64
                                        )
                                    }
                                    FileData::Url { url, .. } => url.clone(),
                                    FileData::Reference { .. } => {
                                        return Err(AiMuxError::UnsupportedFunctionality(
                                            "image file parts with provider references".into(),
                                        ));
                                    }
                                    FileData::Text { .. } => {
                                        return Err(AiMuxError::UnsupportedFunctionality(
                                            "image file parts with text data".into(),
                                        ));
                                    }
                                };
                                has_image = true;
                                parts.push(image_part(url, &file.provider_options)?);
                            } else {
                                let text = match &file.data {
                                    FileData::Data {
                                        data: FileBytes::Binary(bytes),
                                    } => String::from_utf8_lossy(bytes).into_owned(),
                                    FileData::Data {
                                        data: FileBytes::Base64(data),
                                    } => data.clone(),
                                    FileData::Text { text } => text.clone(),
                                    FileData::Url { .. } => {
                                        return Err(AiMuxError::UnsupportedFunctionality(
                                            "File URL data: URLs should be downloaded by the AI SDK and not reach this point. This indicates a configuration issue.".into(),
                                        ));
                                    }
                                    FileData::Reference { .. } => {
                                        return Err(AiMuxError::UnsupportedFunctionality(
                                            "file parts with provider references".into(),
                                        ));
                                    }
                                };
                                let mut doc_data = json!({ "text": text });
                                if let Some(filename) = &file.filename {
                                    doc_data["title"] = json!(filename);
                                }
                                documents.push(json!({ "data": doc_data }));
                            }
                        }
                    }
                }

                if has_image {
                    messages.push(json!({ "role": "user", "content": parts }));
                } else {
                    let text: String = parts
                        .iter()
                        .filter_map(|p| {
                            if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                                p.get("text")
                                    .and_then(|t| t.as_str())
                                    .map(std::string::ToString::to_string)
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    messages.push(json!({ "role": "user", "content": text }));
                }
            }
            LanguageModelMessage::Assistant { content, .. } => {
                let text = join_text_parts(content);
                let has_tool_calls = content
                    .iter()
                    .any(|p| matches!(p, AssistantPart::ToolCall(_)));

                if has_tool_calls {
                    let tool_calls: Vec<Value> = content
                        .iter()
                        .filter_map(|p| match p {
                            AssistantPart::ToolCall(ToolCallPart {
                                tool_call_id,
                                tool_name,
                                input,
                                ..
                            }) => {
                                let arguments = input.to_string();
                                Some(json!({
                                    "id": tool_call_id,
                                    "type": "function",
                                    "function": {
                                        "name": tool_name,
                                        "arguments": arguments,
                                    }
                                }))
                            }
                            _ => None,
                        })
                        .collect();
                    messages.push(json!({
                        "role": "assistant",
                        "tool_calls": tool_calls,
                    }));
                } else {
                    messages.push(json!({ "role": "assistant", "content": text }));
                }
            }
            LanguageModelMessage::Tool { content, .. } => {
                for part in content {
                    let ToolPart::ToolResult(ToolResultPart {
                        tool_call_id,
                        output,
                        ..
                    }) = part
                    else {
                        continue;
                    };
                    let content = crate::openai::convert::tool_result_to_content(output);
                    messages.push(json!({
                        "role": "tool",
                        "content": content,
                        "tool_call_id": tool_call_id,
                    }));
                }
            }
        }
    }

    Ok(ConvertedPrompt {
        messages,
        documents,
    })
}

fn image_part(
    url: String,
    provider_options: &Option<SharedProviderOptions>,
) -> Result<Value, AiMuxError> {
    let mut image = json!({ "url": url });
    if let Some(detail) =
        super::chat_language_model_options::cohere_options(provider_options.as_ref())
            .and_then(|options| options.get("detail"))
    {
        if !matches!(detail.as_str(), Some("auto" | "low" | "high")) {
            return Err(AiMuxError::InvalidArgument(
                "cohere.detail must be auto, low or high".into(),
            ));
        }
        image["detail"] = detail.clone();
    }
    Ok(json!({ "type": "image_url", "image_url": image }))
}

fn join_text_parts(content: &[AssistantPart]) -> String {
    content
        .iter()
        .filter_map(|p| match p {
            AssistantPart::Text(TextPart { text, .. }) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests;
