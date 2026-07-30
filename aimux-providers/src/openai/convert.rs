//! Conversion between `LanguageModelPrompt` and OpenAI API format.

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::{FinishReason, FinishReasonUnified, ReasoningEffort, Warning};
use serde::Serialize;
use serde_json::{Value, json};

use super::OpenAICompatProfile;
use std::collections::HashMap;

// ── Model capabilities ──────────────────────────────────────────────────────

/// Parsed GPT version info (mirrors TS `getGptVersion`).
struct GptVersion {
    major: u32,
    minor: Option<u32>,
    variant: Option<String>,
}

/// Extract GPT version from a model ID (e.g. `gpt-5.1-codex` → major=5, minor=1).
fn get_gpt_version(model_id: &str) -> Option<GptVersion> {
    // ^gpt-(\d+)(?:\.(\d+))?(?:-(.+))?$
    let rest = model_id.strip_prefix("gpt-")?;
    let (major_str, remainder) = rest
        .find(|c: char| !c.is_ascii_digit())
        .map(|i| (&rest[..i], &rest[i..]))
        .unwrap_or((rest, ""));
    if major_str.is_empty() {
        return None;
    }
    let major: u32 = major_str.parse().ok()?;

    // Check for minor version (.N)
    let (minor, remainder) = if let Some(stripped) = remainder.strip_prefix('.') {
        let (minor_str, after) = stripped
            .find(|c: char| !c.is_ascii_digit())
            .map(|i| (&stripped[..i], &stripped[i..]))
            .unwrap_or((stripped, ""));
        if minor_str.is_empty() {
            return Some(GptVersion {
                major,
                minor: None,
                variant: if remainder.is_empty() {
                    None
                } else {
                    Some(remainder.trim_start_matches('-').to_string())
                },
            });
        }
        (minor_str.parse::<u32>().ok(), after)
    } else {
        (None, remainder)
    };

    let variant = if remainder.is_empty() {
        None
    } else {
        Some(remainder.trim_start_matches('-').to_string())
    };

    Some(GptVersion {
        major,
        minor,
        variant,
    })
}

/// Extract o-series version (e.g. `o4-mini` → 4).
fn get_o_series_version(model_id: &str) -> Option<u32> {
    let rest = model_id.strip_prefix('o')?;
    if rest.is_empty() || !rest.chars().next().unwrap().is_ascii_digit() {
        return None;
    }
    let (digits, _) = rest
        .find(|c: char| !c.is_ascii_digit())
        .map(|i| (&rest[..i], &rest[i..]))
        .unwrap_or((rest, ""));
    digits.parse().ok()
}

/// Model capabilities relevant to request body construction.
struct ModelCapabilities {
    is_reasoning_model: bool,
    system_message_mode: SystemMessageMode,
    supports_flex_processing: bool,
    supports_priority_processing: bool,
    supports_non_reasoning_parameters: bool,
}

/// How system messages are mapped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SystemMessageMode {
    System,
    Developer,
    Remove,
}

fn get_model_capabilities(model_id: &str) -> ModelCapabilities {
    let o_version = get_o_series_version(model_id);
    let gpt_version = get_gpt_version(model_id);
    let is_gpt_chat_model = gpt_version.as_ref().is_some_and(|v| {
        v.minor.is_none() && v.variant.as_deref().is_some_and(|s| s.starts_with("chat"))
    });
    let is_gpt_nano_model = gpt_version
        .as_ref()
        .is_some_and(|v| v.variant.as_deref().is_some_and(|s| s.starts_with("nano")));

    let supports_flex_processing = o_version.is_some_and(|v| v >= 3)
        || gpt_version
            .as_ref()
            .is_some_and(|v| v.major >= 5 && !is_gpt_chat_model);

    let supports_priority_processing = model_id.starts_with("gpt-4")
        || gpt_version
            .as_ref()
            .is_some_and(|v| v.major >= 5 && !is_gpt_nano_model && !is_gpt_chat_model)
        || o_version.is_some_and(|v| v >= 3);

    let is_reasoning_model = o_version.is_some()
        || gpt_version
            .as_ref()
            .is_some_and(|v| v.major >= 5 && !is_gpt_chat_model);

    let supports_non_reasoning_parameters = gpt_version
        .as_ref()
        .is_some_and(|v| v.major > 5 || (v.major == 5 && v.minor.unwrap_or(0) >= 1));

    let system_message_mode = if is_reasoning_model {
        SystemMessageMode::Developer
    } else {
        SystemMessageMode::System
    };

    ModelCapabilities {
        is_reasoning_model,
        system_message_mode,
        supports_flex_processing,
        supports_priority_processing,
        supports_non_reasoning_parameters,
    }
}

/// Check if a reasoning value is a custom (non-"provider-default") value.
fn is_custom_reasoning(reasoning: &Option<ReasoningEffort>) -> bool {
    match reasoning {
        Some(ReasoningEffort::ProviderDefault) => false,
        Some(_) => true,
        None => false,
    }
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

// ── Groq tool preparation ───────────────────────────────────────────────────

/// Models that support Groq's browser_search tool.
const GROQ_BROWSER_SEARCH_MODELS: &[&str] = &["openai/gpt-oss-20b", "openai/gpt-oss-120b"];

/// Prepare tools for Groq, supporting `groq.browser_search` provider tools
/// alongside standard function tools. Mirrors TS `prepareTools` in
/// `groq-prepare-tools.ts`.
fn prepare_tools_groq(
    function_tools: &Option<Vec<FunctionTool>>,
    all_tools: Option<&Vec<Tool>>,
    tool_choice: Option<&ToolChoice>,
    model_id: &str,
) -> PreparedTools {
    let non_empty_functions = function_tools.as_ref().filter(|&t| !t.is_empty());
    let has_any_tools = all_tools.as_ref().is_some_and(|t| !t.is_empty());

    if !has_any_tools {
        return PreparedTools {
            tools: None,
            tool_choice: None,
            tool_warnings: vec![],
        };
    }

    let mut groq_tools: Vec<Value> = Vec::new();
    let mut tool_warnings: Vec<ToolWarning> = Vec::new();

    // Function tools
    if let Some(tools) = non_empty_functions {
        for t in tools {
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
            groq_tools.push(json!({ "type": "function", "function": func }));
        }
    }

    // Provider-defined tools (browser_search)
    if let Some(tools) = all_tools {
        for t in tools {
            if let Tool::Provider(pt) = t {
                if pt.id == "groq.browser_search" {
                    if GROQ_BROWSER_SEARCH_MODELS.contains(&model_id) {
                        groq_tools.push(json!({ "type": "browser_search" }));
                    } else {
                        tool_warnings.push(ToolWarning {
                            warning_type: "unsupported".to_string(),
                            feature: format!("provider-defined tool {}", pt.id),
                            details: Some(format!(
                                "Browser search is only supported on the following models: {}. Current model: {}",
                                GROQ_BROWSER_SEARCH_MODELS.join(", "),
                                model_id
                            )),
                        });
                    }
                } else {
                    tool_warnings.push(ToolWarning {
                        warning_type: "unsupported".to_string(),
                        feature: format!("provider-defined tool {}", pt.id),
                        details: None,
                    });
                }
            }
        }
    }

    let tools_opt = if groq_tools.is_empty() {
        None
    } else {
        Some(groq_tools)
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
pub fn convert_prompt_to_openai_messages(prompt: &LanguageModelPrompt) -> Vec<Value> {
    convert_prompt_to_openai_messages_with_mode(prompt, SystemMessageMode::System)
}

/// Convert a `LanguageModelPrompt` to OpenAI `messages` array with a system
/// message mode.
pub fn convert_prompt_to_openai_messages_with_mode(
    prompt: &LanguageModelPrompt,
    system_message_mode: SystemMessageMode,
) -> Vec<Value> {
    convert_prompt_to_openai_messages_with_provider(prompt, system_message_mode, "openai")
}

/// Convert a `LanguageModelPrompt` to OpenAI `messages` array with a system
/// message mode and provider name (for provider-specific message conversion).
pub fn convert_prompt_to_openai_messages_with_provider(
    prompt: &LanguageModelPrompt,
    system_message_mode: SystemMessageMode,
    provider: &str,
) -> Vec<Value> {
    let mut result = Vec::new();
    for msg in prompt {
        for value in convert_message_to_openai(msg, system_message_mode, provider) {
            result.push(value);
        }
    }
    result
}

/// Get the prompt cache breakpoint from provider options.
fn get_prompt_cache_breakpoint(provider_options: &Option<Value>) -> Option<Value> {
    provider_options
        .as_ref()
        .and_then(|po| po.get("openai"))
        .and_then(|o| o.get("promptCacheBreakpoint"))
        .cloned()
}

/// Get imageDetail from provider options.
fn get_image_detail(provider_options: &Option<Value>) -> Option<Value> {
    provider_options
        .as_ref()
        .and_then(|po| po.get("openai"))
        .and_then(|o| o.get("imageDetail"))
        .cloned()
}

/// Get the top-level media type (e.g. "image" from "image/png").
fn get_top_level_media_type(media_type: &str) -> &str {
    media_type.split('/').next().unwrap_or("")
}

/// Resolve a full media type from a top-level-only or wildcard media type.
/// For "image" or "image/*", detects "image/png" from the base64 data.
/// For "application", it stays as-is (cannot be resolved without full data).
fn resolve_full_media_type(media_type: &str, b64_data: &str) -> String {
    let top_level = get_top_level_media_type(media_type);
    if top_level == "image" && media_type != "image" && !media_type.ends_with("/*") {
        return media_type.to_string();
    }
    if top_level == "image" {
        // Detect from base64 data
        if b64_data.starts_with("iVBORw0KGgo") {
            return "image/png".to_string();
        }
        if b64_data.starts_with("/9j/") {
            return "image/jpeg".to_string();
        }
        if b64_data.starts_with("R0lGOD") {
            return "image/gif".to_string();
        }
        if b64_data.starts_with("UklGR") {
            return "image/webp".to_string();
        }
        return "image/png".to_string(); // default
    }
    media_type.to_string()
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
    media_type: &str,
    data_b64: Option<&str>,
    url: Option<&str>,
    reference: Option<&Value>,
    filename: Option<&str>,
    provider_options: &Option<Value>,
    part_index: usize,
) -> Result<Value, String> {
    let prompt_cache_breakpoint = get_prompt_cache_breakpoint(provider_options);

    // Reference type
    if let Some(ref_val) = reference {
        let file_id = resolve_provider_reference(ref_val, "openai")?;
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
            let full_mt = resolve_full_media_type(media_type, b64);
            json!({ "url": format!("data:{};base64,{}", full_mt, b64) })
        } else {
            return Err("image part has no data or url".to_string());
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
            return Err("audio file parts with URLs".to_string());
        }
        let b64 = data_b64.ok_or("audio part has no data")?;
        let full_mt = resolve_full_media_type(media_type, b64);
        let format = match full_mt.as_str() {
            "audio/wav" => "wav",
            "audio/mp3" | "audio/mpeg" => "mp3",
            _ => return Err(format!("audio content parts with media type {}", full_mt)),
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
    let full_mt = if media_type == "application" {
        return Err("media type \"application\".*is not passed as inline bytes.*".to_string());
    } else {
        media_type.to_string()
    };

    if full_mt != "application/pdf" {
        return Err(format!("file part media type {}", full_mt));
    }

    if url.is_some() {
        return Err("PDF file parts with URLs".to_string());
    }

    let b64 = data_b64.ok_or("PDF part has no data")?;
    let fname = filename
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("part-{}.pdf", part_index));
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
    provider: &str,
) -> Vec<Value> {
    let role = match msg.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };

    // System messages: respect systemMessageMode.
    if msg.role == Role::System {
        match system_message_mode {
            SystemMessageMode::Remove => return vec![],
            SystemMessageMode::Developer => {
                return convert_system_message(msg, "developer");
            }
            SystemMessageMode::System => {
                return convert_system_message(msg, "system");
            }
        }
    }

    // Tool-role messages: each ToolResult part becomes its own OpenAI message.
    if msg.role == Role::Tool {
        return msg
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
    }

    // Assistant messages with tool calls
    let has_tool_calls = msg
        .content
        .iter()
        .any(|p| matches!(p, ContentPart::ToolCall { .. }));

    // Groq: assistant messages always collect text, reasoning, and tool_calls
    // together (mirrors TS `convertToGroqChatMessages`). The `reasoning` field
    // is added when non-empty, and `content` is "" (not null) when there is no
    // text.
    if msg.role == Role::Assistant && provider == "groq" {
        let text: String = msg
            .content
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        let reasoning: String = msg
            .content
            .iter()
            .filter_map(|p| match p {
                ContentPart::Reasoning { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

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

        let mut msg_obj = json!({
            "role": "assistant",
            "content": text,
        });
        if !reasoning.is_empty() {
            msg_obj["reasoning"] = json!(reasoning);
        }
        if !tool_calls_json.is_empty() {
            msg_obj["tool_calls"] = json!(tool_calls_json);
        }
        return vec![msg_obj];
    }

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

        return vec![json!({
            "role": "assistant",
            "content": content,
            "tool_calls": tool_calls_json,
        })];
    }

    // Default: when every part is text (without providerOptions), collapse to string.
    let all_plain_text = msg.content.iter().all(|p| {
        matches!(
            p,
            ContentPart::Text {
                provider_options: None,
                ..
            }
        )
    });

    if all_plain_text {
        let text: String = msg
            .content
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        vec![json!({ "role": role, "content": text })]
    } else {
        let parts: Vec<Value> = msg
            .content
            .iter()
            .enumerate()
            .map(|(i, part)| convert_part_to_openai(part, i))
            .collect();
        vec![json!({ "role": role, "content": parts })]
    }
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
fn convert_part_to_openai(part: &ContentPart, index: usize) -> Value {
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
            p
        }
        ContentPart::Image {
            image,
            media_type,
            provider_options,
        } => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(image);
            convert_file_part_to_openai(
                media_type,
                Some(&b64),
                None,
                None,
                None,
                provider_options,
                index,
            )
            .unwrap_or_else(|e| panic!("{}", e))
        }
        ContentPart::File {
            data,
            media_type,
            filename,
            provider_options,
        } => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(data);
            convert_file_part_to_openai(
                media_type,
                Some(&b64),
                None,
                None,
                filename.as_deref(),
                provider_options,
                index,
            )
            .unwrap_or_else(|e| panic!("{}", e))
        }
        ContentPart::FileBase64 {
            data,
            media_type,
            filename,
            provider_options,
        } => convert_file_part_to_openai(
            media_type,
            Some(data),
            None,
            None,
            filename.as_deref(),
            provider_options,
            index,
        )
        .unwrap_or_else(|e| panic!("{}", e)),
        ContentPart::FileUrl {
            url,
            media_type,
            provider_options,
        } => convert_file_part_to_openai(
            media_type,
            None,
            Some(url),
            None,
            None,
            provider_options,
            index,
        )
        .unwrap_or_else(|e| panic!("{}", e)),
        ContentPart::FileReference {
            media_type,
            reference,
            filename,
            provider_options,
        } => convert_file_part_to_openai(
            media_type,
            None,
            None,
            Some(reference),
            filename.as_deref(),
            provider_options,
            index,
        )
        .unwrap_or_else(|e| panic!("{}", e)),
        ContentPart::Reasoning { .. } => Value::Null,
        // These variants are handled by `convert_message_to_openai` for
        // assistant/tool roles; kept here as a defensive fallback.
        ContentPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => {
            json!({
                "type": "tool_call",
                "id": tool_call_id,
                "function": {
                    "name": tool_name,
                    "arguments": input.to_string(),
                }
            })
        }
        ContentPart::ToolResult {
            tool_call_id,
            result,
            ..
        } => {
            json!({
                "type": "tool_result",
                "tool_call_id": tool_call_id,
                "content": result,
            })
        }
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
fn openai_option(
    options: &Option<HashMap<std::string::String, Value>>,
    key: &str,
) -> Option<Value> {
    use std::collections::HashMap;
    options
        .as_ref()
        .and_then(|m: &HashMap<String, Value>| m.get("openai"))
        .and_then(|o| o.get(key))
        .cloned()
}

/// Convert `CallOptions` to an OpenAI request body (without warnings).
pub fn build_request_body(model_id: &str, options: &CallOptions, stream: bool) -> Value {
    build_request_body_with_warnings(
        model_id,
        options,
        stream,
        "openai",
        &OpenAICompatProfile::full(),
    )
    .body
}

/// Convert `CallOptions` to an OpenAI request body, returning warnings.
/// `provider` controls provider-specific behaviour (e.g. groq reads provider
/// options from the `"groq"` key and applies a reasoning-effort map).
/// `profile` declares provider capability differences (top_k, tools, etc.).
pub fn build_request_body_with_warnings(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
    provider: &str,
    profile: &OpenAICompatProfile,
) -> RequestBodyResult {
    let mut warnings: Vec<Warning> = Vec::new();
    let caps = get_model_capabilities(model_id);

    // Parse provider options — groq reads from the "groq" key (with "openai"
    // fallback), other providers read from "openai".
    let provider_opts = &options.provider_options;

    // Helper: look up a key from the provider-specific options (groq → "groq"
    // then "openai"; otherwise "openai").
    let popt = |key: &str| -> Option<Value> {
        if provider == "groq"
            && let Some(v) = provider_opts
                .as_ref()
                .and_then(|m| m.get("groq"))
                .and_then(|o| o.get(key))
                .cloned()
        {
            return Some(v);
        }
        openai_option(provider_opts, key)
    };

    // Resolve reasoning effort.
    // Groq maps minimal→low, xhigh→high, and skips 'none' (no reasoning_effort
    // sent). Other providers use the raw value.
    let resolved_reasoning_effort: Option<String> = popt("reasoningEffort")
        .map(|v| v.as_str().map(|s| s.to_string()).unwrap_or(v.to_string()))
        .or_else(|| {
            if is_custom_reasoning(&options.reasoning) {
                if provider == "groq" {
                    match options.reasoning? {
                        ReasoningEffort::None => None,
                        ReasoningEffort::Minimal => Some("low".to_string()),
                        ReasoningEffort::Xhigh => Some("high".to_string()),
                        other => Some(other.to_string()),
                    }
                } else {
                    options.reasoning.map(|r| r.to_string())
                }
            } else {
                None
            }
        });

    let is_reasoning_model = popt("forceReasoning")
        .map(|v| v.as_bool().unwrap_or(false))
        .unwrap_or(caps.is_reasoning_model);

    // Determine system message mode
    let system_message_mode = popt("systemMessageMode")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .map(|s| match s.as_str() {
            "developer" => SystemMessageMode::Developer,
            "remove" => SystemMessageMode::Remove,
            _ => SystemMessageMode::System,
        })
        .unwrap_or(if is_reasoning_model {
            SystemMessageMode::Developer
        } else {
            caps.system_message_mode
        });

    // top_k: only send when the provider supports it. The actual body
    // insertion happens after `body` is created below.
    let top_k_value = options.top_k;
    let top_k_supported = profile.supports_top_k;
    if top_k_value.is_some() && !top_k_supported {
        warnings.push(Warning::Unsupported {
            feature: "topK".to_string(),
            details: None,
        });
    }

    let messages = convert_prompt_to_openai_messages_with_provider(
        &options.prompt,
        system_message_mode,
        provider,
    );

    let mut body = json!({
        "model": model_id,
        "messages": messages,
    });

    if stream {
        body["stream"] = json!(true);
        // Groq does not send stream_options; other providers include usage.
        if provider != "groq" {
            body["stream_options"] = json!({ "include_usage": true });
        }
    }

    // top_k: only send when the provider supports it.
    if let Some(tk) = top_k_value
        && top_k_supported
    {
        body["top_k"] = json!(tk);
    }

    // Max tokens / max_completion_tokens
    let max_completion_tokens_opt =
        popt("maxCompletionTokens").and_then(|v| v.as_u64().map(|n| n as u32));

    if is_reasoning_model {
        if let Some(max_tokens) = options.max_output_tokens {
            match max_completion_tokens_opt {
                None => body["max_completion_tokens"] = json!(max_tokens),
                Some(mct) => body["max_completion_tokens"] = json!(mct),
            }
        } else if let Some(mct) = max_completion_tokens_opt {
            body["max_completion_tokens"] = json!(mct);
        }
    } else {
        if let Some(max_tokens) = options.max_output_tokens {
            body["max_tokens"] = json!(max_tokens);
        }
        if let Some(mct) = max_completion_tokens_opt {
            body["max_completion_tokens"] = json!(mct);
        }
    }

    // Temperature, top_p, frequency_penalty, presence_penalty
    let mut temperature = options.temperature;
    let mut top_p = options.top_p;
    let mut frequency_penalty = options.frequency_penalty;
    let mut presence_penalty = options.presence_penalty;

    // Remove unsupported settings for reasoning models
    if is_reasoning_model {
        let allow_non_reasoning = resolved_reasoning_effort.as_deref() == Some("none")
            && caps.supports_non_reasoning_parameters;

        if !allow_non_reasoning {
            if temperature.is_some() {
                temperature = None;
                warnings.push(Warning::Unsupported {
                    feature: "temperature".to_string(),
                    details: Some("temperature is not supported for reasoning models".to_string()),
                });
            }
            if top_p.is_some() {
                top_p = None;
                warnings.push(Warning::Unsupported {
                    feature: "topP".to_string(),
                    details: Some("topP is not supported for reasoning models".to_string()),
                });
            }
        }

        if frequency_penalty.is_some() {
            frequency_penalty = None;
            warnings.push(Warning::Unsupported {
                feature: "frequencyPenalty".to_string(),
                details: Some("frequencyPenalty is not supported for reasoning models".to_string()),
            });
        }
        if presence_penalty.is_some() {
            presence_penalty = None;
            warnings.push(Warning::Unsupported {
                feature: "presencePenalty".to_string(),
                details: Some("presencePenalty is not supported for reasoning models".to_string()),
            });
        }
    } else if (model_id.starts_with("gpt-4o-search-preview")
        || model_id.starts_with("gpt-4o-mini-search-preview"))
        && temperature.is_some()
    {
        temperature = None;
        warnings.push(Warning::Unsupported {
            feature: "temperature".to_string(),
            details: Some(
                "temperature is not supported for the search preview models and has been removed."
                    .to_string(),
            ),
        });
    }

    if let Some(temp) = temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(tp) = top_p {
        body["top_p"] = json!(tp);
    }
    if let Some(fp) = frequency_penalty {
        body["frequency_penalty"] = json!(fp);
    }
    if let Some(pp) = presence_penalty {
        body["presence_penalty"] = json!(pp);
    }
    if let Some(ref stop) = options.stop_sequences {
        body["stop"] = json!(stop);
    }
    if let Some(seed) = options.seed {
        body["seed"] = json!(seed);
    }

    // Response format
    if let Some(ref rf) = options.response_format {
        match rf {
            ResponseFormat::Text => {}
            ResponseFormat::Json {
                schema,
                name,
                description,
            } => {
                // Groq: structuredOutputs defaults to true, strictJsonSchema
                // defaults to true. When structuredOutputs is false and a schema
                // is provided, emit a warning and use json_object.
                let (structured_outputs, strict_json_schema) = if provider == "groq" {
                    (
                        popt("structuredOutputs")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true),
                        popt("strictJsonSchema")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true),
                    )
                } else {
                    (true, true)
                };

                if schema.is_some() && structured_outputs {
                    let mut schema_obj = json!({});
                    if let Some(s) = schema {
                        schema_obj["schema"] = s.clone();
                    }
                    schema_obj["name"] =
                        json!(name.clone().unwrap_or_else(|| "response".to_string()));
                    if let Some(d) = description {
                        schema_obj["description"] = json!(d);
                    }
                    schema_obj["strict"] = json!(strict_json_schema);
                    body["response_format"] = json!({
                        "type": "json_schema",
                        "json_schema": schema_obj,
                    });
                } else if schema.is_some() && !structured_outputs {
                    // Schema provided but structuredOutputs disabled → json_object + warning
                    body["response_format"] = json!({ "type": "json_object" });
                    warnings.push(Warning::Unsupported {
                        feature: "responseFormat".to_string(),
                        details: Some(
                            "JSON response format schema is only supported with structuredOutputs"
                                .to_string(),
                        ),
                    });
                } else {
                    body["response_format"] = json!({ "type": "json_object" });
                }
            }
        }
    }

    // Provider-specific options
    if let Some(val) = popt("logitBias") {
        body["logit_bias"] = val;
    }
    if let Some(val) = popt("user") {
        body["user"] = val;
    }
    if let Some(val) = popt("parallelToolCalls") {
        body["parallel_tool_calls"] = val;
    }
    if let Some(val) = popt("textVerbosity") {
        body["verbosity"] = val;
    }
    if let Some(val) = popt("store") {
        body["store"] = val;
    }
    if let Some(val) = popt("metadata") {
        body["metadata"] = val;
    }
    if let Some(val) = popt("prediction") {
        body["prediction"] = val;
    }
    if let Some(val) = popt("promptCacheKey") {
        body["prompt_cache_key"] = val;
    }
    if let Some(val) = popt("promptCacheRetention") {
        body["prompt_cache_retention"] = val;
    }
    if let Some(val) = popt("promptCacheOptions") {
        body["prompt_cache_options"] = val;
    }
    if let Some(val) = popt("safetyIdentifier") {
        body["safety_identifier"] = val;
    }

    // Groq: reasoning_format provider option
    if provider == "groq"
        && let Some(val) = popt("reasoningFormat")
    {
        body["reasoning_format"] = val;
    }

    // Reasoning effort
    if let Some(ref effort) = resolved_reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }

    // Service tier
    if provider == "groq" {
        // Groq passes service_tier through without model-capability validation.
        if let Some(val) = popt("serviceTier") {
            body["service_tier"] = val;
        }
    } else {
        let service_tier = popt("serviceTier").and_then(|v| v.as_str().map(|s| s.to_string()));
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

    // Tools
    let function_tools: Option<Vec<FunctionTool>> = options.tools.as_ref().map(|tools| {
        tools
            .iter()
            .filter_map(|t| match t {
                Tool::Function(ft) => Some(ft.clone()),
                Tool::Provider(_) => None,
            })
            .collect()
    });

    // Groq: handle provider-defined tools (browser_search) alongside function tools.
    let prepared = if provider == "groq" {
        prepare_tools_groq(
            &function_tools,
            options.tools.as_ref(),
            Some(&options.tool_choice),
            model_id,
        )
    } else {
        prepare_tools(&function_tools, Some(&options.tool_choice))
    };
    if let Some(tools) = prepared.tools {
        body["tools"] = json!(tools);
        if let Some(tc) = prepared.tool_choice {
            body["tool_choice"] = tc;
        }
    }

    // Convert tool warnings to Warning type
    for tw in prepared.tool_warnings {
        warnings.push(Warning::Unsupported {
            feature: tw.feature,
            details: tw.details,
        });
    }

    // 请求体后处理：某些厂商需要追加额外字段。
    if let Some(ref override_kind) = profile.request_body_override {
        match override_kind {
            super::RequestBodyOverride::DeepSeek => {
                apply_deepseek_override(&mut body, &mut warnings, options);
            }
        }
    }

    RequestBodyResult { body, warnings }
}

/// DeepSeek 请求体后处理：追加 `thinking` 字段，重映射 `reasoning_effort`。
fn apply_deepseek_override(body: &mut Value, warnings: &mut Vec<Warning>, options: &CallOptions) {
    use aimux_core::types::ReasoningEffort;

    let deepseek_opts = options
        .provider_options
        .as_ref()
        .and_then(|m| m.get("deepseek"));

    // thinking 字段
    if let Some(thinking) = deepseek_opts.and_then(|o| o.get("thinking")).cloned() {
        body["thinking"] = thinking;
    } else if let Some(reasoning) = options.reasoning
        && reasoning != ReasoningEffort::ProviderDefault
    {
        if reasoning == ReasoningEffort::None {
            body["thinking"] = json!({ "type": "disabled" });
        } else {
            body["thinking"] = json!({ "type": "enabled" });
        }
    }

    // reasoning_effort 重映射
    if body.get("reasoning_effort").is_some() {
        body.as_object_mut()
            .expect("request body is a JSON object")
            .remove("reasoning_effort");
    }

    if let Some(effort) = deepseek_opts
        .and_then(|o| o.get("reasoningEffort"))
        .cloned()
    {
        body["reasoning_effort"] = effort;
    } else if let Some(reasoning) = options.reasoning {
        match reasoning {
            ReasoningEffort::None | ReasoningEffort::ProviderDefault => {}
            ReasoningEffort::High => {
                body["reasoning_effort"] = json!("high");
            }
            ReasoningEffort::Low => {
                body["reasoning_effort"] = json!("low");
            }
            ReasoningEffort::Medium => {
                body["reasoning_effort"] = json!("medium");
            }
            ReasoningEffort::Xhigh => {
                body["reasoning_effort"] = json!("max");
                warnings.push(Warning::Compatibility {
                    feature: "reasoning".to_string(),
                    details: Some(
                        "reasoning \"xhigh\" is not directly supported by this model. mapped to effort \"max\"."
                            .to_string(),
                    ),
                });
            }
            ReasoningEffort::Minimal => {
                body["reasoning_effort"] = json!("low");
                warnings.push(Warning::Compatibility {
                    feature: "reasoning".to_string(),
                    details: Some(
                        "reasoning \"minimal\" is not directly supported by this model. mapped to effort \"low\"."
                            .to_string(),
                    ),
                });
            }
        }
    }
}

/// Parse OpenAI finish reason string into `FinishReason`.
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
