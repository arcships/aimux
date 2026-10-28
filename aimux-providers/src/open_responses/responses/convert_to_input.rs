//! responses/convert-to-open-responses-input.ts.

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, TextPart, ToolCallPart,
    ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::shared::FileData;
use aimux_core::types::Warning;
use serde_json::{Value, json};

// == Prompt conversion ==

/// Convert a `LanguageModelPrompt` to the Open Responses input format.
///
/// Mirrors the TS `convertToOpenResponsesInput`. System messages become
/// `instructions`; user/assistant/tool messages become `input` items.
#[must_use]
pub fn convert_to_open_responses_input(
    prompt: &LanguageModelPrompt,
) -> (Value, Option<String>, Vec<Warning>) {
    convert_to_open_responses_input_with_namespace(prompt, "open-responses")
}

pub(crate) fn convert_to_open_responses_input_with_namespace(
    prompt: &LanguageModelPrompt,
    provider_options_name: &str,
) -> (Value, Option<String>, Vec<Warning>) {
    let mut input: Vec<Value> = Vec::new();
    let mut warnings = Vec::new();
    let mut system_messages: Vec<String> = Vec::new();

    for msg in prompt {
        match msg {
            LanguageModelMessage::System { content, .. } => {
                system_messages.push(content.clone());
            }
            LanguageModelMessage::User { content, .. } => {
                let user_content =
                    convert_user_content(content, provider_options_name, &mut warnings);
                input.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": user_content,
                }));
            }
            LanguageModelMessage::Assistant { content, .. } => {
                let mut assistant_content: Vec<Value> = Vec::new();
                let mut assistant_message_id: Option<String> = None;

                for part in content {
                    match part {
                        AssistantPart::Text(TextPart {
                            text,
                            provider_options,
                        }) => {
                            let data = provider_options
                                .as_ref()
                                .and_then(|options| options.get(provider_options_name));
                            let item_id = data
                                .and_then(|data| data.get("itemId"))
                                .and_then(Value::as_str)
                                .map(str::to_string);
                            if !assistant_content.is_empty() && assistant_message_id != item_id {
                                flush_assistant_content(
                                    &mut assistant_content,
                                    assistant_message_id.take(),
                                    &mut input,
                                );
                            }
                            assistant_message_id = item_id;
                            let mut output_text = json!({ "type": "output_text", "text": text });
                            if let Some(annotations) = data
                                .and_then(|data| data.get("annotations"))
                                .and_then(url_citations)
                            {
                                output_text["annotations"] = annotations;
                            }
                            assistant_content.push(output_text);
                        }
                        AssistantPart::ToolCall(ToolCallPart {
                            tool_call_id,
                            tool_name,
                            input: tool_input,
                            provider_options,
                            ..
                        }) => {
                            let arguments = match tool_input {
                                Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            flush_assistant_content(
                                &mut assistant_content,
                                assistant_message_id.take(),
                                &mut input,
                            );
                            let mut function_call = json!({
                                "type": "function_call",
                                "call_id": tool_call_id,
                                "name": tool_name,
                                "arguments": arguments,
                            });
                            if let Some(item_id) = provider_options
                                .as_ref()
                                .and_then(|options| options.get(provider_options_name))
                                .and_then(|data| data.get("itemId"))
                                .and_then(Value::as_str)
                            {
                                function_call["id"] = json!(item_id);
                            }
                            input.push(function_call);
                        }
                        AssistantPart::Reasoning(part) => {
                            flush_assistant_content(
                                &mut assistant_content,
                                assistant_message_id.take(),
                                &mut input,
                            );
                            let metadata = part
                                .provider_options
                                .as_ref()
                                .and_then(|options| options.get(provider_options_name));
                            let parse_parts = |key: &str, kind: &str| {
                                metadata
                                    .and_then(|data| data.get(key))
                                    .and_then(Value::as_array)
                                    .filter(|parts| {
                                        parts.iter().all(|part| {
                                            part["type"] == kind && part["text"].is_string()
                                        })
                                    })
                                    .map(|parts| {
                                        json!(
                                            parts
                                                .iter()
                                                .map(
                                                    |part| json!({"type":kind, "text":part["text"]})
                                                )
                                                .collect::<Vec<_>>()
                                        )
                                    })
                            };
                            let mut reasoning = json!({"type":"reasoning", "summary":parse_parts("reasoningSummary", "summary_text").unwrap_or_else(|| json!([]))});
                            if let Some(id) = metadata
                                .and_then(|data| data.get("itemId"))
                                .and_then(Value::as_str)
                            {
                                reasoning["id"] = json!(id);
                            }
                            if let Some(content) = parse_parts("reasoningContent", "reasoning_text")
                            {
                                reasoning["content"] = content;
                            } else if !metadata
                                .is_some_and(|data| data.contains_key("reasoningContent"))
                                && !part.text.is_empty()
                            {
                                reasoning["content"] =
                                    json!([{"type":"reasoning_text", "text":part.text}]);
                            }
                            if let Some(encrypted) = metadata
                                .and_then(|data| data.get("reasoningEncryptedContent"))
                                .and_then(Value::as_str)
                            {
                                reasoning["encrypted_content"] = json!(encrypted);
                            }
                            if let Some(previous) = input.last_mut().filter(|previous| {
                                reasoning.get("id").is_some()
                                    && previous["type"] == "reasoning"
                                    && previous.get("id") == reasoning.get("id")
                            }) {
                                if let Some(content) =
                                    reasoning.get("content").and_then(Value::as_array)
                                {
                                    if previous.get("content").is_none() {
                                        previous["content"] = json!([]);
                                    }
                                    if let Some(previous_content) =
                                        previous["content"].as_array_mut()
                                    {
                                        previous_content.extend(content.iter().cloned());
                                    }
                                }
                            } else {
                                input.push(reasoning);
                            }
                        }
                        _ => {}
                    }
                }
                flush_assistant_content(
                    &mut assistant_content,
                    assistant_message_id.take(),
                    &mut input,
                );
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
                    let content_value =
                        resolve_tool_result_output(output, provider_options_name, &mut warnings);
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": tool_call_id,
                        "output": content_value,
                    }));
                }
            }
        }
    }

    let instructions = if system_messages.is_empty() {
        None
    } else {
        Some(system_messages.join("\n"))
    };

    (json!(input), instructions, warnings)
}

/// Push the pending assistant text as one message, with the item id the text
/// parts replayed (`providerOptions.<name>.itemId`) when they had one.
fn flush_assistant_content(content: &mut Vec<Value>, id: Option<String>, input: &mut Vec<Value>) {
    if content.is_empty() {
        return;
    }
    let mut message =
        json!({"type":"message", "role":"assistant", "content":std::mem::take(content)});
    if let Some(id) = id {
        message["id"] = json!(id);
    }
    input.push(message);
}

/// The `url_citation` annotations of a replayed output text, or `None` when
/// the value is not a list of them (parseOutputTextAnnotations).
pub(super) fn url_citations(value: &Value) -> Option<Value> {
    let annotations = value.as_array()?;
    annotations
        .iter()
        .map(|annotation| {
            (annotation["type"] == "url_citation"
                && annotation["start_index"].is_number()
                && annotation["end_index"].is_number()
                && annotation["url"].is_string()
                && annotation["title"].is_string())
            .then(|| {
                json!({
                    "type": "url_citation",
                    "start_index": annotation["start_index"],
                    "end_index": annotation["end_index"],
                    "url": annotation["url"],
                    "title": annotation["title"],
                })
            })
        })
        .collect::<Option<Vec<_>>>()
        .map(Value::Array)
}

/// Convert user message content parts to the Open Responses format.
fn convert_user_content(
    content: &[UserPart],
    provider_options_name: &str,
    warnings: &mut Vec<Warning>,
) -> Value {
    let mut parts: Vec<Value> = Vec::new();
    for part in content {
        match part {
            UserPart::Text(TextPart { text, .. }) => {
                parts.push(json!({ "type": "input_text", "text": text }));
            }
            UserPart::File(
                part @ FilePart {
                    data,
                    media_type,
                    filename,
                    ..
                },
            ) => {
                let image = top_level_media_type(media_type) == "image";
                match data {
                    FileData::Data { data } => {
                        let media_type = match aimux_provider_utils::resolve_full_media_type(part) {
                            Ok(media_type) => media_type,
                            Err(error) => {
                                warnings.push(Warning::Other {
                                    message: error.to_string(),
                                });
                                continue;
                            }
                        };
                        let b64 = aimux_provider_utils::convert_to_base64(data).into_owned();
                        let data_url = format!("data:{media_type};base64,{b64}");
                        parts.push(if image {
                            json!({
                                "type": "input_image",
                                "image_url": data_url,
                                "detail": image_detail(part, provider_options_name),
                            })
                        } else {
                            json!({
                                "type": "input_file",
                                "filename": filename.as_deref().unwrap_or("data"),
                                "file_data": data_url,
                            })
                        });
                    }
                    FileData::Url { url, .. } => {
                        parts.push(if image {
                            json!({
                                "type": "input_image",
                                "image_url": url,
                                "detail": image_detail(part, provider_options_name),
                            })
                        } else {
                            json!({ "type": "input_file", "file_url": url })
                        });
                    }
                    FileData::Reference { .. } => {
                        warnings.push(Warning::Other {
                            message: "unsupported file part with provider reference".to_string(),
                        });
                    }
                    FileData::Text { .. } => {
                        warnings.push(Warning::Other {
                            message: "unsupported text file part".to_string(),
                        });
                    }
                }
            }
        }
    }
    json!(parts)
}

/// Fail before sending where `convertToOpenResponsesInput` throws: a user
/// file given as a provider reference or as text, and an inline user or
/// tool-result file whose media type cannot be resolved.
pub(super) fn validate_file_media(prompt: &LanguageModelPrompt) -> Result<(), AiMuxError> {
    for message in prompt {
        match message {
            LanguageModelMessage::User { content, .. } => {
                for part in content {
                    let UserPart::File(part) = part else {
                        continue;
                    };
                    match part.data {
                        FileData::Data { .. } => {
                            aimux_provider_utils::resolve_full_media_type(part)?;
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
                        FileData::Url { .. } => {}
                    }
                }
            }
            LanguageModelMessage::Tool { content, .. } => {
                for part in content {
                    let ToolPart::ToolResult(ToolResultPart {
                        output: ToolResultOutput::Content { value },
                        ..
                    }) = part
                    else {
                        continue;
                    };
                    for item in value {
                        if let ToolResultContent::File(part) = item
                            && matches!(part.data, FileData::Data { .. })
                        {
                            aimux_provider_utils::resolve_full_media_type(part)?;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Resolve a tool-result `output` value into the Open Responses `output`
/// field, mirroring the TS convert logic.
fn resolve_tool_result_output(
    output: &ToolResultOutput,
    provider_options_name: &str,
    warnings: &mut Vec<Warning>,
) -> Value {
    let ToolResultOutput::Content { value } = output else {
        return aimux_provider_utils::tool_result_to_content(output);
    };
    let mut parts = Vec::new();
    for item in value {
        match item {
            ToolResultContent::Text(part) => {
                parts.push(json!({"type":"input_text", "text":part.text}))
            }
            ToolResultContent::File(part) => {
                let image = top_level_media_type(&part.media_type) == "image";
                let mut converted = match &part.data {
                    FileData::Data { data } => {
                        let media_type = match aimux_provider_utils::resolve_full_media_type(part) {
                            Ok(media_type) => media_type,
                            Err(error) => {
                                warnings.push(Warning::Other {
                                    message: error.to_string(),
                                });
                                continue;
                            }
                        };
                        let b64 = aimux_provider_utils::convert_to_base64(data).into_owned();
                        let data_url = format!("data:{media_type};base64,{b64}");
                        if image {
                            json!({"type":"input_image", "image_url":data_url})
                        } else {
                            json!({"type":"input_file", "filename":part.filename.as_deref().unwrap_or("data"), "file_data":data_url})
                        }
                    }
                    FileData::Url { url, .. } => {
                        if image {
                            json!({"type":"input_image", "image_url":url})
                        } else {
                            json!({"type":"input_file", "file_url":url})
                        }
                    }
                    data => {
                        warnings.push(Warning::Other {
                            message: format!(
                                "unsupported tool content part type: file with data type: {}",
                                if matches!(data, FileData::Reference { .. }) {
                                    "reference"
                                } else {
                                    "text"
                                }
                            ),
                        });
                        continue;
                    }
                };
                if image {
                    converted["detail"] = json!(image_detail(part, provider_options_name));
                }
                parts.push(converted);
            }
            ToolResultContent::Custom { .. } => warnings.push(Warning::Other {
                message: "unsupported tool content part type: custom".into(),
            }),
        }
    }
    json!(parts)
}

/// The `detail` of an input image: `providerOptions.<name>.imageDetail` when
/// it is `low`, `high` or `auto`, else `auto` (getImageDetail).
fn image_detail<'a>(part: &'a FilePart, provider_options_name: &str) -> &'a str {
    part.provider_options
        .as_ref()
        .and_then(|options| options.get(provider_options_name))
        .and_then(|options| options.get("imageDetail"))
        .and_then(Value::as_str)
        .filter(|detail| matches!(*detail, "low" | "high" | "auto"))
        .unwrap_or("auto")
}

/// Extract the top-level media type (e.g. "image" from "image/png").
fn top_level_media_type(media_type: &str) -> &str {
    media_type.split('/').next().unwrap_or("")
}
