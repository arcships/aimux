//! Prompt to Responses `input` (`convert-to-huggingface-responses-messages.ts`).

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart, UserPart,
};
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::types::Warning;

use aimux_provider_utils::{
    MediaTypeData, detect_media_type, get_top_level_media_type, is_full_media_type,
};

/// Convert a [`LanguageModelPrompt`] into the Hugging Face Responses `input`
/// array, returning the JSON value and any warnings.
///
/// Mirrors the TS `convertToHuggingFaceResponsesMessages`. Returns an error
/// for unsupported file part variants (provider references, non-image media
/// types) — matching the TS `UnsupportedFunctionalityError`.
///
/// # Errors
///
/// Returns `AiMuxError::UnsupportedFunctionality` for file parts with no
/// Responses representation (provider references, non-image media types).
pub fn convert_to_huggingface_responses_messages(
    prompt: &LanguageModelPrompt,
) -> Result<(Value, Vec<Warning>), AiMuxError> {
    let mut messages: Vec<Value> = Vec::new();
    let mut warnings: Vec<Warning> = Vec::new();

    for msg in prompt {
        match msg {
            LanguageModelMessage::System { content, .. } => {
                messages.push(json!({ "role": "system", "content": content }));
            }
            LanguageModelMessage::User { content, .. } => {
                let mut parts: Vec<Value> = Vec::new();
                for part in content {
                    match part {
                        UserPart::Text(TextPart { text, .. }) => {
                            parts.push(json!({ "type": "input_text", "text": text }));
                        }
                        UserPart::File(file) => {
                            parts.push(match &file.data {
                                FileData::Data {
                                    data: FileBytes::Binary(bytes),
                                } => {
                                    let encoded =
                                        aimux_provider_utils::convert_uint8_array_to_base64(bytes);
                                    convert_file_part_base64(&file.media_type, &encoded)?
                                }
                                FileData::Data {
                                    data: FileBytes::Base64(data),
                                } => convert_file_part_base64(&file.media_type, data)?,
                                FileData::Url { url, .. } => {
                                    convert_file_part_url(&file.media_type, url)?
                                }
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
                            });
                        }
                    }
                }
                messages.push(json!({ "role": "user", "content": parts }));
            }

            LanguageModelMessage::Assistant { content, .. } => {
                for part in content {
                    match part {
                        AssistantPart::Text(TextPart { text, .. }) => {
                            messages.push(json!({
                                "role": "assistant",
                                "content": [{ "type": "output_text", "text": text }]
                            }));
                        }
                        AssistantPart::Reasoning(ReasoningPart { text, .. }) => {
                            messages.push(json!({
                                "role": "assistant",
                                "content": [{ "type": "output_text", "text": text }]
                            }));
                        }
                        // Tool calls and tool results are handled by the
                        // Responses API — skip (no warning, matching TS).
                        AssistantPart::ToolCall(_)
                        | AssistantPart::ToolResult(_)
                        | AssistantPart::File(_)
                        | AssistantPart::ReasoningFile(_)
                        | AssistantPart::Custom(_) => {}
                    }
                }
            }

            LanguageModelMessage::Tool { .. } => {
                warnings.push(Warning::Unsupported {
                    feature: "tool messages".into(),
                    details: None,
                });
            }
        }
    }

    Ok((Value::Array(messages), warnings))
}

/// Convert a file part with inline base64 data to the HF `input_image` format.
///
/// Returns the JSON object `{ type: "input_image", image_url: "data:...;base64,..." }`
/// or an error if the media type is not an image.
fn convert_file_part_base64(media_type: &str, data: &str) -> Result<Value, AiMuxError> {
    let top_level = get_top_level_media_type(media_type);
    if top_level == "image" {
        let full_type = resolve_full_media_type_base64(media_type, data)?;
        Ok(json!({
            "type": "input_image",
            "image_url": format!("data:{};base64,{}", full_type, data)
        }))
    } else {
        Err(AiMuxError::UnsupportedFunctionality(format!(
            "file part media type {media_type}"
        )))
    }
}

/// Convert a file part with a URL source to the HF `input_image` format.
fn convert_file_part_url(media_type: &str, url: &str) -> Result<Value, AiMuxError> {
    let top_level = get_top_level_media_type(media_type);
    if top_level == "image" {
        Ok(json!({
            "type": "input_image",
            "image_url": url
        }))
    } else {
        Err(AiMuxError::UnsupportedFunctionality(format!(
            "file part media type {media_type}"
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Media type detection (top-level-only media type resolution)
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve a media type to its full `type/subtype` form.
///
/// - If already a full media type, return as-is.
/// - Otherwise, detect the subtype from the base64 data bytes.
fn resolve_full_media_type_base64(media_type: &str, data: &str) -> Result<String, AiMuxError> {
    if is_full_media_type(media_type) {
        return Ok(media_type.to_string());
    }

    let top_level = get_top_level_media_type(media_type);
    if let Some(detected) = detect_media_type(MediaTypeData::Base64(data), Some(top_level))? {
        return Ok(detected.to_string());
    }

    Err(AiMuxError::UnsupportedFunctionality(format!(
        "file of media type \"{media_type}\" must specify subtype since it could not be auto-detected"
    )))
}
