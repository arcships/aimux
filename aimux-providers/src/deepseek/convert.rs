//! Prompt to DeepSeek chat messages (`convert-to-deepseek-chat-messages.ts`).

use base64::Engine;
use serde_json::{Map, Value, json};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::message::Role;
use aimux_core::options::ResponseFormat;
use aimux_core::types::Warning;

use super::is_v4_model::is_deepseek_v4_model;
use super::options::{parse_file_part_options, parse_message_options};
use crate::xai::convert::resolve_full_media_type;

const SUPPORTED_IMAGE_MEDIA_TYPES: [&str; 5] = [
    "image/gif",
    "image/jpeg",
    "image/jpg",
    "image/png",
    "image/webp",
];

/// An image file part: where its data is, and what goes with it.
struct ImagePart<'a> {
    source: ImageSource<'a>,
    media_type: &'a str,
    filename: Option<&'a str>,
    provider_options: Option<&'a Value>,
}

enum ImageSource<'a> {
    /// A provider file reference object.
    Reference(&'a Value),
    Url(&'a str),
    /// Inline data, base64.
    Data(String),
}

/// The part as an image file part, when it is one.
fn image_part(part: &ContentPart) -> Option<ImagePart<'_>> {
    let encode =
        |data: &[u8]| ImageSource::Data(base64::engine::general_purpose::STANDARD.encode(data));
    let (source, media_type, filename, provider_options) = match part {
        ContentPart::Image {
            image,
            media_type,
            provider_options,
        } => (encode(image), media_type, None, provider_options),
        ContentPart::File {
            data,
            media_type,
            filename,
            provider_options,
        } => (
            encode(data),
            media_type,
            filename.as_deref(),
            provider_options,
        ),
        ContentPart::FileBase64 {
            data,
            media_type,
            filename,
            provider_options,
        } => (
            ImageSource::Data(data.clone()),
            media_type,
            filename.as_deref(),
            provider_options,
        ),
        ContentPart::FileUrl {
            url,
            media_type,
            provider_options,
        } => (ImageSource::Url(url), media_type, None, provider_options),
        ContentPart::FileReference {
            media_type,
            reference,
            filename,
            provider_options,
        } => (
            ImageSource::Reference(reference),
            media_type,
            filename.as_deref(),
            provider_options,
        ),
        _ => return None,
    };
    (media_type.split('/').next() == Some("image")).then_some(ImagePart {
        source,
        media_type,
        filename,
        provider_options: provider_options.as_ref(),
    })
}

fn part_type(part: &ContentPart) -> &'static str {
    match part {
        ContentPart::Text { .. } => "text",
        ContentPart::Reasoning { .. } => "reasoning",
        ContentPart::ToolCall { .. } => "tool-call",
        ContentPart::ToolResult { .. } => "tool-result",
        _ => "file",
    }
}

fn resolve_deepseek_image_media_type(media_type: &str, data: &str) -> Result<String, AiMuxError> {
    let resolved = resolve_full_media_type(media_type, data);
    if !SUPPORTED_IMAGE_MEDIA_TYPES.contains(&resolved.as_str()) {
        return Err(AiMuxError::UnsupportedFunctionality(format!(
            "DeepSeek image media type {resolved}: DeepSeek supports JPEG, PNG, GIF, and WebP image inputs."
        )));
    }
    Ok(resolved)
}

/// The content part of an image file part.
fn convert_image_part(
    part: &ImagePart<'_>,
    provider_options_name: &str,
) -> Result<Value, AiMuxError> {
    let options = parse_file_part_options(part.provider_options, provider_options_name)?;
    let image_url = |url: &str| {
        let mut image_url = json!({ "url": url });
        if let Some(detail) = &options.image_detail {
            image_url["detail"] = json!(detail);
        }
        json!({ "type": "image_url", "image_url": image_url })
    };
    match &part.source {
        ImageSource::Reference(reference) => {
            let file_id = reference
                .get("deepseek")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AiMuxError::InvalidArgument(
                        "No provider reference found for provider 'deepseek'.".to_string(),
                    )
                })?;
            Ok(json!({ "type": "file", "file_id": file_id }))
        }
        ImageSource::Url(url) => {
            resolve_deepseek_image_media_type(part.media_type, "")?;
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
        ImageSource::Data(data) => {
            let media_type = resolve_deepseek_image_media_type(part.media_type, data)?;
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
            if let Some(filename) = part.filename {
                file["filename"] = json!(filename);
            }
            Ok(file)
        }
    }
}

/// The messages of a request and the warnings raised while converting.
pub(crate) struct ConvertedMessages {
    pub messages: Vec<Value>,
    pub warnings: Vec<Warning>,
}

fn join_text(content: &[ContentPart]) -> String {
    content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
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
        .rposition(|message| message.role == Role::User);

    for (index, message) in prompt.iter().enumerate() {
        let options =
            parse_message_options(message.provider_options.as_ref(), provider_options_name)?;
        if options.prefix == Some(true) && message.role != Role::Assistant {
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

        match message.role {
            Role::System => {
                let mut wire = Map::new();
                wire.insert("role".into(), json!("system"));
                wire.insert("content".into(), json!(join_text(&message.content)));
                name(&mut wire);
                messages.push(Value::Object(wire));
            }

            Role::User => {
                let images: Vec<Option<ImagePart<'_>>> =
                    message.content.iter().map(image_part).collect();
                let has_image_part = images.iter().any(Option::is_some);
                let mut content = Vec::new();
                for (part, image) in message.content.iter().zip(&images) {
                    match (part, image) {
                        (ContentPart::Text { text, .. }, _) => {
                            content.push(json!({ "type": "text", "text": text }));
                        }
                        (_, Some(image)) => {
                            content.push(convert_image_part(image, provider_options_name)?);
                        }
                        (part, None) => warnings.push(Warning::Unsupported {
                            feature: format!("user message part type: {}", part_type(part)),
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
                        json!(join_text(&message.content))
                    },
                );
                name(&mut wire);
                messages.push(Value::Object(wire));
            }

            Role::Assistant => {
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
                for part in &message.content {
                    match part {
                        ContentPart::Text { text: t, .. } => text.push_str(t),
                        ContentPart::Reasoning { text: t, .. } => {
                            // R1 must not receive prior reasoning; V4 requires it.
                            if last_user_message_index.is_some_and(|last| index <= last)
                                && !is_deepseek_v4
                            {
                                continue;
                            }
                            reasoning.get_or_insert_with(String::new).push_str(t);
                        }
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
                                "arguments": if input.is_null() { "{}".to_string() } else { input.to_string() },
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

            Role::Tool => {
                if options.name.is_some() {
                    warnings.push(Warning::Unsupported {
                        feature: "message name on tool messages".to_string(),
                        details: None,
                    });
                }
                for part in &message.content {
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

    Ok(ConvertedMessages { messages, warnings })
}
