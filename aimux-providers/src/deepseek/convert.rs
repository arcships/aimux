//! Prompt to DeepSeek chat messages (`convert-to-deepseek-chat-messages.ts`).

use base64::Engine;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::options::ResponseFormat;
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::types::Warning;
use aimux_provider_utils::resolve_full_media_type;

use super::is_v4_model::is_deepseek_v4_model;
use super::options::{parse_file_part_options, parse_message_options};

const SUPPORTED_IMAGE_MEDIA_TYPES: [&str; 5] = [
    "image/gif",
    "image/jpeg",
    "image/jpg",
    "image/png",
    "image/webp",
];

fn resolve_deepseek_image_media_type(part: &FilePart) -> Result<String, AiMuxError> {
    let resolved = resolve_full_media_type(part)?;
    if !SUPPORTED_IMAGE_MEDIA_TYPES.contains(&resolved.as_str()) {
        return Err(AiMuxError::UnsupportedFunctionality(format!(
            "DeepSeek image media type {resolved}: DeepSeek supports JPEG, PNG, GIF, and WebP image inputs."
        )));
    }
    Ok(resolved)
}

/// The content part of an image file part.
fn convert_image_part(
    part: &FilePart,
    provider_options_name: &str,
    tool_result: bool,
) -> Result<Value, AiMuxError> {
    if tool_result && let FileData::Reference { reference } = &part.data {
        let file_id = reference.get("deepseek").ok_or_else(|| {
            AiMuxError::InvalidArgument(
                "No provider reference found for provider 'deepseek'.".into(),
            )
        })?;
        return Ok(json!({ "type": "file", "file_id": file_id }));
    }
    let mut options =
        parse_file_part_options(part.provider_options.as_ref(), provider_options_name)?;
    if tool_result {
        options.file_data = None;
    }
    let image_url = |url: &str| {
        let mut image_url = json!({ "url": url });
        if let Some(detail) = &options.image_detail {
            image_url["detail"] = json!(detail);
        }
        json!({ "type": "image_url", "image_url": image_url })
    };
    match &part.data {
        FileData::Reference { reference } => {
            let file_id = reference.get("deepseek").ok_or_else(|| {
                AiMuxError::InvalidArgument(
                    "No provider reference found for provider 'deepseek'.".to_string(),
                )
            })?;
            Ok(json!({ "type": "file", "file_id": file_id }))
        }
        FileData::Url { url, .. } => {
            resolve_deepseek_image_media_type(part)?;
            if url.len() > 8192 {
                return Err(AiMuxError::InvalidPrompt(
                    "DeepSeek image URLs must not exceed 8192 characters.".to_string(),
                ));
            }
            if options.file_data == Some(true) {
                return Err(AiMuxError::InvalidPrompt(
                    "DeepSeek `fileData` image parts require inline data, not a URL.".to_string(),
                ));
            }
            Ok(image_url(url))
        }
        FileData::Data { data } => {
            let media_type = resolve_deepseek_image_media_type(part)?;
            let data = match data {
                FileBytes::Binary(bytes) => base64::engine::general_purpose::STANDARD.encode(bytes),
                FileBytes::Base64(data) => data.clone(),
            };
            let media_type = if media_type == "image/jpg" {
                "image/jpeg"
            } else {
                &media_type
            };
            let data_url = format!("data:{media_type};base64,{data}");
            if options.file_data != Some(true) {
                return Ok(image_url(&data_url));
            }
            if options.image_detail.is_some() {
                return Err(AiMuxError::InvalidPrompt(
                    "DeepSeek `imageDetail` cannot be combined with `fileData`.".to_string(),
                ));
            }
            let mut file = json!({ "type": "file", "file_data": data_url });
            if let Some(filename) = &part.filename {
                file["filename"] = json!(filename);
            }
            Ok(file)
        }
        FileData::Text { .. } => unreachable!("inline text is not an image input"),
    }
}

/// The messages of a request and the warnings raised while converting.
pub(crate) struct ConvertedMessages {
    pub messages: Vec<Value>,
    pub warnings: Vec<Warning>,
}

/// The `messages` of the request body.
///
/// # Errors
///
/// `InvalidPrompt` / `UnsupportedFunctionality` / `InvalidArgument` for a
/// prompt DeepSeek cannot take (an assistant prefix that is not the last
/// message or needs a beta base URL, an unsupported image).
pub(crate) fn convert_to_deepseek_chat_messages(
    prompt: &LanguageModelPrompt,
    response_format: Option<&ResponseFormat>,
    model_id: &str,
    provider_options_name: &str,
    supports_assistant_prefix_completion: bool,
) -> Result<ConvertedMessages, AiMuxError> {
    let is_deepseek_v4 = is_deepseek_v4_model(model_id);
    let mut messages = Vec::new();
    let mut warnings = Vec::new();

    // Inject a system message if the response format is JSON. The DeepSeek
    // package never supports structured outputs, so a schema is injected too.
    if let Some(ResponseFormat::Json { schema, .. }) = response_format {
        match schema {
            None => messages.push(json!({ "role": "system", "content": "Return JSON." })),
            Some(schema) => {
                messages.push(json!({
                    "role": "system",
                    "content": format!(
                        "Return JSON that conforms to the following schema: {schema}"
                    ),
                }));
                warnings.push(Warning::Compatibility {
                    feature: "responseFormat JSON schema".to_string(),
                    details: Some(
                        "JSON response schema is injected into the system message.".to_string(),
                    ),
                });
            }
        }
    }
    let last_user_message_index = prompt
        .iter()
        .rposition(|message| matches!(message, LanguageModelMessage::User { .. }));

    for (index, message) in prompt.iter().enumerate() {
        let provider_options = match message {
            LanguageModelMessage::System {
                provider_options, ..
            }
            | LanguageModelMessage::User {
                provider_options, ..
            }
            | LanguageModelMessage::Assistant {
                provider_options, ..
            }
            | LanguageModelMessage::Tool {
                provider_options, ..
            } => provider_options,
        };
        let options = parse_message_options(provider_options.as_ref(), provider_options_name)?;
        if options.prefix == Some(true)
            && !matches!(message, LanguageModelMessage::Assistant { .. })
        {
            return Err(AiMuxError::InvalidPrompt(
                "DeepSeek assistant prefix completion requires `prefix: true` on an assistant message."
                    .to_string(),
            ));
        }
        let name = |wire: &mut Map<String, Value>| {
            if let Some(name) = &options.name {
                wire.insert("name".into(), json!(name));
            }
        };

        match message {
            LanguageModelMessage::System { content, .. } => {
                let mut wire = Map::new();
                wire.insert("role".into(), json!("system"));
                wire.insert("content".into(), json!(content));
                name(&mut wire);
                messages.push(Value::Object(wire));
            }

            LanguageModelMessage::User { content: parts, .. } => {
                let is_image = |file: &FilePart| {
                    file.media_type.split('/').next() == Some("image")
                        && !matches!(file.data, FileData::Text { .. })
                };
                let has_image_part = parts
                    .iter()
                    .any(|part| matches!(part, UserPart::File(file) if is_image(file)));
                let mut content = Vec::new();
                let mut text = String::new();
                for part in parts {
                    match part {
                        UserPart::Text(TextPart {
                            text: part_text, ..
                        }) => {
                            text.push_str(part_text);
                            content.push(json!({ "type": "text", "text": part_text }));
                        }
                        UserPart::File(file) if is_image(file) => {
                            content.push(convert_image_part(file, provider_options_name, false)?);
                        }
                        UserPart::File(_) => warnings.push(Warning::Unsupported {
                            feature: "user message part type: file".to_string(),
                            details: None,
                        }),
                    }
                }
                let mut wire = Map::new();
                wire.insert("role".into(), json!("user"));
                wire.insert(
                    "content".into(),
                    if has_image_part {
                        Value::Array(content)
                    } else {
                        json!(text)
                    },
                );
                name(&mut wire);
                messages.push(Value::Object(wire));
            }

            LanguageModelMessage::Assistant { content, .. } => {
                if options.prefix == Some(true) {
                    if index != prompt.len() - 1 {
                        return Err(AiMuxError::InvalidPrompt(
                            "DeepSeek assistant prefix completion requires the prefixed assistant message to be the final message."
                                .to_string(),
                        ));
                    }
                    if !supports_assistant_prefix_completion {
                        return Err(AiMuxError::UnsupportedFunctionality(
                            "DeepSeek assistant prefix completion requires a beta base URL ending in `/beta`."
                                .to_string(),
                        ));
                    }
                }

                let mut text = String::new();
                let mut reasoning: Option<String> = None;
                let mut tool_calls = Vec::new();
                for part in content {
                    match part {
                        AssistantPart::Text(TextPart { text: t, .. }) => text.push_str(t),
                        AssistantPart::Reasoning(ReasoningPart { text: t, .. }) => {
                            // R1 must not receive prior reasoning; V4 requires it.
                            if last_user_message_index.is_some_and(|last| index <= last)
                                && !is_deepseek_v4
                            {
                                continue;
                            }
                            reasoning.get_or_insert_with(String::new).push_str(t);
                        }
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

                let mut wire = Map::new();
                wire.insert("role".into(), json!("assistant"));
                wire.insert("content".into(), json!(text));
                name(&mut wire);
                if options.prefix == Some(true) {
                    wire.insert("prefix".into(), json!(true));
                }
                // V4 demands the field on every assistant turn: back-fill an
                // empty string when the message had no reasoning part at all.
                if let Some(reasoning) = reasoning.or_else(|| is_deepseek_v4.then(String::new)) {
                    wire.insert("reasoning_content".into(), json!(reasoning));
                }
                if !tool_calls.is_empty() {
                    wire.insert("tool_calls".into(), Value::Array(tool_calls));
                }
                messages.push(Value::Object(wire));
            }

            LanguageModelMessage::Tool { content, .. } => {
                if options.name.is_some() {
                    warnings.push(Warning::Unsupported {
                        feature: "message name on tool messages".to_string(),
                        details: None,
                    });
                }
                for part in content {
                    let ToolPart::ToolResult(ToolResultPart {
                        tool_call_id,
                        output,
                        ..
                    }) = part
                    else {
                        continue;
                    };
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": convert_tool_result_content(output, provider_options_name, &mut warnings)?,
                    }));
                }
            }
        }
    }

    Ok(ConvertedMessages { messages, warnings })
}

fn convert_tool_result_content(
    output: &ToolResultOutput,
    provider_options_name: &str,
    warnings: &mut Vec<Warning>,
) -> Result<Value, AiMuxError> {
    let ToolResultOutput::Content { value } = output else {
        return Ok(crate::openai::convert::tool_result_to_content(output));
    };
    let is_image = |part: &ToolResultContent| {
        matches!(part, ToolResultContent::File(file)
            if file.media_type.split('/').next() == Some("image")
                && !matches!(file.data, FileData::Text { .. }))
    };
    if !value.iter().any(is_image) {
        return Ok(crate::openai::convert::tool_result_to_content(output));
    }
    let mut content = Vec::new();
    for part in value {
        match part {
            ToolResultContent::Text(text) => {
                content.push(json!({ "type": "text", "text": text.text }));
            }
            ToolResultContent::File(file) if is_image(part) => {
                content.push(convert_image_part(file, provider_options_name, true)?);
            }
            other => {
                let kind = match other {
                    ToolResultContent::File(_) => "file",
                    ToolResultContent::Custom { .. } => "custom",
                    ToolResultContent::Text(_) => unreachable!(),
                };
                warnings.push(Warning::Unsupported {
                    feature: format!("tool result content part type: {kind}"),
                    details: None,
                });
            }
        }
    }
    Ok(Value::Array(content))
}
