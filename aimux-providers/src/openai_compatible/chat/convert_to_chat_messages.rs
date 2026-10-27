//! The prompt to the chat-completions `messages`
//! (`convert-to-openai-compatible-chat-messages.ts`).

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::shared::{FileBytes, FileData, SharedProviderOptions};
use aimux_provider_utils::{get_top_level_media_type, resolve_full_media_type};
use base64::Engine;
use serde_json::{Map, Value, json};

pub(crate) struct MessageSpec<'a> {
    pub metadata_key: &'a str,
    /// Send a `content` tool output as content parts, not a JSON string.
    pub supports_multi_part_tool_content: bool,
}

/// The generic `openaiCompatible` metadata of a message or part
/// (`getOpenAIMetadata`).
fn wire_metadata(provider_options: Option<&SharedProviderOptions>) -> Map<String, Value> {
    provider_options
        .and_then(|options| options.get("openaiCompatible"))
        .cloned()
        .unwrap_or_default()
}

fn with_metadata(mut object: Value, metadata: Map<String, Value>) -> Value {
    if let Some(map) = object.as_object_mut() {
        map.extend(metadata);
    }
    object
}

/// `messages` for the body.
///
/// # Errors
///
/// `InvalidArgument` when a part has no wire form (an unsupported media type,
/// an audio or PDF part given by URL).
pub(crate) fn convert_messages(
    prompt: &LanguageModelPrompt,
    spec: &MessageSpec<'_>,
) -> Result<Vec<Value>, AiMuxError> {
    let mut out = Vec::new();
    for message in prompt {
        out.extend(convert_message(message, spec)?);
    }
    Ok(out)
}

fn convert_message(
    message: &LanguageModelMessage,
    spec: &MessageSpec<'_>,
) -> Result<Vec<Value>, AiMuxError> {
    match message {
        LanguageModelMessage::System {
            content,
            provider_options,
        } => Ok(vec![with_metadata(
            json!({ "role": "system", "content": content }),
            wire_metadata(provider_options.as_ref()),
        )]),
        LanguageModelMessage::User {
            content,
            provider_options,
        } => Ok(vec![convert_user_message(
            content,
            wire_metadata(provider_options.as_ref()),
        )?]),
        LanguageModelMessage::Assistant {
            content,
            provider_options,
        } => Ok(vec![convert_assistant_message(
            content,
            spec,
            wire_metadata(provider_options.as_ref()),
        )]),
        LanguageModelMessage::Tool { content, .. } => content
            .iter()
            .filter_map(|part| match part {
                ToolPart::ToolResult(ToolResultPart {
                    tool_call_id,
                    output,
                    provider_options,
                    ..
                }) => Some((tool_call_id, output, provider_options)),
                _ => None,
            })
            .map(|(tool_call_id, output, provider_options)| {
                // convert-to-openai-compatible-chat-messages.ts:284-321: a
                // `content` output is an array of content parts only when
                // the provider supports multi-part tool content.
                let content = match output {
                    ToolResultOutput::Content { value }
                        if spec.supports_multi_part_tool_content =>
                    {
                        Value::Array(
                            value
                                .iter()
                                .map(convert_tool_content_part)
                                .collect::<Result<_, _>>()?,
                        )
                    }
                    output => aimux_provider_utils::tool_result_to_content(output),
                };
                Ok(with_metadata(
                    json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": content,
                    }),
                    wire_metadata(provider_options.as_ref()),
                ))
            })
            .collect(),
    }
}

/// One part of a multi-part tool result (`convertToolContentPart`,
/// convert-to-openai-compatible-chat-messages.ts:162-176): a text or file part
/// is converted like a user content part; a custom part has no wire form.
fn convert_tool_content_part(part: &ToolResultContent) -> Result<Value, AiMuxError> {
    match part {
        ToolResultContent::Text(text) => Ok(convert_text_part(text)),
        ToolResultContent::File(file) => convert_file_part(file),
        ToolResultContent::Custom { .. } => Err(AiMuxError::UnsupportedFunctionality(
            "custom tool content parts".to_string(),
        )),
    }
}

fn convert_user_message(
    content: &[UserPart],
    message_metadata: Map<String, Value>,
) -> Result<Value, AiMuxError> {
    if let [
        UserPart::Text(TextPart {
            text,
            provider_options,
        }),
    ] = content
    {
        let metadata = wire_metadata(provider_options.as_ref());
        return Ok(with_metadata(
            json!({ "role": "user", "content": text }),
            metadata,
        ));
    }
    let parts = content
        .iter()
        .map(convert_user_part)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(with_metadata(
        json!({ "role": "user", "content": parts }),
        message_metadata,
    ))
}

fn convert_user_part(part: &UserPart) -> Result<Value, AiMuxError> {
    match part {
        UserPart::Text(text) => Ok(convert_text_part(text)),
        UserPart::File(file) => convert_file_part(file),
    }
}

/// A text content part (`convertToOpenAICompatibleContentPart`).
fn convert_text_part(part: &TextPart) -> Value {
    with_metadata(
        json!({ "type": "text", "text": part.text }),
        wire_metadata(part.provider_options.as_ref()),
    )
}

/// A file content part (`convertToOpenAICompatibleContentPart`).
fn convert_file_part(file: &FilePart) -> Result<Value, AiMuxError> {
    let wire = match &file.data {
        FileData::Data {
            data: FileBytes::Binary(data),
        } => file_part(
            file,
            FileSource::Base64(&base64::engine::general_purpose::STANDARD.encode(data)),
        )?,
        FileData::Data {
            data: FileBytes::Base64(data),
        } => file_part(file, FileSource::Base64(data))?,
        FileData::Url { url, .. } => file_part(file, FileSource::Url(url))?,
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
    };
    Ok(with_metadata(
        wire,
        wire_metadata(file.provider_options.as_ref()),
    ))
}

enum FileSource<'a> {
    Base64(&'a str),
    Url(&'a str),
}

fn file_part(part: &FilePart, source: FileSource<'_>) -> Result<Value, AiMuxError> {
    let media_type = part.media_type.as_str();
    let filename = part.filename.as_deref();
    let unsupported = |what: String| AiMuxError::UnsupportedFunctionality(what);

    match get_top_level_media_type(media_type) {
        kind @ ("image" | "video") => {
            let url = match source {
                FileSource::Url(url) => url.to_string(),
                FileSource::Base64(b64) => {
                    format!("data:{};base64,{}", resolve_full_media_type(part)?, b64)
                }
            };
            let kind = format!("{kind}_url");
            Ok(json!({ "type": kind, kind: { "url": url } }))
        }
        "audio" => {
            let FileSource::Base64(b64) = source else {
                return Err(unsupported("audio file parts with URLs".to_string()));
            };
            let full_media_type = resolve_full_media_type(part)?;
            let format = match full_media_type.as_str() {
                "audio/wav" => "wav",
                "audio/mp3" | "audio/mpeg" => "mp3",
                other => {
                    return Err(unsupported(format!("audio media type {other}")));
                }
            };
            Ok(json!({ "type": "input_audio", "input_audio": { "data": b64, "format": format } }))
        }
        "application" => {
            let FileSource::Base64(b64) = source else {
                return Err(unsupported("PDF file parts with URLs".to_string()));
            };
            let full_media_type = resolve_full_media_type(part)?;
            if full_media_type != "application/pdf" {
                return Err(unsupported(format!(
                    "file part media type {full_media_type}"
                )));
            }
            let filename = filename.unwrap_or("document.pdf");
            Ok(json!({
                "type": "file",
                "file": {
                    "filename": filename,
                    "file_data": format!("data:application/pdf;base64,{b64}"),
                }
            }))
        }
        "text" => {
            let b64 = match source {
                FileSource::Url(url) => return Ok(json!({ "type": "text", "text": url })),
                FileSource::Base64(b64) => b64,
            };
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| unsupported(format!("text file part is not valid base64: {e}")))?;
            let text = String::from_utf8_lossy(&bytes);
            Ok(json!({ "type": "text", "text": text }))
        }
        _ => Err(unsupported(format!("file part media type {media_type}"))),
    }
}

fn convert_assistant_message(
    content: &[AssistantPart],
    spec: &MessageSpec<'_>,
    message_metadata: Map<String, Value>,
) -> Value {
    let key = spec.metadata_key;
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    let metadata = message_metadata;
    for part in content {
        match part {
            AssistantPart::Text(TextPart { text: t, .. }) => text.push_str(t),
            AssistantPart::Reasoning(ReasoningPart { text: t, .. }) => reasoning.push_str(t),
            AssistantPart::ToolCall(ToolCallPart {
                tool_call_id,
                tool_name,
                input,
                provider_options,
                ..
            }) => {
                let arguments = input.to_string();
                let mut call = json!({
                    "id": tool_call_id,
                    "type": "function",
                    "function": { "name": tool_name, "arguments": arguments },
                });
                call = with_metadata(call, wire_metadata(provider_options.as_ref()));
                let signature = provider_options.as_ref().and_then(|options| {
                    options
                        .get(key)
                        .and_then(|options| options.get("thoughtSignature"))
                        .filter(|value| !value.is_null())
                        .or_else(|| {
                            options
                                .get("google")
                                .and_then(|options| options.get("thoughtSignature"))
                        })
                });
                let signature = signature.and_then(|value| match value {
                    Value::Null | Value::Bool(false) => None,
                    Value::String(text) if text.is_empty() => None,
                    Value::Number(number) if number.as_f64() == Some(0.0) => None,
                    Value::String(text) => Some(text.clone()),
                    other => Some(signature_string(other)),
                });
                if let Some(signature) = signature {
                    call["extra_content"] = json!({ "google": { "thought_signature": signature } });
                }
                tool_calls.push(call);
            }
            // Provider-executed results and media have no assistant-content
            // form on this wire.
            _ => {}
        }
    }
    let mut wire = Map::new();
    wire.insert("role".into(), json!("assistant"));
    wire.insert(
        "content".into(),
        if tool_calls.is_empty() || !text.is_empty() {
            json!(text)
        } else {
            Value::Null
        },
    );
    if !reasoning.is_empty() {
        wire.insert("reasoning_content".into(), json!(reasoning));
    }
    if !tool_calls.is_empty() {
        wire.insert("tool_calls".into(), Value::Array(tool_calls));
    }
    wire.extend(metadata);
    Value::Object(wire)
}

// String(value), as used by the compatible package for thought signatures.
fn signature_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Object(_) => "[object Object]".into(),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    String::new()
                } else {
                    signature_string(value)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Number(value) => value.as_f64().map_or_else(
            || value.to_string(),
            |value| {
                if value == 0.0 {
                    "0".into()
                } else {
                    value.to_string()
                }
            },
        ),
        _ => value.to_string(),
    }
}
