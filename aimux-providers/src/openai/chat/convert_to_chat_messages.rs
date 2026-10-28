//! Conversion of a `LanguageModelPrompt` to Chat Completions messages
//! (convert-to-openai-chat-messages.ts).

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, TextPart, ToolPart,
    ToolResultOutput, UserPart,
};
use aimux_core::shared::{FileData, SharedProviderOptions};
use aimux_provider_utils::{
    get_top_level_media_type, resolve_full_media_type, tool_result_to_content,
};
use serde_json::{Value, json};

use crate::openai::language_model_capabilities::SystemMessageMode;

/// Convert a `LanguageModelPrompt` to OpenAI `messages` array with a system
/// message mode.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when a message part cannot be
/// converted (e.g. an unsupported file part shape or a missing provider
/// reference).
pub fn convert_prompt_to_openai_messages_with_mode_fallible(
    prompt: &LanguageModelPrompt,
    system_message_mode: SystemMessageMode,
) -> Result<Vec<Value>, AiMuxError> {
    let mut result = Vec::new();
    for msg in prompt {
        result.extend(convert_message_to_openai(msg, system_message_mode)?);
    }
    Ok(result)
}

/// Get the prompt cache breakpoint from provider options.
fn get_prompt_cache_breakpoint(provider_options: &Option<SharedProviderOptions>) -> Option<Value> {
    provider_options
        .as_ref()
        .and_then(|po| po.get("openai"))
        .and_then(|o| o.get("promptCacheBreakpoint"))
        .cloned()
}

/// Get imageDetail from provider options.
fn get_image_detail(provider_options: &Option<SharedProviderOptions>) -> Option<Value> {
    provider_options
        .as_ref()
        .and_then(|po| po.get("openai"))
        .and_then(|o| o.get("imageDetail"))
        .cloned()
}

/// Convert a file part to the OpenAI format, handling images, audio, and PDF.
fn convert_file_part_to_openai(file: &FilePart, part_index: usize) -> Result<Value, AiMuxError> {
    let FilePart {
        data,
        media_type,
        filename,
        provider_options,
    } = file;
    let prompt_cache_breakpoint = get_prompt_cache_breakpoint(provider_options);
    let (data_b64, url) = match data {
        FileData::Reference { reference } => {
            let file_id = aimux_provider_utils::resolve_provider_reference(reference, "openai")?;
            let mut part = json!({ "type": "file", "file": { "file_id": file_id } });
            if let Some(bpt) = prompt_cache_breakpoint {
                part["prompt_cache_breakpoint"] = bpt;
            }
            return Ok(part);
        }
        FileData::Text { .. } => {
            return Err(AiMuxError::UnsupportedFunctionality(
                "text file parts".into(),
            ));
        }
        FileData::Url { url, .. } => (None, Some(url.as_str())),
        FileData::Data { data } => (
            Some(aimux_provider_utils::convert_to_base64(data).into_owned()),
            None,
        ),
    };
    let data_b64 = data_b64.as_deref();
    let filename = filename.as_deref();

    let top_level = get_top_level_media_type(media_type);

    // Image
    if top_level == "image" {
        let image_url = if let Some(url_str) = url {
            json!({ "url": url_str })
        } else if let Some(b64) = data_b64 {
            let full_mt = resolve_full_media_type(file)?;
            json!({ "url": format!("data:{};base64,{}", full_mt, b64) })
        } else {
            return Err(AiMuxError::invalid_argument(
                "image part has no data or url",
            ));
        };

        let mut image_url_obj = image_url;
        if let Some(detail) = get_image_detail(provider_options) {
            image_url_obj["detail"] = detail;
        }

        let mut part = json!({
            "type": "image_url",
            "image_url": image_url_obj,
        });
        if let Some(bpt) = prompt_cache_breakpoint {
            part["prompt_cache_breakpoint"] = bpt;
        }
        return Ok(part);
    }

    // Audio
    if top_level == "audio" {
        if url.is_some() {
            return Err(AiMuxError::UnsupportedFunctionality(
                "audio file parts with URLs".into(),
            ));
        }
        let b64 = data_b64.ok_or_else(|| AiMuxError::invalid_argument("audio part has no data"))?;
        let full_mt = resolve_full_media_type(file)?;
        let format = match full_mt.as_str() {
            "audio/wav" => "wav",
            "audio/mp3" | "audio/mpeg" => "mp3",
            _ => {
                return Err(AiMuxError::UnsupportedFunctionality(format!(
                    "audio content parts with media type {full_mt}"
                )));
            }
        };
        let mut part = json!({
            "type": "input_audio",
            "input_audio": { "data": b64, "format": format }
        });
        if let Some(bpt) = prompt_cache_breakpoint {
            part["prompt_cache_breakpoint"] = bpt;
        }
        return Ok(part);
    }

    // PDF / application
    let full_mt = resolve_full_media_type(file)?;

    if full_mt != "application/pdf" {
        return Err(AiMuxError::UnsupportedFunctionality(format!(
            "file part media type {full_mt}"
        )));
    }

    if url.is_some() {
        return Err(AiMuxError::UnsupportedFunctionality(
            "PDF file parts with URLs".into(),
        ));
    }

    let b64 = data_b64.ok_or_else(|| AiMuxError::invalid_argument("PDF part has no data"))?;
    let fname = filename
        .map(std::string::ToString::to_string)
        .unwrap_or_else(|| format!("part-{part_index}.pdf"));
    let mut part = json!({
        "type": "file",
        "file": {
            "filename": fname,
            "file_data": format!("data:application/pdf;base64,{}", b64),
        }
    });
    if let Some(bpt) = prompt_cache_breakpoint {
        part["prompt_cache_breakpoint"] = bpt;
    }
    Ok(part)
}

/// Convert a single provider-facing message into one or more OpenAI messages.
fn convert_message_to_openai(
    msg: &LanguageModelMessage,
    system_message_mode: SystemMessageMode,
) -> Result<Vec<Value>, AiMuxError> {
    let message = match msg {
        LanguageModelMessage::System {
            content,
            provider_options,
        } => {
            let role = match system_message_mode {
                SystemMessageMode::Remove => return Ok(vec![]),
                SystemMessageMode::Developer => "developer",
                SystemMessageMode::System => "system",
            };
            let content = match get_prompt_cache_breakpoint(provider_options) {
                None => json!(content),
                Some(bpt) => json!([{
                    "type": "text", "text": content, "prompt_cache_breakpoint": bpt,
                }]),
            };
            json!({ "role": role, "content": content })
        }
        LanguageModelMessage::Tool { content, .. } => {
            return Ok(content
                .iter()
                .filter_map(|part| {
                    let ToolPart::ToolResult(result) = part else { return None; };
                    let mut content = tool_result_to_content(&result.output);
                    let breakpoint = tool_result_cache_breakpoint(&result.output).or_else(|| get_prompt_cache_breakpoint(&result.provider_options));
                    if let Some(breakpoint) = breakpoint {
                        content = json!([{ "type": "text", "text": content, "prompt_cache_breakpoint": breakpoint }]);
                    }
                    Some(json!({ "role": "tool", "content": content, "tool_call_id": result.tool_call_id }))
                })
                .collect());
        }
        LanguageModelMessage::User { content, .. } => {
            let all_plain_text = content.len() == 1
                && content.iter().all(|part| {
                    matches!(part, UserPart::Text(text)
                    if get_prompt_cache_breakpoint(&text.provider_options).is_none())
                });
            let content = if all_plain_text {
                json!(
                    content
                        .iter()
                        .filter_map(|part| match part {
                            UserPart::Text(text) => Some(text.text.as_str()),
                            UserPart::File(_) => None,
                        })
                        .collect::<String>()
                )
            } else {
                json!(
                    content
                        .iter()
                        .enumerate()
                        .map(|(index, part)| match part {
                            UserPart::Text(text) => Ok(convert_text_part_to_openai(text)),
                            UserPart::File(file) => convert_file_part_to_openai(file, index),
                        })
                        .collect::<Result<Vec<_>, _>>()?
                )
            };
            json!({ "role": "user", "content": content })
        }
        LanguageModelMessage::Assistant { content, .. } => {
            let mut text = String::new();
            let mut text_parts = Vec::new();
            let mut has_cache_breakpoint = false;
            let mut tool_calls = Vec::new();
            for part in content {
                match part {
                    AssistantPart::Text(part) => {
                        text.push_str(&part.text);
                        has_cache_breakpoint |=
                            get_prompt_cache_breakpoint(&part.provider_options).is_some();
                        text_parts.push(convert_text_part_to_openai(part));
                    }
                    AssistantPart::ToolCall(part) => {
                        // Upstream `serializeToolCallArguments` (@ai-sdk/openai
                        // 4.0.49+) replays any non-object input as `{}` on
                        // purpose; OpenAI function arguments are always objects.
                        let arguments = if !part.input.is_object() {
                            "{}".to_string()
                        } else {
                            part.input.to_string()
                        };
                        tool_calls.push(json!({
                            "type": "function", "id": part.tool_call_id,
                            "function": { "name": part.tool_name, "arguments": arguments },
                        }));
                    }
                    _ => {}
                }
            }
            let content = if has_cache_breakpoint {
                json!(text_parts)
            } else if !tool_calls.is_empty() && text.is_empty() {
                Value::Null
            } else {
                json!(text)
            };
            let mut message = json!({ "role": "assistant", "content": content });
            if !tool_calls.is_empty() {
                message["tool_calls"] = json!(tool_calls);
            }
            message
        }
    };
    Ok(vec![message])
}

pub(crate) fn tool_result_cache_breakpoint(output: &ToolResultOutput) -> Option<Value> {
    match output {
        ToolResultOutput::Text {
            provider_options, ..
        }
        | ToolResultOutput::Json {
            provider_options, ..
        }
        | ToolResultOutput::ErrorText {
            provider_options, ..
        }
        | ToolResultOutput::ErrorJson {
            provider_options, ..
        }
        | ToolResultOutput::ExecutionDenied {
            provider_options, ..
        } => get_prompt_cache_breakpoint(provider_options),
        ToolResultOutput::Content { value } => value.iter().find_map(|part| match part {
            aimux_core::language_model_message::ToolResultContent::Text(part) => {
                get_prompt_cache_breakpoint(&part.provider_options)
            }
            aimux_core::language_model_message::ToolResultContent::File(part) => {
                get_prompt_cache_breakpoint(&part.provider_options)
            }
            aimux_core::language_model_message::ToolResultContent::Custom { provider_options } => {
                get_prompt_cache_breakpoint(provider_options)
            }
        }),
    }
}

fn convert_text_part_to_openai(part: &TextPart) -> Value {
    let mut value = json!({ "type": "text", "text": part.text });
    if let Some(bpt) = get_prompt_cache_breakpoint(&part.provider_options) {
        value["prompt_cache_breakpoint"] = bpt;
    }
    value
}
