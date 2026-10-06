//! Conversion between `LanguageModelPrompt` and OpenAI API format.

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, TextPart, ToolPart,
    ToolResultContent, ToolResultOutput, UserPart,
};
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::shared::{FileBytes, FileData, JsonObject, SharedProviderOptions};
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::{FinishReason, FinishReasonUnified, ReasoningEffort, Warning};
use aimux_provider_utils::{get_top_level_media_type, resolve_full_media_type};
use serde::Serialize;
use serde_json::{Value, json};

/// Public capability enum used by the conversion helpers (moved to
/// `convert_common` in M10; re-exported for API compatibility).
pub use super::convert_common::SystemMessageMode;
use super::convert_common::{ModelCapabilities, get_model_capabilities};

// ── Model capabilities ──────────────────────────────────────────────────────
// `GptVersion` / `get_gpt_version` / `get_o_series_version` /
// `ModelCapabilities` / `SystemMessageMode` / `get_model_capabilities` live in
// `super::convert_common` and are shared with the Responses converter (M10).

/// Apply the upstream structured-output compatibility rules only at schema positions.
pub(crate) fn normalize_json_schema(
    schema: &Value,
    warnings: &mut Vec<Warning>,
) -> Result<Value, AiMuxError> {
    fn walk(value: &mut Value, removed: &mut [bool; 2]) -> Result<(), AiMuxError> {
        let Some(obj) = value.as_object_mut() else {
            return Ok(());
        };
        if let Some(names) = obj.get("propertyNames").filter(|v| !v.is_null()) {
            if names.get("type").and_then(Value::as_str) != Some("string") {
                return Err(AiMuxError::UnsupportedFunctionality(
                    "JSON Schema propertyNames that does not use a string schema".into(),
                ));
            }
            obj.remove("propertyNames");
            removed[0] = true;
        }
        if obj
            .get("pattern")
            .and_then(Value::as_str)
            .is_some_and(contains_lookaround)
        {
            obj.remove("pattern");
            removed[1] = true;
        }
        for key in [
            "properties",
            "patternProperties",
            "definitions",
            "$defs",
            "dependencies",
        ] {
            if let Some(Value::Object(record)) = obj.get_mut(key) {
                for child in record.values_mut().filter(|v| !v.is_array()) {
                    walk(child, removed)?;
                }
            }
        }
        for key in [
            "additionalProperties",
            "additionalItems",
            "items",
            "contains",
            "not",
            "allOf",
            "anyOf",
            "oneOf",
            "if",
            "then",
            "else",
        ] {
            if let Some(child) = obj.get_mut(key) {
                if let Value::Array(children) = child {
                    for child in children {
                        walk(child, removed)?;
                    }
                } else {
                    walk(child, removed)?;
                }
            }
        }
        Ok(())
    }
    let mut result = schema.clone();
    let mut removed = [false; 2];
    walk(&mut result, &mut removed)?;
    for (index, feature, details) in [
        (
            0,
            "JSON Schema propertyNames",
            "OpenAI does not support JSON Schema propertyNames. It was removed before sending the schema, so OpenAI will not enforce property-name constraints.",
        ),
        (
            1,
            "JSON Schema pattern with regex lookaround",
            "OpenAI does not support regex lookaround in JSON Schema patterns. The pattern was removed before sending the schema, so OpenAI will not enforce that constraint.",
        ),
    ] {
        if removed[index] {
            warnings.push(Warning::Compatibility {
                feature: feature.into(),
                details: Some(details.into()),
            });
        }
    }
    Ok(result)
}

fn contains_lookaround(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let (mut escaped, mut in_class) = (false, false);
    for (i, &byte) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'[' => in_class = true,
            b']' => in_class = false,
            b'(' if !in_class
                && bytes.get(i + 1) == Some(&b'?')
                && (matches!(bytes.get(i + 2), Some(b'=' | b'!'))
                    || (bytes.get(i + 2) == Some(&b'<')
                        && matches!(bytes.get(i + 3), Some(b'=' | b'!')))) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

// ── Prepared tools ──────────────────────────────────────────────────────────

/// A warning emitted while preparing tools (mirrors the V4 `SharedV4Warning`
/// `unsupported` shape used by the TS `prepareChatTools`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolWarning {
    #[serde(rename = "type")]
    pub warning_type: String,
    pub feature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// The result of preparing tools for an OpenAI request body.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub tool_warnings: Vec<ToolWarning>,
}

/// Prepare `FunctionTool`s into the OpenAI `tools` / `tool_choice` JSON shape.
#[must_use]
pub fn prepare_tools(
    tools: &Option<Vec<FunctionTool>>,
    tool_choice: Option<&ToolChoice>,
) -> PreparedTools {
    let non_empty = tools.as_ref().filter(|&t| !t.is_empty());

    let tool_warnings: Vec<ToolWarning> = Vec::new();

    let tools_opt = match non_empty {
        None => None,
        Some(tools) => {
            let openai_tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    let mut func = json!({
                        "name": t.name,
                        "parameters": t.input_schema,
                    });
                    if let Some(ref desc) = t.description {
                        func["description"] = json!(desc);
                    }
                    if let Some(strict) = t.strict {
                        func["strict"] = json!(strict);
                    }
                    json!({ "type": "function", "function": func })
                })
                .collect();
            Some(openai_tools)
        }
    };

    let tool_choice_opt = match (&tools_opt, tool_choice) {
        (None, _) => None,
        (Some(_), None) => None,
        (Some(_), Some(tc)) => match tc {
            ToolChoice::Auto => Some(json!("auto")),
            ToolChoice::None => Some(json!("none")),
            ToolChoice::Required => Some(json!("required")),
            ToolChoice::Tool { tool_name } => {
                Some(json!({ "type": "function", "function": { "name": tool_name } }))
            }
        },
    };

    PreparedTools {
        tools: tools_opt,
        tool_choice: tool_choice_opt,
        tool_warnings,
    }
}

// ── Message conversion ──────────────────────────────────────────────────────

/// Convert a `LanguageModelPrompt` to OpenAI `messages` array.
///
/// Panics on conversion failure. Production paths use the fallible variant
/// [`convert_prompt_to_openai_messages_with_mode_fallible`]; this panic
/// wrapper exists only for integration tests under `tests/`. It is
/// `#[doc(hidden)]` and `#[deprecated]` so it neither appears on the public
/// API surface nor can be pulled in by accident (release uses
/// `panic = "abort"`, so reaching a panic here via FFI would kill the host
/// process).
#[doc(hidden)]
#[deprecated(
    since = "0.2.1",
    note = "panics on failure; use convert_prompt_to_openai_messages_with_mode_fallible instead (issue #90 R1)"
)]
#[must_use]
pub fn convert_prompt_to_openai_messages(prompt: &LanguageModelPrompt) -> Vec<Value> {
    convert_prompt_to_openai_messages_with_mode_fallible(prompt, SystemMessageMode::System)
        .expect("convert_prompt_to_openai_messages: conversion failed")
}

/// Convert a `LanguageModelPrompt` to OpenAI `messages` array with a system
/// message mode.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when a message part cannot be
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

/// Convert a `LanguageModelPrompt` to OpenAI `messages` array with a system
/// message mode.
///
/// Panics on conversion failure. Production paths use the fallible variant
/// [`convert_prompt_to_openai_messages_with_mode_fallible`]; this panic
/// wrapper exists only for integration tests under `tests/`. It is
/// `#[doc(hidden)]` and `#[deprecated]` so it neither appears on the public
/// API surface nor can be pulled in by accident (release uses
/// `panic = "abort"`, so reaching a panic here via FFI would kill the host
/// process).
#[doc(hidden)]
#[deprecated(
    since = "0.2.1",
    note = "panics on failure; use convert_prompt_to_openai_messages_with_mode_fallible instead (issue #90 R1)"
)]
#[must_use]
pub fn convert_prompt_to_openai_messages_with_mode(
    prompt: &LanguageModelPrompt,
    system_message_mode: SystemMessageMode,
) -> Vec<Value> {
    convert_prompt_to_openai_messages_with_mode_fallible(prompt, system_message_mode)
        .expect("convert_prompt_to_openai_messages_with_mode: conversion failed")
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

/// Resolve a provider reference, throwing if the provider is not found.
fn resolve_provider_reference(
    reference: &std::collections::HashMap<String, String>,
    provider: &str,
) -> Result<String, String> {
    reference.get(provider).cloned().ok_or_else(|| {
        let mut available: Vec<&str> = reference.keys().map(String::as_str).collect();
        available.sort_unstable();
        format!(
            "No provider reference found for provider '{}'. Available providers: {}",
            provider,
            available.join(", ")
        )
    })
}

/// Convert a file part to the OpenAI format, handling images, audio, and PDF.
fn convert_file_part_to_openai(file: &FilePart, part_index: usize) -> Result<Value, AiMuxError> {
    use base64::Engine;
    let FilePart {
        data,
        media_type,
        filename,
        provider_options,
    } = file;
    let prompt_cache_breakpoint = get_prompt_cache_breakpoint(provider_options);
    let (data_b64, url) = match data {
        FileData::Reference { reference } => {
            let file_id = resolve_provider_reference(reference, "openai")
                .map_err(AiMuxError::InvalidArgument)?;
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
            Some(match data {
                FileBytes::Binary(bytes) => base64::engine::general_purpose::STANDARD.encode(bytes),
                FileBytes::Base64(data) => data.clone(),
            }),
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
            return Err(AiMuxError::InvalidArgument(
                "image part has no data or url".into(),
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
        let b64 =
            data_b64.ok_or_else(|| AiMuxError::InvalidArgument("audio part has no data".into()))?;
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

    let b64 = data_b64.ok_or_else(|| AiMuxError::InvalidArgument("PDF part has no data".into()))?;
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

pub(crate) fn tool_result_to_content(output: &ToolResultOutput) -> Value {
    Value::String(match output {
        ToolResultOutput::Text { value, .. } | ToolResultOutput::ErrorText { value, .. } => {
            value.clone()
        }
        ToolResultOutput::ExecutionDenied { reason, .. } => reason
            .clone()
            .unwrap_or_else(|| "Tool call execution denied.".to_string()),
        ToolResultOutput::Json { value, .. } | ToolResultOutput::ErrorJson { value, .. } => {
            value.to_string()
        }
        ToolResultOutput::Content { value } => tool_result_content_value(value).to_string(),
    })
}

pub(crate) fn tool_result_content_value(content: &[ToolResultContent]) -> Value {
    json!(
        content
            .iter()
            .map(|part| {
                let (mut item, provider_options) = match part {
                    ToolResultContent::Text(part) => (
                        json!({"type":"text", "text":part.text}),
                        &part.provider_options,
                    ),
                    ToolResultContent::File(part) => {
                        let data = match &part.data {
                            FileData::Data { data } => {
                                let data = match data {
                                    FileBytes::Base64(data) => json!(data),
                                    FileBytes::Binary(bytes) => Value::Object(
                                        bytes
                                            .iter()
                                            .enumerate()
                                            .map(|(i, byte)| (i.to_string(), json!(byte)))
                                            .collect(),
                                    ),
                                };
                                json!({"type":"data", "data":data})
                            }
                            FileData::Url { url, original_url } => {
                                let mut data = json!({"type":"url", "url":url});
                                if let Some(original_url) = original_url {
                                    data["originalUrl"] = json!(original_url);
                                }
                                data
                            }
                            FileData::Reference { reference } => {
                                json!({"type":"reference", "reference":reference})
                            }
                            FileData::Text { text } => json!({"type":"text", "text":text}),
                        };
                        let mut item =
                            json!({"type":"file", "data":data, "mediaType":part.media_type});
                        if let Some(filename) = &part.filename {
                            item["filename"] = json!(filename);
                        }
                        (item, &part.provider_options)
                    }
                    ToolResultContent::Custom { provider_options } => {
                        (json!({"type":"custom"}), provider_options)
                    }
                };
                if let Some(provider_options) = provider_options {
                    item["providerOptions"] = json!(provider_options);
                }
                item
            })
            .collect::<Vec<_>>()
    )
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

// ── Request body ────────────────────────────────────────────────────────────

/// Result of building a request body, including warnings.
#[derive(Debug, Clone)]
pub struct RequestBodyResult {
    pub body: Value,
    pub warnings: Vec<Warning>,
}

/// Parse declared namespace fields, discarding unknown keys like a default z.object.
///
/// # Errors
/// Returns a parse error when a declared field fails its schema.
pub(crate) fn parse_option_fields(
    options: &JsonObject,
    provider: &str,
    parse: impl Fn(&str, &Value) -> Option<Result<Value, ()>>,
) -> Result<JsonObject, AiMuxError> {
    let mut parsed = JsonObject::new();
    for (key, value) in options {
        if let Some(result) = parse(key, value) {
            parsed.insert(
                key.clone(),
                result.map_err(|()| {
                    AiMuxError::InvalidArgument(format!("invalid {provider} provider options"))
                })?,
            );
        }
    }
    Ok(parsed)
}

fn parse_chat_provider_options(
    options: &Option<SharedProviderOptions>,
    provider: &str,
) -> Result<Option<SharedProviderOptions>, AiMuxError> {
    let provider = if provider == "azure" {
        "openai"
    } else {
        provider
    };
    if !matches!(provider, "openai" | "groq" | "deepseek") {
        return Ok(options.clone());
    }
    let Some(namespace) = options.as_ref().and_then(|options| options.get(provider)) else {
        return Ok(None);
    };
    let parsed = parse_option_fields(namespace, provider, |key, value| {
        let one_of = |values: &[&str]| value.as_str().is_some_and(|s| values.contains(&s));
        let valid = match (provider, key) {
            ("openai", "logitBias") => {
                return Some(value.as_object().ok_or(()).and_then(|object| {
                    let mut parsed = JsonObject::new();
                    for (key, value) in object {
                        if !value.is_number() {
                            return Err(());
                        }
                        let key = key.trim_matches(|c| {
                            matches!(c,
                                '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}'
                                | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
                                | '\u{205f}' | '\u{3000}' | '\u{feff}'
                            )
                        });
                        let number = if key.is_empty() {
                            0.0
                        } else if let Some((digits, radix)) = key
                            .strip_prefix("0x")
                            .or_else(|| key.strip_prefix("0X"))
                            .map(|s| (s, 16))
                            .or_else(|| {
                                key.strip_prefix("0o")
                                    .or_else(|| key.strip_prefix("0O"))
                                    .map(|s| (s, 8))
                            })
                            .or_else(|| {
                                key.strip_prefix("0b")
                                    .or_else(|| key.strip_prefix("0B"))
                                    .map(|s| (s, 2))
                            })
                        {
                            if digits.is_empty() {
                                return Err(());
                            }
                            let bits_per_digit = match radix {
                                16 => 4,
                                8 => 3,
                                _ => 1,
                            };
                            let mut significant_bits = 0usize;
                            let mut mantissa = 0u64;
                            let mut guard = false;
                            let mut sticky = false;
                            for digit in digits.chars() {
                                let digit = digit.to_digit(radix).ok_or(())?;
                                for shift in (0..bits_per_digit).rev() {
                                    let bit = (digit >> shift) & 1;
                                    if significant_bits == 0 && bit == 0 {
                                        continue;
                                    }
                                    if significant_bits < 53 {
                                        mantissa = (mantissa << 1) | u64::from(bit);
                                    } else if significant_bits == 53 {
                                        guard = bit != 0;
                                    } else {
                                        sticky |= bit != 0;
                                    }
                                    significant_bits += 1;
                                }
                            }
                            if guard && (sticky || mantissa & 1 != 0) {
                                mantissa += 1;
                            }
                            let exponent = significant_bits.saturating_sub(53);
                            if exponent > 1023 {
                                f64::INFINITY
                            } else {
                                mantissa as f64 * 2.0f64.powi(exponent as i32)
                            }
                        } else {
                            key.parse::<f64>().map_err(|_| ())?
                        };
                        if !number.is_finite() {
                            return Err(());
                        }
                        let key = if number == 0.0 {
                            "0".to_string()
                        } else if number.abs() >= 1e21 || number.abs() < 1e-6 {
                            let scientific = format!("{number:e}");
                            let (mantissa, exponent) = scientific.split_once('e').ok_or(())?;
                            let exponent = exponent.parse::<i32>().map_err(|_| ())?;
                            format!("{mantissa}e{exponent:+}")
                        } else {
                            number.to_string()
                        };
                        parsed.insert(key, value.clone());
                    }
                    Ok(Value::Object(parsed))
                }));
            }
            ("openai", "logprobs") => value.is_boolean() || value.is_number(),
            ("openai", "maxCompletionTokens") => value.is_number(),
            ("openai", "metadata") => value.as_object().is_some_and(|object| {
                object.iter().all(|(key, value)| {
                    key.encode_utf16().count() <= 64
                        && value
                            .as_str()
                            .is_some_and(|s| s.encode_utf16().count() <= 512)
                })
            }),
            ("openai", "prediction") => value.is_object(),
            ("openai", "reasoningEffort") => {
                one_of(&["none", "minimal", "low", "medium", "high", "xhigh", "max"])
            }
            ("openai", "serviceTier") => {
                one_of(&["auto", "flex", "priority", "fast", "ultrafast", "default"])
            }
            ("openai", "textVerbosity") => one_of(&["low", "medium", "high"]),
            ("openai", "promptCacheRetention") => one_of(&["in_memory", "24h"]),
            ("openai", "systemMessageMode") => one_of(&["system", "developer", "remove"]),
            ("openai", "user" | "promptCacheKey" | "safetyIdentifier") | ("groq", "user") => {
                value.is_string()
            }
            ("openai", "parallelToolCalls" | "store" | "strictJsonSchema" | "forceReasoning")
            | ("groq", "parallelToolCalls" | "structuredOutputs" | "strictJsonSchema")
            | ("deepseek", "logprobs" | "strictJsonSchema") => value.is_boolean(),
            ("groq", "reasoningFormat") => one_of(&["parsed", "raw", "hidden"]),
            ("groq", "reasoningEffort") => one_of(&["none", "default", "low", "medium", "high"]),
            ("groq", "serviceTier") => one_of(&["on_demand", "performance", "flex", "auto"]),
            ("deepseek", "reasoningEffort") => one_of(&["low", "medium", "high", "xhigh", "max"]),
            ("deepseek", "topLogprobs") => value
                .as_f64()
                .is_some_and(|n| n.fract() == 0.0 && (0.0..=20.0).contains(&n)),
            ("deepseek", "userId") => value.as_str().is_some_and(|s| {
                !s.is_empty()
                    && s.len() <= 512
                    && s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
            }),
            ("openai", "promptCacheOptions") | ("deepseek", "thinking") => {
                return Some(value.as_object().ok_or(()).and_then(|object| {
                    parse_option_fields(object, provider, |key, value| {
                        let valid = match (provider, key) {
                            ("openai", "mode") => value
                                .as_str()
                                .is_some_and(|s| matches!(s, "implicit" | "explicit")),
                            ("openai", "ttl") => value.as_str() == Some("30m"),
                            ("deepseek", "type") => value
                                .as_str()
                                .is_some_and(|s| matches!(s, "adaptive" | "enabled" | "disabled")),
                            _ => return None,
                        };
                        Some(if valid { Ok(value.clone()) } else { Err(()) })
                    })
                    .map(Value::Object)
                    .map_err(|_| ())
                }));
            }
            _ => return None,
        };
        Some(if valid { Ok(value.clone()) } else { Err(()) })
    })?;
    Ok(Some(std::collections::HashMap::from([(
        provider.to_string(),
        parsed,
    )])))
}

/// Get a value from provider_options.openai.<key>.
fn openai_option(options: &Option<SharedProviderOptions>, key: &str) -> Option<Value> {
    options
        .as_ref()
        .and_then(|m| m.get("openai"))
        .and_then(|o| o.get(key))
        .cloned()
}

/// Convert `CallOptions` to an OpenAI request body (without warnings), or an
/// [`AiMuxError`] describing what failed.
///
/// # Errors
///
/// Returns the first conversion error from `build_request_body_with_warnings`
/// (e.g. `InvalidArgument` for an unconvertible prompt part).
pub fn build_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<Value, AiMuxError> {
    build_request_body_with_warnings(model_id, options, stream).map(|r| r.body)
}

/// Resolve the effective reasoning effort — direct passthrough (v3: no built-in
/// vendor normalization). `providerOptions.reasoningEffort` wins over top-level
/// `reasoning`; custom top-level levels map verbatim to `reasoning_effort`.
fn resolve_reasoning_effort(
    provider_opts: &Option<SharedProviderOptions>,
    reasoning: &Option<ReasoningEffort>,
) -> Option<String> {
    openai_option(provider_opts, "reasoningEffort")
        .map(|v| {
            v.as_str()
                .map(std::string::ToString::to_string)
                .unwrap_or(v.to_string())
        })
        .or_else(|| {
            if reasoning.is_some_and(ReasoningEffort::is_custom) {
                reasoning.map(|r| r.to_string())
            } else {
                None
            }
        })
}

fn resolve_is_reasoning_model(
    provider_opts: &Option<SharedProviderOptions>,
    caps: &ModelCapabilities,
) -> bool {
    openai_option(provider_opts, "forceReasoning")
        .map(|v| v.as_bool().unwrap_or(false))
        .unwrap_or(caps.is_reasoning_model)
}

fn resolve_system_message_mode(
    provider_opts: &Option<SharedProviderOptions>,
    is_reasoning_model: bool,
    caps: &ModelCapabilities,
) -> SystemMessageMode {
    openai_option(provider_opts, "systemMessageMode")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string))
        .map(|s| match s.as_str() {
            "developer" => SystemMessageMode::Developer,
            "remove" => SystemMessageMode::Remove,
            _ => SystemMessageMode::System,
        })
        .unwrap_or(if is_reasoning_model {
            SystemMessageMode::Developer
        } else {
            caps.system_message_mode
        })
}

/// Insert `max_tokens` / `max_completion_tokens`: reasoning models take the
/// latter, the rest the former; an explicit `maxCompletionTokens` option is
/// always sent as `max_completion_tokens`.
fn apply_max_tokens(
    body: &mut Value,
    options: &CallOptions,
    provider_opts: &Option<SharedProviderOptions>,
    is_reasoning_model: bool,
) {
    let max_completion_tokens_opt = openai_option(provider_opts, "maxCompletionTokens");

    if let Some(max_tokens) = options.max_output_tokens {
        let key = if is_reasoning_model {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        body[key] = json!(max_tokens);
    }
    if let Some(mct) = max_completion_tokens_opt {
        body["max_completion_tokens"] = json!(mct);
    }
}

/// Sampling parameters that survive the reasoning-model / search-preview
/// capability filtering.
#[derive(Default)]
struct SamplingParams {
    temperature: Option<f64>,
    top_p: Option<f64>,
    frequency_penalty: Option<f64>,
    presence_penalty: Option<f64>,
}

/// Remove unsupported sampling settings for reasoning models and the search
/// preview models, pushing a compatibility warning for each removal.
fn strip_sampling_params(
    options: &CallOptions,
    model_id: &str,
    caps: &ModelCapabilities,
    is_reasoning_model: bool,
    resolved_reasoning_effort: &Option<String>,
    warnings: &mut Vec<Warning>,
) -> SamplingParams {
    let mut params = SamplingParams {
        temperature: options.temperature,
        top_p: options.top_p,
        frequency_penalty: options.frequency_penalty,
        presence_penalty: options.presence_penalty,
    };

    if is_reasoning_model {
        let allow_non_reasoning = resolved_reasoning_effort.as_deref() == Some("none")
            && caps.supports_non_reasoning_parameters;

        if !allow_non_reasoning {
            if params.temperature.is_some() {
                params.temperature = None;
                warnings.push(Warning::Unsupported {
                    feature: "temperature".to_string(),
                    details: Some("temperature is not supported for reasoning models".to_string()),
                });
            }
            if params.top_p.is_some() {
                params.top_p = None;
                warnings.push(Warning::Unsupported {
                    feature: "topP".to_string(),
                    details: Some("topP is not supported for reasoning models".to_string()),
                });
            }
        }

        if params.frequency_penalty.is_some() {
            params.frequency_penalty = None;
            warnings.push(Warning::Unsupported {
                feature: "frequencyPenalty".to_string(),
                details: Some("frequencyPenalty is not supported for reasoning models".to_string()),
            });
        }
        if params.presence_penalty.is_some() {
            params.presence_penalty = None;
            warnings.push(Warning::Unsupported {
                feature: "presencePenalty".to_string(),
                details: Some("presencePenalty is not supported for reasoning models".to_string()),
            });
        }
    } else if (model_id.starts_with("gpt-4o-search-preview")
        || model_id.starts_with("gpt-4o-mini-search-preview"))
        && params.temperature.is_some()
    {
        params.temperature = None;
        warnings.push(Warning::Unsupported {
            feature: "temperature".to_string(),
            details: Some(
                "temperature is not supported for the search preview models and has been removed."
                    .to_string(),
            ),
        });
    }

    params
}

/// Write the surviving sampling params (plus stop/seed) into the body.
fn insert_sampling_params(body: &mut Value, params: &SamplingParams, options: &CallOptions) {
    if let Some(temp) = params.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(tp) = params.top_p {
        body["top_p"] = json!(tp);
    }
    if let Some(fp) = params.frequency_penalty {
        body["frequency_penalty"] = json!(fp);
    }
    if let Some(pp) = params.presence_penalty {
        body["presence_penalty"] = json!(pp);
    }
    if let Some(ref stop) = options.stop_sequences {
        body["stop"] = json!(stop);
    }
    if let Some(seed) = options.seed {
        body["seed"] = json!(seed);
    }
}

/// `response_format` → body: a schema is sent as `json_schema` (strict), a
/// bare JSON request as `json_object`.
fn apply_response_format(
    body: &mut Value,
    options: &CallOptions,
    warnings: &mut Vec<Warning>,
) -> Result<(), AiMuxError> {
    let Some(ref rf) = options.response_format else {
        return Ok(());
    };
    match rf {
        ResponseFormat::Text => {}
        ResponseFormat::Json {
            schema,
            name,
            description,
        } => {
            if let Some(schema) = schema {
                let mut schema_obj = json!({});
                schema_obj["schema"] = normalize_json_schema(schema, warnings)?;
                schema_obj["name"] = json!(name.clone().unwrap_or_else(|| "response".to_string()));
                if let Some(d) = description {
                    schema_obj["description"] = json!(d);
                }
                schema_obj["strict"] = openai_option(&options.provider_options, "strictJsonSchema")
                    .unwrap_or(json!(true));
                body["response_format"] = json!({
                    "type": "json_schema",
                    "json_schema": schema_obj,
                });
            } else {
                body["response_format"] = json!({ "type": "json_object" });
            }
        }
    }
    Ok(())
}

fn validate_chat_options(options: &Option<SharedProviderOptions>) -> Result<(), AiMuxError> {
    let invalid =
        |key: &str| AiMuxError::InvalidArgument(format!("invalid openai provider option: {key}"));
    for key in [
        "parallelToolCalls",
        "store",
        "strictJsonSchema",
        "forceReasoning",
    ] {
        if let Some(value) = openai_option(options, key)
            && !value.is_boolean()
        {
            return Err(invalid(key));
        }
    }
    for key in ["user", "promptCacheKey", "safetyIdentifier"] {
        if let Some(value) = openai_option(options, key)
            && !value.is_string()
        {
            return Err(invalid(key));
        }
    }
    for (key, allowed) in [
        (
            "reasoningEffort",
            &["none", "minimal", "low", "medium", "high", "xhigh", "max"][..],
        ),
        (
            "serviceTier",
            &["auto", "flex", "priority", "fast", "ultrafast", "default"][..],
        ),
        ("textVerbosity", &["low", "medium", "high"][..]),
        ("promptCacheRetention", &["in_memory", "24h"][..]),
        ("systemMessageMode", &["system", "developer", "remove"][..]),
    ] {
        if let Some(value) = openai_option(options, key)
            && !value.as_str().is_some_and(|v| allowed.contains(&v))
        {
            return Err(invalid(key));
        }
    }
    if let Some(value) = openai_option(options, "logprobs")
        && !value.is_boolean()
        && !value.is_number()
    {
        return Err(invalid("logprobs"));
    }
    if let Some(value) = openai_option(options, "maxCompletionTokens")
        && !value.is_number()
    {
        return Err(invalid("maxCompletionTokens"));
    }
    for key in ["metadata", "prediction", "logitBias", "promptCacheOptions"] {
        let Some(value) = openai_option(options, key) else {
            continue;
        };
        let Some(obj) = value.as_object() else {
            return Err(invalid(key));
        };
        if key == "metadata"
            && obj.iter().any(|(k, v)| {
                k.chars().count() > 64 || !v.as_str().is_some_and(|v| v.chars().count() <= 512)
            })
        {
            return Err(invalid(key));
        }
        if key == "logitBias"
            && obj
                .iter()
                .any(|(k, v)| k.parse::<f64>().is_err() || !v.is_number())
        {
            return Err(invalid(key));
        }
        if key == "promptCacheOptions"
            && (obj
                .get("mode")
                .is_some_and(|v| !matches!(v.as_str(), Some("implicit" | "explicit")))
                || obj.get("ttl").is_some_and(|v| v != "30m"))
        {
            return Err(invalid(key));
        }
    }
    Ok(())
}

/// Pass through the simple provider-specific options that map 1:1 to a body
/// field.
fn apply_provider_option_passthrough(
    body: &mut Value,
    provider_opts: &Option<SharedProviderOptions>,
) {
    let mut set = |key: &str, body_key: &str| {
        if let Some(val) = openai_option(provider_opts, key) {
            body[body_key] = val;
        }
    };
    set("logitBias", "logit_bias");
    set("user", "user");
    set("parallelToolCalls", "parallel_tool_calls");
    set("textVerbosity", "verbosity");
    set("store", "store");
    set("metadata", "metadata");
    set("prediction", "prediction");
    set("promptCacheKey", "prompt_cache_key");
    set("promptCacheRetention", "prompt_cache_retention");
    set("promptCacheOptions", "prompt_cache_options");
    set("safetyIdentifier", "safety_identifier");
    if let Some(logprobs) = openai_option(provider_opts, "logprobs")
        && (logprobs == true || logprobs.is_number())
    {
        body["logprobs"] = json!(true);
        body["top_logprobs"] = if logprobs.is_number() {
            logprobs
        } else {
            json!(0)
        };
    }
}

/// `service_tier` with model-capability validation.
fn apply_service_tier(
    body: &mut Value,
    provider_opts: &Option<SharedProviderOptions>,
    caps: &ModelCapabilities,
    warnings: &mut Vec<Warning>,
) {
    let service_tier = openai_option(provider_opts, "serviceTier")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string));
    if let Some(ref st) = service_tier {
        match st.as_str() {
            "flex" => {
                if caps.supports_flex_processing {
                    body["service_tier"] = json!(st);
                } else {
                    warnings.push(Warning::Unsupported {
                        feature: "serviceTier".to_string(),
                        details: Some(
                            "flex processing is only available for o3, o4-mini, and gpt-5 models"
                                .to_string(),
                        ),
                    });
                }
            }
            "priority" | "fast" => {
                if caps.supports_priority_processing {
                    body["service_tier"] = json!(st);
                } else {
                    warnings.push(Warning::Unsupported {
                        feature: "serviceTier".to_string(),
                        details: Some(
                            "priority processing is only available for supported models (gpt-4, gpt-5, gpt-5-mini, o3, o4-mini) and requires Enterprise access. gpt-5-nano is not supported".to_string(),
                        ),
                    });
                }
            }
            _ => {
                body["service_tier"] = json!(st);
            }
        }
    }
}

/// Function tools → `tools` / `tool_choice`.
fn apply_tools(
    body: &mut Value,
    options: &CallOptions,
    warnings: &mut Vec<Warning>,
) -> Result<(), AiMuxError> {
    let function_tools: Option<Vec<FunctionTool>> = options.tools.as_ref().map(|tools| {
        tools
            .iter()
            .filter_map(|t| match t {
                Tool::Function(ft) => Some(ft.clone()),
                Tool::Provider(_) => None,
            })
            .collect()
    });

    let mut function_tools = function_tools;
    for tool in function_tools.iter_mut().flatten() {
        tool.input_schema = normalize_json_schema(&tool.input_schema, warnings)?;
    }
    for tool in options.tools.iter().flatten() {
        if matches!(tool, Tool::Provider(_)) {
            warnings.push(Warning::Unsupported {
                feature: "tool type: provider".into(),
                details: None,
            });
        }
    }
    if options
        .tools
        .as_ref()
        .is_some_and(|tools| !tools.is_empty())
        && function_tools.as_ref().is_some_and(Vec::is_empty)
    {
        body["tools"] = json!([]);
    }
    let prepared = prepare_tools(&function_tools, options.tool_choice.as_ref());
    if let Some(tools) = prepared.tools {
        body["tools"] = json!(tools);
        if let Some(tc) = prepared.tool_choice {
            body["tool_choice"] = tc;
        }
    }
    Ok(())
}

/// Convert `CallOptions` to an OpenAI request body, returning warnings.
///
/// Conversion errors propagate to the caller (fail-fast, issue H2): the old
/// behaviour of silently returning `body: null` sent empty requests upstream
/// and made conversion failures invisible.
///
/// # Errors
///
/// Propagates conversion errors, fail-fast: e.g. `InvalidArgument` for an
/// unconvertible prompt part or an invalid option combination.
pub fn build_request_body_with_warnings(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<RequestBodyResult, AiMuxError> {
    let mut warnings: Vec<Warning> = Vec::new();
    let caps = get_model_capabilities(model_id);
    let parsed_provider_opts = parse_chat_provider_options(&options.provider_options, "openai")?;
    let provider_opts = &parsed_provider_opts;

    validate_chat_options(provider_opts)?;
    let mut resolved_reasoning_effort = resolve_reasoning_effort(provider_opts, &options.reasoning);
    if let (Some(effort), Some(supported)) =
        (&resolved_reasoning_effort, caps.supported_reasoning_efforts)
        && !supported.contains(&effort.as_str())
    {
        warnings.push(Warning::Unsupported {
            feature: "reasoningEffort".into(),
            details: Some(format!(
                "{model_id} only supports the following reasoning efforts: {}",
                supported.join(", ")
            )),
        });
        resolved_reasoning_effort = None;
    }
    let is_reasoning_model = resolve_is_reasoning_model(provider_opts, &caps);
    let system_message_mode = resolve_system_message_mode(provider_opts, is_reasoning_model, &caps);

    // OpenAI has no top_k.
    if options.top_k.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "topK".to_string(),
            details: None,
        });
    }

    if system_message_mode == SystemMessageMode::Remove {
        for _ in options
            .prompt
            .iter()
            .filter(|m| matches!(m, LanguageModelMessage::System { .. }))
        {
            warnings.push(Warning::Other {
                message: "system messages are removed for this model".into(),
            });
        }
    }
    let messages =
        convert_prompt_to_openai_messages_with_mode_fallible(&options.prompt, system_message_mode)?;

    let mut body = json!({
        "model": model_id,
        "messages": messages,
    });

    if stream {
        body["stream"] = json!(true);
        body["stream_options"] = json!({ "include_usage": true });
    }

    apply_max_tokens(&mut body, options, provider_opts, is_reasoning_model);

    let sampling = strip_sampling_params(
        options,
        model_id,
        &caps,
        is_reasoning_model,
        &resolved_reasoning_effort,
        &mut warnings,
    );
    insert_sampling_params(&mut body, &sampling, options);

    apply_response_format(&mut body, options, &mut warnings)?;
    apply_provider_option_passthrough(&mut body, provider_opts);

    if caps.supported_reasoning_efforts.is_some() && body.get("prompt_cache_retention").is_some() {
        body.as_object_mut()
            .unwrap()
            .remove("prompt_cache_retention");
        warnings.push(Warning::Unsupported { feature: "promptCacheRetention".into(), details: Some("promptCacheRetention is not supported by sixth-generation and later models; use promptCacheOptions instead".into()) });
    }
    if is_reasoning_model {
        let allow_logprobs = resolved_reasoning_effort.as_deref() == Some("none")
            && caps.supports_non_reasoning_parameters;
        for (key, feature) in [
            ("logprobs", "logprobs"),
            ("logit_bias", "logitBias"),
            ("top_logprobs", "topLogprobs"),
        ] {
            if key == "logprobs" && allow_logprobs {
                continue;
            }
            if body.as_object_mut().unwrap().remove(key).is_some() {
                warnings.push(Warning::Other {
                    message: format!("{feature} is not supported for reasoning models"),
                });
            }
        }
    }

    // Provider reasoning takes precedence over the standardized setting.
    if let Some(ref effort) = resolved_reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }

    apply_service_tier(&mut body, provider_opts, &caps, &mut warnings);
    apply_tools(&mut body, options, &mut warnings)?;

    Ok(RequestBodyResult { body, warnings })
}

/// Parse OpenAI finish reason string into `FinishReason`.
#[must_use]
pub fn parse_finish_reason(s: &str) -> FinishReason {
    let unified = match s {
        "stop" => FinishReasonUnified::Stop,
        "length" => FinishReasonUnified::Length,
        "tool_calls" => FinishReasonUnified::ToolCalls,
        "content_filter" => FinishReasonUnified::ContentFilter,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(s.to_string()),
    }
}
