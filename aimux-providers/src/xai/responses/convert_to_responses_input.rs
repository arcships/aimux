//! Prompt to the Responses `input` array (`convert-to-xai-responses-input.ts`),
//! with the image `detail` option of `xai-file-part-options.ts`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::types::Warning;
use aimux_provider_utils::{
    MediaTypeData, detect_media_type, get_top_level_media_type, is_full_media_type,
};
use serde_json::{Value, json};

use super::responses_language_model_options::validate_value;

/// Convert a [`LanguageModelPrompt`] into the xAI Responses `input` array,
/// returning warnings.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when a part cannot be converted.
pub fn convert_to_xai_responses_input(
    prompt: &LanguageModelPrompt,
) -> Result<(Vec<Value>, Vec<Warning>), AiMuxError> {
    let mut input = Vec::new();
    let mut warnings: Vec<Warning> = Vec::new();

    for msg in prompt {
        match msg {
            LanguageModelMessage::System { content, .. } => {
                input.push(json!({ "role": "system", "content": content }));
            }
            LanguageModelMessage::User { content, .. } => {
                let mut content_parts = Vec::new();
                for part in content {
                    match part {
                        UserPart::Text(TextPart { text, .. }) => {
                            content_parts.push(json!({ "type": "input_text", "text": text }));
                        }
                        UserPart::File(file) => {
                            if matches!(file.data, FileData::Data { .. })
                                && file.media_type.split('/').next() != Some("image")
                            {
                                return Err(AiMuxError::UnsupportedFunctionality(format!(
                                    "file part media type {} as inline data (xAI Responses requires a URL or a Files API reference for non-image files)",
                                    file.media_type
                                )));
                            }
                            content_parts.push(match &file.data {
                                FileData::Data {
                                    data: FileBytes::Binary(bytes),
                                } => {
                                    use base64::Engine;
                                    let b64 =
                                        base64::engine::general_purpose::STANDARD.encode(bytes);
                                    convert_image_part(file, Some(&b64), None)?
                                }
                                FileData::Data {
                                    data: FileBytes::Base64(data),
                                } => convert_image_part(file, Some(data), None)?,
                                FileData::Url { url, .. }
                                    if get_top_level_media_type(&file.media_type) == "image" =>
                                {
                                    convert_image_part(file, None, Some(url))?
                                }
                                FileData::Url { url, .. } => {
                                    json!({ "type": "input_file", "file_url": url })
                                }
                                FileData::Reference { reference } => {
                                    let file_id = aimux_provider_utils::resolve_provider_reference(
                                        reference,
                                        crate::xai::options::NAMESPACE,
                                    )?;
                                    json!({ "type": "input_file", "file_id": file_id })
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
                input.push(json!({ "role": "user", "content": content_parts }));
            }
            LanguageModelMessage::Assistant { content, .. } => {
                for part in content {
                    match part {
                        AssistantPart::Text(TextPart {
                            text,
                            provider_options,
                        }) => {
                            let id = provider_options
                                .as_ref()
                                .and_then(|po| crate::xai::options::xai_options(Some(po)))
                                .and_then(|x| x.get("itemId"))
                                .and_then(|v| v.as_str())
                                .map(std::string::ToString::to_string);
                            let mut msg_obj = json!({ "role": "assistant", "content": text });
                            if let Some(id) = id {
                                msg_obj["id"] = json!(id);
                            }
                            input.push(msg_obj);
                        }
                        AssistantPart::ToolCall(ToolCallPart {
                            tool_call_id,
                            tool_name,
                            input: tool_input,
                            provider_executed,
                            provider_options,
                            ..
                        }) => {
                            // Skip provider-executed tool calls.
                            let is_provider_executed = provider_executed.unwrap_or(false);
                            if is_provider_executed {
                                continue;
                            }
                            let id = provider_options
                                .as_ref()
                                .and_then(|po| crate::xai::options::xai_options(Some(po)))
                                .and_then(|x| x.get("itemId"))
                                .and_then(|v| v.as_str())
                                .map(std::string::ToString::to_string);
                            let item_id = id.unwrap_or_else(|| tool_call_id.clone());
                            let arguments = tool_input.to_string();
                            input.push(json!({
                                "type": "function_call",
                                "id": item_id,
                                "call_id": tool_call_id,
                                "name": tool_name,
                                "arguments": arguments,
                                "status": "completed"
                            }));
                        }
                        AssistantPart::ToolResult(_) => {}
                        AssistantPart::Reasoning(ReasoningPart {
                            text,
                            provider_options,
                            ..
                        }) => {
                            let item_id = provider_options
                                .as_ref()
                                .and_then(|po| crate::xai::options::xai_options(Some(po)))
                                .and_then(|x| x.get("itemId"))
                                .and_then(|v| v.as_str())
                                .map(std::string::ToString::to_string);
                            let encrypted_content = provider_options
                                .as_ref()
                                .and_then(|po| crate::xai::options::xai_options(Some(po)))
                                .and_then(|x| x.get("reasoningEncryptedContent"))
                                .and_then(|v| v.as_str())
                                .map(std::string::ToString::to_string);

                            if item_id.is_some() || encrypted_content.is_some() {
                                let mut summary: Vec<Value> = Vec::new();
                                if !text.is_empty() {
                                    summary.push(json!({ "type": "summary_text", "text": text }));
                                }
                                let mut reasoning_obj = json!({
                                    "type": "reasoning",
                                    "id": item_id.clone().unwrap_or_default(),
                                    "summary": summary,
                                    "status": "completed"
                                });
                                if let Some(ec) = encrypted_content {
                                    reasoning_obj["encrypted_content"] = json!(ec);
                                }
                                input.push(reasoning_obj);
                            } else {
                                warnings.push(Warning::Other {
                                    message: "Reasoning parts without itemId or encrypted content cannot be sent back to xAI. Skipping.".to_string(),
                                });
                            }
                        }
                        AssistantPart::File(_)
                        | AssistantPart::ReasoningFile(_)
                        | AssistantPart::Custom(_) => {
                            let kind = match part {
                                AssistantPart::File(_) => "file",
                                AssistantPart::ReasoningFile(_) => "reasoning-file",
                                _ => "custom",
                            };
                            warnings.push(Warning::Other {
                                message: format!("xAI Responses API does not support {kind} in assistant messages"),
                            });
                        }
                    }
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
                    let output_value = convert_tool_output(output)?;
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": tool_call_id,
                        "output": output_value
                    }));
                }
            }
        }
    }

    Ok((input, warnings))
}

fn convert_tool_output(output: &ToolResultOutput) -> Result<Value, AiMuxError> {
    match output {
        ToolResultOutput::ExecutionDenied { reason, .. } => {
            Ok(json!(reason.as_deref().unwrap_or("tool execution denied")))
        }
        ToolResultOutput::Content { value } => {
            let mut parts = Vec::new();
            for item in value {
                match item {
                    ToolResultContent::Text(part) => {
                        parts.push(json!({"type":"input_text", "text":part.text}))
                    }
                    ToolResultContent::File(part)
                        if get_top_level_media_type(&part.media_type) == "image" =>
                    {
                        let image_url = match &part.data {
                            FileData::Url { url, .. } => url.clone(),
                            FileData::Data { data } => {
                                use base64::Engine;
                                let encoded = match data {
                                    FileBytes::Base64(data) => data.clone(),
                                    FileBytes::Binary(bytes) => {
                                        base64::engine::general_purpose::STANDARD.encode(bytes)
                                    }
                                };
                                format!("data:{};base64,{encoded}", resolve_full_media_type(part)?)
                            }
                            _ => continue,
                        };
                        parts.push(json!({"type":"input_image", "image_url":image_url}));
                    }
                    _ => {}
                }
            }
            Ok(json!(parts))
        }
        _ => Ok(crate::openai::convert::tool_result_to_content(output)),
    }
}

fn convert_image_part(
    file_part: &FilePart,
    b64_data: Option<&str>,
    url: Option<&str>,
) -> Result<Value, AiMuxError> {
    let media_type = &file_part.media_type;
    if get_top_level_media_type(media_type) != "image" {
        return Err(AiMuxError::UnsupportedFunctionality(format!(
            "file part media type {media_type} as inline data (xAI Responses requires a URL or a Files API reference for non-image files)"
        )));
    }
    let image_url = if let Some(url_str) = url {
        url_str.to_string()
    } else if let Some(b64) = b64_data {
        let full_mt = resolve_full_media_type(file_part)?;
        format!("data:{full_mt};base64,{b64}")
    } else {
        String::new()
    };

    let mut part = json!({
        "type": "input_image",
        "image_url": image_url,
    });

    // Image detail provider option.
    if let Some(detail) = file_part
        .provider_options
        .as_ref()
        .and_then(|po| crate::xai::options::xai_options(Some(po)))
        .and_then(|x| x.get("imageDetail"))
    {
        validate_value("imageDetail", detail, |value| {
            value
                .as_str()
                .is_some_and(|value| ["auto", "low", "high"].contains(&value))
        })?;
        part["detail"] = detail.clone();
    }

    Ok(part)
}

fn resolve_full_media_type(part: &FilePart) -> Result<String, AiMuxError> {
    let media_type = &part.media_type;
    if is_full_media_type(media_type) {
        return Ok(media_type.clone());
    }
    let data = match &part.data {
        FileData::Data {
            data: FileBytes::Binary(bytes),
        } => Some(MediaTypeData::Bytes(bytes)),
        FileData::Data {
            data: FileBytes::Base64(data),
        } => Some(MediaTypeData::Base64(data)),
        _ => None,
    };
    let reason = if let Some(data) = data {
        if let Some(detected) = detect_media_type(data, Some(get_top_level_media_type(media_type)))?
        {
            return Ok(detected.into());
        }
        "it could not be auto-detected"
    } else {
        "it is not passed as inline bytes"
    };
    Err(AiMuxError::UnsupportedFunctionality(format!(
        "file of media type \"{media_type}\" must specify subtype since {reason}"
    )))
}
