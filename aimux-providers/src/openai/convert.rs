//! Conversion between `LanguageModelPrompt` and OpenAI API format.

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::shared::SharedProviderOptions;
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
fn resolve_provider_reference(reference: &Value, provider: &str) -> Result<String, String> {
    if let Some(val) = reference.get(provider) {
        if let Some(s) = val.as_str() {
            return Ok(s.to_string());
        }
        return Ok(val.to_string());
    }
    let available: Vec<String> = reference
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    Err(format!(
        "No provider reference found for provider '{}'. Available providers: {}",
        provider,
        available.join(", ")
    ))
}

/// Convert a file part to the OpenAI format, handling images, audio, and PDF.
fn convert_file_part_to_openai(
    content_part: &ContentPart,
    data_b64: Option<&str>,
    url: Option<&str>,
    reference: Option<&Value>,
    filename: Option<&str>,
    provider_options: &Option<SharedProviderOptions>,
    part_index: usize,
) -> Result<Value, AiMuxError> {
    let media_type = match content_part {
        ContentPart::Image { media_type, .. }
        | ContentPart::File { media_type, .. }
        | ContentPart::FileBase64 { media_type, .. }
        | ContentPart::FileUrl { media_type, .. }
        | ContentPart::FileReference { media_type, .. } => media_type,
        _ => unreachable!("file conversion requires a file part"),
    };
    let prompt_cache_breakpoint = get_prompt_cache_breakpoint(provider_options);

    // Reference type
    if let Some(ref_val) = reference {
        let file_id =
            resolve_provider_reference(ref_val, "openai").map_err(AiMuxError::InvalidArgument)?;
        let mut part = json!({
            "type": "file",
            "file": { "file_id": file_id }
        });
        if let Some(bpt) = prompt_cache_breakpoint {
            part["prompt_cache_breakpoint"] = bpt;
        }
        return Ok(part);
    }

    let top_level = get_top_level_media_type(media_type);

    // Image
    if top_level == "image" {
        let image_url = if let Some(url_str) = url {
            json!({ "url": url_str })
        } else if let Some(b64) = data_b64 {
            let full_mt = resolve_full_media_type(content_part)?;
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
        let full_mt = resolve_full_media_type(content_part)?;
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
    let full_mt = resolve_full_media_type(content_part)?;

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
    msg: &LanguageModelPromptMessage,
    system_message_mode: SystemMessageMode,
) -> Result<Vec<Value>, AiMuxError> {
    let role = match msg.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };

    // System messages: respect systemMessageMode.
    if msg.role == Role::System {
        match system_message_mode {
            SystemMessageMode::Remove => return Ok(vec![]),
            SystemMessageMode::Developer => {
                return Ok(convert_system_message(msg, "developer"));
            }
            SystemMessageMode::System => {
                return Ok(convert_system_message(msg, "system"));
            }
        }
    }

    // Tool-role messages: each ToolResult part becomes its own OpenAI message.
    if msg.role == Role::Tool {
        let messages = msg
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::ToolResult {
                    tool_call_id,
                    result,
                    ..
                } => {
                    let content = tool_result_to_content(result);
                    Some(json!({
                        "role": "tool",
                        "content": content,
                        "tool_call_id": tool_call_id,
                    }))
                }
                _ => None,
            })
            .collect();
        return Ok(messages);
    }

    // Assistant messages with tool calls
    let has_tool_calls = msg
        .content
        .iter()
        .any(|p| matches!(p, ContentPart::ToolCall { .. }));

    if msg.role == Role::Assistant && has_tool_calls {
        let text: String = msg
            .content
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        // Reasoning / thinking content. Thinking-mode servers (and other
        // OpenAI-compatible reasoning models such as xAI) require prior
        // assistant `reasoning_content` to be replayed on subsequent turns,
        // including tool-call turns. Mirrors the Vercel AI SDK
        // `openai-compatible` assistant conversion, which emits
        // `reasoning_content` whenever a reasoning part is present.
        let reasoning: String = collect_reasoning(&msg.content);

        let tool_calls_json: Vec<Value> = msg
            .content
            .iter()
            .filter_map(|p| match p {
                ContentPart::ToolCall {
                    tool_call_id,
                    tool_name,
                    input,
                    ..
                } => {
                    let arguments = if input.is_null() {
                        "{}".to_string()
                    } else {
                        input.to_string()
                    };
                    Some(json!({
                        "type": "function",
                        "id": tool_call_id,
                        "function": {
                            "name": tool_name,
                            "arguments": arguments,
                        }
                    }))
                }
                _ => None,
            })
            .collect();

        let content = if text.is_empty() {
            Value::Null
        } else {
            Value::String(text)
        };

        let mut msg_obj = json!({
            "role": "assistant",
            "content": content,
            "tool_calls": tool_calls_json,
        });
        if !reasoning.is_empty() {
            msg_obj["reasoning_content"] = json!(reasoning);
        }
        return Ok(vec![msg_obj]);
    }

    // Default path (no tool calls). Assistant reasoning / thinking
    // parts are lifted to a top-level `reasoning_content` string (thinking-mode servers
    // thinking mode and other OpenAI-compatible reasoning models require it to
    // be replayed on later turns); they are never valid OpenAI content parts,
    // so they are excluded from the content shape below. Non-assistant roles do
    // not carry reasoning, but the filter is harmless.
    let reasoning = if msg.role == Role::Assistant {
        collect_reasoning(&msg.content)
    } else {
        String::new()
    };
    let has_reasoning = !reasoning.is_empty();

    // Consider only non-reasoning parts for the content shape. When they are
    // all plain text (without providerOptions), collapse to a string — matching
    // the Vercel AI SDK `openai-compatible` assistant conversion
    // (`content: toolCalls.length > 0 ? text || null : text`).
    //
    // Provider-executed tool results are dropped from assistant messages: the
    // OpenAI chat wire format has no content part for them, and upstream's
    // assistant branch handles only `text` and `tool-call`
    // (convert-to-openai-chat-messages.ts:246-300). Emitting one produces a
    // part type the API rejects.
    let content_parts: Vec<&ContentPart> = msg
        .content
        .iter()
        .filter(|p| match p {
            ContentPart::Reasoning { .. } => false,
            ContentPart::ToolResult { .. } => msg.role != Role::Assistant,
            _ => true,
        })
        .collect();
    let all_plain_text = content_parts.iter().all(|p| {
        matches!(
            p,
            ContentPart::Text {
                provider_options: None,
                ..
            }
        )
    });

    let mut msg_obj = if all_plain_text {
        let text: String = content_parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        json!({ "role": role, "content": text })
    } else {
        let parts: Vec<Value> = content_parts
            .iter()
            .enumerate()
            .map(|(i, part)| convert_part_to_openai(part, i))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|p| !p.is_null())
            .collect();
        json!({ "role": role, "content": parts })
    };
    if has_reasoning {
        msg_obj["reasoning_content"] = json!(reasoning);
    }
    Ok(vec![msg_obj])
}

/// Collect and concatenate the `text` of all `ContentPart::Reasoning` parts in
/// `content`, mirroring the Vercel AI SDK assistant-message conversion. Used to
/// build the OpenAI-compatible top-level `reasoning_content` / `reasoning`
/// field that thinking models (xAI and other servers) require to be replayed
/// across turns.
fn collect_reasoning(content: &[ContentPart]) -> String {
    content
        .iter()
        .filter_map(|p| match p {
            ContentPart::Reasoning { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Convert a system message, respecting promptCacheBreakpoint.
fn convert_system_message(msg: &LanguageModelPromptMessage, role: &str) -> Vec<Value> {
    // System messages in the Rust model are always a single text part.
    let text = msg
        .content
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text {
                text,
                provider_options,
            } => {
                let bpt = get_prompt_cache_breakpoint(provider_options);
                Some((text.clone(), bpt))
            }
            _ => None,
        })
        .next();

    match text {
        Some((t, None)) => vec![json!({ "role": role, "content": t })],
        Some((t, Some(bpt))) => vec![json!({
            "role": role,
            "content": [{
                "type": "text",
                "text": t,
                "prompt_cache_breakpoint": bpt,
            }]
        })],
        None => vec![json!({ "role": role, "content": "" })],
    }
}

/// Serialize a tool-result `output` value into the OpenAI tool message
/// `content` string.
fn tool_result_to_content(output: &Value) -> Value {
    match output {
        Value::String(s) => Value::String(s.clone()),
        other => Value::String(other.to_string()),
    }
}

/// Convert a content part to the OpenAI format.
fn convert_part_to_openai(part: &ContentPart, index: usize) -> Result<Value, AiMuxError> {
    match part {
        ContentPart::Text {
            text,
            provider_options,
        } => {
            let bpt = get_prompt_cache_breakpoint(provider_options);
            let mut p = json!({ "type": "text", "text": text });
            if let Some(b) = bpt {
                p["prompt_cache_breakpoint"] = b;
            }
            Ok(p)
        }
        ContentPart::Image {
            image,
            provider_options,
            ..
        } => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(image);
            convert_file_part_to_openai(part, Some(&b64), None, None, None, provider_options, index)
        }
        ContentPart::File {
            data,
            filename,
            provider_options,
            ..
        } => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(data);
            convert_file_part_to_openai(
                part,
                Some(&b64),
                None,
                None,
                filename.as_deref(),
                provider_options,
                index,
            )
        }
        ContentPart::FileBase64 {
            data,
            filename,
            provider_options,
            ..
        } => convert_file_part_to_openai(
            part,
            Some(data),
            None,
            None,
            filename.as_deref(),
            provider_options,
            index,
        ),
        ContentPart::FileUrl {
            url,
            provider_options,
            ..
        } => {
            convert_file_part_to_openai(part, None, Some(url), None, None, provider_options, index)
        }
        ContentPart::FileReference {
            reference,
            filename,
            provider_options,
            ..
        } => convert_file_part_to_openai(
            part,
            None,
            None,
            Some(reference),
            filename.as_deref(),
            provider_options,
            index,
        ),
        ContentPart::Reasoning { .. } => Ok(Value::Null),
        // These variants are handled by `convert_message_to_openai` for
        // assistant/tool roles; kept here as a defensive fallback.
        ContentPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => Ok(json!({
            "type": "tool_call",
            "id": tool_call_id,
            "function": {
                "name": tool_name,
                "arguments": input.to_string(),
            }
        })),
        // Tool results reach the wire as a separate `role: "tool"` message
        // (built by `convert_message_to_openai`), never as a content part —
        // there is no such part type in the OpenAI chat format.
        ContentPart::ToolResult { .. } => Ok(Value::Null),
    }
}

// ── Request body ────────────────────────────────────────────────────────────

/// Result of building a request body, including warnings.
#[derive(Debug, Clone)]
pub struct RequestBodyResult {
    pub body: Value,
    pub warnings: Vec<Warning>,
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
    let max_completion_tokens_opt = openai_option(provider_opts, "maxCompletionTokens")
        .and_then(|v| v.as_u64().map(|n| n as u32));

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
fn apply_response_format(body: &mut Value, options: &CallOptions) {
    let Some(ref rf) = options.response_format else {
        return;
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
                schema_obj["schema"] = schema.clone();
                schema_obj["name"] = json!(name.clone().unwrap_or_else(|| "response".to_string()));
                if let Some(d) = description {
                    schema_obj["description"] = json!(d);
                }
                schema_obj["strict"] = json!(true);
                body["response_format"] = json!({
                    "type": "json_schema",
                    "json_schema": schema_obj,
                });
            } else {
                body["response_format"] = json!({ "type": "json_object" });
            }
        }
    }
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
    // M3 (RFC-0016): logprobs request support. Previously `logprobs` /
    // `topLogprobs` were silently dropped by the provider_options whitelist —
    // the only option that "quietly did nothing". Pass-through as-is (OpenAI
    // expects `logprobs: bool` and `top_logprobs: int`).
    set("logprobs", "logprobs");
    set("topLogprobs", "top_logprobs");
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
            "priority" => {
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
fn apply_tools(body: &mut Value, options: &CallOptions) {
    let function_tools: Option<Vec<FunctionTool>> = options.tools.as_ref().map(|tools| {
        tools
            .iter()
            .filter_map(|t| match t {
                Tool::Function(ft) => Some(ft.clone()),
                Tool::Provider(_) => None,
            })
            .collect()
    });

    let prepared = prepare_tools(&function_tools, Some(&options.tool_choice));
    if let Some(tools) = prepared.tools {
        body["tools"] = json!(tools);
        if let Some(tc) = prepared.tool_choice {
            body["tool_choice"] = tc;
        }
    }
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
    let provider_opts = &options.provider_options;

    let resolved_reasoning_effort = resolve_reasoning_effort(provider_opts, &options.reasoning);
    let is_reasoning_model = resolve_is_reasoning_model(provider_opts, &caps);
    let system_message_mode = resolve_system_message_mode(provider_opts, is_reasoning_model, &caps);

    // OpenAI has no top_k.
    if options.top_k.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "topK".to_string(),
            details: None,
        });
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

    apply_response_format(&mut body, options);
    apply_provider_option_passthrough(&mut body, provider_opts);

    // Reasoning effort (v3 passthrough: no built-in vendor normalization).
    if let Some(ref effort) = resolved_reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }

    apply_service_tier(&mut body, provider_opts, &caps, &mut warnings);
    apply_tools(&mut body, options);

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
