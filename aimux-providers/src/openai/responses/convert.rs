//! Conversion between `LanguageModelPrompt` and the OpenAI Responses API
//! `input` format, plus request-body construction, tool preparation, usage
//! conversion, and finish-reason mapping.
//!
//! Mirrors the TS sources:
//! - `convert-to-openai-responses-input.ts` -> [`convert_to_responses_input`]
//! - `openai-responses-language-model.ts` `getArgs` ->
//!   [`build_responses_request_body`]
//! - `openai-responses-prepare-tools.ts` -> [`prepare_responses_tools`]
//! - `convert-openai-responses-usage.ts` -> [`convert_responses_usage`]
//! - `map-openai-responses-finish-reason.ts` -> [`map_responses_finish_reason`]

use std::collections::HashMap;

use serde_json::{Value, json};

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::{FinishReason, FinishReasonUnified, ReasoningEffort, Usage, Warning};

use super::types::ResponsesUsage;

// -- Model capabilities ------------------------------------------------------

/// Parsed GPT version info (mirrors TS `getGptVersion`).
struct GptVersion {
    major: u32,
    minor: Option<u32>,
    variant: Option<String>,
}

/// Extract GPT version from a model ID (e.g. `gpt-5.1-codex` -> major=5, minor=1).
fn get_gpt_version(model_id: &str) -> Option<GptVersion> {
    let rest = model_id.strip_prefix("gpt-")?;
    let (major_str, remainder) = rest
        .find(|c: char| !c.is_ascii_digit())
        .map(|i| (&rest[..i], &rest[i..]))
        .unwrap_or((rest, ""));
    if major_str.is_empty() {
        return None;
    }
    let major: u32 = major_str.parse().ok()?;

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
        (
            minor_str.parse::<u32>().ok(),
            if after.is_empty() {
                None
            } else {
                Some(after.trim_start_matches('-').to_string())
            },
        )
    } else {
        (
            None,
            if remainder.is_empty() {
                None
            } else {
                Some(remainder.trim_start_matches('-').to_string())
            },
        )
    };

    Some(GptVersion {
        major,
        minor,
        variant: remainder,
    })
}

/// Extract o-series version (e.g. `o3-mini` -> 3). Mirrors TS `getOSeriesVersion`.
fn get_o_series_version(model_id: &str) -> Option<u32> {
    let rest = model_id.strip_prefix('o')?;
    let (digits, _) = rest
        .find(|c: char| !c.is_ascii_digit())
        .map(|i| (&rest[..i], &rest[i..]))
        .unwrap_or((rest, ""));
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u32>().ok()
}

/// Model capabilities relevant to Responses request body construction.
///
/// Mirrors TS `getOpenAILanguageModelCapabilities`.
struct ResponsesModelCapabilities {
    is_reasoning_model: bool,
    system_message_mode: ResponsesSystemMessageMode,
    supports_flex_processing: bool,
    supports_priority_processing: bool,
    supports_non_reasoning_parameters: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResponsesSystemMessageMode {
    System,
    Developer,
    Remove,
}

fn get_model_capabilities(model_id: &str) -> ResponsesModelCapabilities {
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
        ResponsesSystemMessageMode::Developer
    } else {
        ResponsesSystemMessageMode::System
    };

    ResponsesModelCapabilities {
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

// -- Provider options helper -------------------------------------------------

/// Get a value from `provider_options.openai.<key>`.
fn openai_option(options: &Option<HashMap<String, Value>>, key: &str) -> Option<Value> {
    options
        .as_ref()
        .and_then(|m| m.get("openai"))
        .and_then(|o| o.get(key))
        .cloned()
}

// -- Input conversion --------------------------------------------------------

/// Result of converting a prompt into the Responses API `input` array.
pub struct ResponsesInputResult {
    pub input: Vec<Value>,
    pub warnings: Vec<Warning>,
}

/// Convert a `LanguageModelPrompt` into the OpenAI Responses API `input` array.
///
/// Mirrors the core paths of TS `convertToOpenAIResponsesInput`:
/// - system -> `{ role: "system"|"developer", content: <text> }`
/// - user -> `{ role: "user", content: [{ type: "input_text", text }] }`
/// - assistant text -> `{ role: "assistant", content: [{ type: "output_text", text }] }`
///   (or `{ type: "item_reference", id }` when `store` and an itemId are present)
/// - assistant tool-call -> `{ type: "function_call", call_id, name, arguments }`
/// - tool result -> `{ type: "function_call_output", call_id, output }`
///
/// `store` defaults to `true` (matching the API default). When
/// `has_previous_response_id` is true, assistant reasoning/function-call items
/// that already carry an `itemId` are skipped (they live in the previous
/// response chain).
pub fn convert_to_responses_input(
    prompt: &LanguageModelPrompt,
    system_message_mode: ResponsesSystemMessageMode,
    store: bool,
    has_previous_response_id: bool,
) -> ResponsesInputResult {
    let mut input: Vec<Value> = Vec::new();
    let mut warnings: Vec<Warning> = Vec::new();

    for msg in prompt {
        match msg.role {
            Role::System => match system_message_mode {
                ResponsesSystemMessageMode::System => {
                    input.push(json!({
                        "role": "system",
                        "content": join_text_parts(&msg.content),
                    }));
                }
                ResponsesSystemMessageMode::Developer => {
                    input.push(json!({
                        "role": "developer",
                        "content": join_text_parts(&msg.content),
                    }));
                }
                ResponsesSystemMessageMode::Remove => {
                    warnings.push(Warning::Other {
                        message: "system messages are removed for this model".to_string(),
                    });
                }
            },
            Role::User => {
                let content: Vec<Value> = msg.content.iter().map(convert_user_part).collect();
                input.push(json!({ "role": "user", "content": content }));
            }
            Role::Assistant => {
                for part in &msg.content {
                    match part {
                        ContentPart::Text {
                            text,
                            provider_options,
                        } => {
                            let id = item_id(provider_options);
                            if has_previous_response_id && id.is_some() {
                                continue;
                            }
                            if store && let Some(ref id) = id {
                                input.push(json!({ "type": "item_reference", "id": id }));
                                continue;
                            }
                            let phase = phase_from_provider_options(provider_options);
                            let mut item = json!({
                                "role": "assistant",
                                "content": [{ "type": "output_text", "text": text }],
                            });
                            if let Some(ref id) = id {
                                item["id"] = json!(id);
                            }
                            if let Some(phase) = phase {
                                item["phase"] = json!(phase);
                            }
                            input.push(item);
                        }
                        ContentPart::ToolCall {
                            tool_call_id,
                            tool_name,
                            input: tool_input,
                            provider_options,
                            ..
                        } => {
                            let id = item_id(provider_options);
                            if has_previous_response_id && id.is_some() {
                                continue;
                            }
                            let namespace = namespace_from_provider_options(provider_options);
                            let mut item = json!({
                                "type": "function_call",
                                "call_id": tool_call_id,
                                "name": tool_name,
                                "arguments": serialize_arguments(tool_input),
                            });
                            if let Some(ref ns) = namespace {
                                item["namespace"] = json!(ns);
                            }
                            input.push(item);
                        }
                        ContentPart::Reasoning {
                            text,
                            provider_options,
                            ..
                        } => {
                            let reasoning_id = openai_sub_option(provider_options, "itemId");
                            if has_previous_response_id && reasoning_id.is_some() {
                                continue;
                            }
                            if let Some(ref rid) = reasoning_id {
                                if store {
                                    input.push(json!({ "type": "item_reference", "id": rid }));
                                } else {
                                    let encrypted = openai_sub_option(
                                        provider_options,
                                        "reasoningEncryptedContent",
                                    );
                                    let mut summary: Vec<Value> = Vec::new();
                                    if !text.is_empty() {
                                        summary
                                            .push(json!({ "type": "summary_text", "text": text }));
                                    }
                                    let mut item = json!({
                                        "type": "reasoning",
                                        "id": rid,
                                        "summary": summary,
                                    });
                                    if let Some(enc) = encrypted {
                                        item["encrypted_content"] = json!(enc);
                                    }
                                    input.push(item);
                                }
                            } else {
                                let encrypted = openai_sub_option(
                                    provider_options,
                                    "reasoningEncryptedContent",
                                );
                                if let Some(enc) = encrypted {
                                    let mut summary: Vec<Value> = Vec::new();
                                    if !text.is_empty() {
                                        summary
                                            .push(json!({ "type": "summary_text", "text": text }));
                                    }
                                    input.push(json!({
                                        "type": "reasoning",
                                        "encrypted_content": enc,
                                        "summary": summary,
                                    }));
                                } else {
                                    warnings.push(Warning::Other {
                                        message: "Non-OpenAI reasoning parts are not supported. Skipping reasoning part.".to_string(),
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Role::Tool => {
                for part in &msg.content {
                    if let ContentPart::ToolResult {
                        tool_call_id,
                        result,
                        ..
                    } = part
                    {
                        let content_value = match result {
                            Value::String(s) => Value::String(s.clone()),
                            other => Value::String(other.to_string()),
                        };
                        input.push(json!({
                            "type": "function_call_output",
                            "call_id": tool_call_id,
                            "output": content_value,
                        }));
                    }
                }
            }
        }
    }

    // When store is false, remove reasoning parts without encrypted content.
    if !store
        && input.iter().any(|item| {
            item.get("type").and_then(|v| v.as_str()) == Some("reasoning")
                && item.get("encrypted_content").is_none()
        })
    {
        warnings.push(Warning::Other {
            message:
                "Reasoning parts without encrypted content are not supported when store is false. Skipping reasoning parts."
                    .to_string(),
        });
        input.retain(|item| {
            item.get("type").and_then(|v| v.as_str()) != Some("reasoning")
                || item.get("encrypted_content").is_some()
        });
    }

    ResponsesInputResult { input, warnings }
}

/// Join all text parts of a message into a single string (for system messages).
fn join_text_parts(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Convert a single user-message content part into the Responses input shape.
fn convert_user_part(part: &ContentPart) -> Value {
    match part {
        ContentPart::Text { text, .. } => json!({ "type": "input_text", "text": text }),
        ContentPart::Image {
            image,
            media_type,
            provider_options,
        } => {
            let b64 = {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(image)
            };
            let detail = openai_sub_option(provider_options, "imageDetail");
            let mut img = json!({
                "type": "input_image",
                "image_url": format!("data:{};base64,{}", media_type, b64),
            });
            if let Some(d) = detail {
                img["detail"] = d;
            }
            img
        }
        ContentPart::File {
            data,
            media_type,
            filename,
            ..
        } => {
            let b64 = {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(data)
            };
            let fname = filename.clone().unwrap_or_else(|| "part.pdf".to_string());
            json!({
                "type": "input_file",
                "filename": fname,
                "file_data": format!("data:{};base64,{}", media_type, b64),
            })
        }
        ContentPart::FileBase64 {
            data,
            media_type,
            filename,
            ..
        } => {
            let fname = filename.clone().unwrap_or_else(|| "part.pdf".to_string());
            json!({
                "type": "input_file",
                "filename": fname,
                "file_data": format!("data:{};base64,{}", media_type, data),
            })
        }
        ContentPart::FileUrl {
            url, media_type, ..
        } => {
            if media_type.starts_with("image") {
                json!({ "type": "input_image", "image_url": url })
            } else {
                json!({ "type": "input_file", "file_url": url })
            }
        }
        _ => json!({ "type": "input_text", "text": "" }),
    }
}

/// Serialize tool-call arguments: null/empty -> `"{}"`, objects -> JSON string.
fn serialize_arguments(input: &Value) -> String {
    match input {
        Value::Null => "{}".to_string(),
        Value::String(s) if s.is_empty() => "{}".to_string(),
        other => other.to_string(),
    }
}

/// Read the `itemId` from a content part's `providerOptions.openai.itemId`.
fn item_id(provider_options: &Option<Value>) -> Option<String> {
    provider_options
        .as_ref()
        .and_then(|v| v.get("openai"))
        .and_then(|o| o.get("itemId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Read the `phase` from a content part's `providerOptions.openai.phase`.
fn phase_from_provider_options(provider_options: &Option<Value>) -> Option<String> {
    provider_options
        .as_ref()
        .and_then(|v| v.get("openai"))
        .and_then(|o| o.get("phase"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Read the `namespace` from a content part's `providerOptions.openai.namespace`.
fn namespace_from_provider_options(provider_options: &Option<Value>) -> Option<String> {
    provider_options
        .as_ref()
        .and_then(|v| v.get("openai"))
        .and_then(|o| o.get("namespace"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Read a sub-key from `providerOptions.openai.<key>` on a content part.
fn openai_sub_option(provider_options: &Option<Value>, key: &str) -> Option<Value> {
    provider_options
        .as_ref()
        .and_then(|v| v.get("openai"))
        .and_then(|o| o.get(key))
        .cloned()
}

// -- Tool preparation --------------------------------------------------------

/// The result of preparing tools for a Responses request body.
#[derive(Debug, Clone)]
pub struct PreparedResponsesTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub tool_warnings: Vec<Warning>,
}

/// Prepare `FunctionTool`s into the Responses `tools` / `tool_choice` JSON shape.
///
/// Mirrors the function-tool path of TS `prepareResponsesTools`. Built-in
/// provider tools (web_search, file_search, etc.) are out of scope for the
/// core implementation and emit an `unsupported` warning.
pub fn prepare_responses_tools(
    tools: &Option<Vec<Tool>>,
    tool_choice: Option<&ToolChoice>,
) -> PreparedResponsesTools {
    let non_empty = tools.as_ref().filter(|t| !t.is_empty());
    let mut tool_warnings: Vec<Warning> = Vec::new();

    let tools_opt = match non_empty {
        None => None,
        Some(tools) => {
            let mut openai_tools: Vec<Value> = Vec::new();
            for t in tools {
                match t {
                    Tool::Function(ft) => {
                        openai_tools.push(function_tool_to_json(ft));
                    }
                    Tool::Provider(pt) => {
                        tool_warnings.push(Warning::Unsupported {
                            feature: format!("provider-defined tool {}", pt.id),
                            details: None,
                        });
                    }
                }
            }
            if openai_tools.is_empty() {
                None
            } else {
                Some(openai_tools)
            }
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
                Some(json!({ "type": "function", "name": tool_name }))
            }
        },
    };

    PreparedResponsesTools {
        tools: tools_opt,
        tool_choice: tool_choice_opt,
        tool_warnings,
    }
}

/// Convert a `FunctionTool` into the Responses `function` tool JSON shape.
fn function_tool_to_json(t: &FunctionTool) -> Value {
    let mut func = json!({
        "type": "function",
        "name": t.name,
        "parameters": t.input_schema,
    });
    if let Some(ref desc) = t.description {
        func["description"] = json!(desc);
    }
    if let Some(strict) = t.strict {
        func["strict"] = json!(strict);
    }
    func
}

// -- Request body ------------------------------------------------------------

/// Result of building a Responses request body.
pub struct ResponsesRequestBodyResult {
    pub body: Value,
    pub warnings: Vec<Warning>,
}

/// Build the Responses API request body from `CallOptions`.
///
/// Mirrors TS `getArgs`. `stream` adds `"stream": true` to the body.
pub fn build_responses_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> ResponsesRequestBodyResult {
    let mut warnings: Vec<Warning> = Vec::new();
    let caps = get_model_capabilities(model_id);
    let provider_opts = &options.provider_options;

    // -- Warnings for unsupported call options --
    if options.top_k.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "topK".to_string(),
            details: None,
        });
    }
    if options.seed.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "seed".to_string(),
            details: None,
        });
    }
    if options.presence_penalty.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "presencePenalty".to_string(),
            details: None,
        });
    }
    if options.frequency_penalty.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "frequencyPenalty".to_string(),
            details: None,
        });
    }
    if options.stop_sequences.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "stopSequences".to_string(),
            details: None,
        });
    }

    // -- Reasoning resolution --
    let resolved_reasoning_effort: Option<String> = openai_option(provider_opts, "reasoningEffort")
        .map(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| v.to_string())
        })
        .or_else(|| {
            if is_custom_reasoning(&options.reasoning) {
                options.reasoning.map(|r| r.to_string())
            } else {
                None
            }
        });

    let resolved_reasoning_summary: Option<String> =
        openai_option(provider_opts, "reasoningSummary")
            .map(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| v.to_string())
            })
            .or_else(|| {
                if resolved_reasoning_effort
                    .as_deref()
                    .is_some_and(|e| e != "none")
                {
                    Some("detailed".to_string())
                } else {
                    None
                }
            });

    let is_reasoning_model = openai_option(provider_opts, "forceReasoning")
        .map(|v| v.as_bool().unwrap_or(false))
        .unwrap_or(caps.is_reasoning_model);

    // -- conversation + previousResponseId conflict --
    let has_conversation = openai_option(provider_opts, "conversation").is_some();
    let has_previous_response_id = openai_option(provider_opts, "previousResponseId").is_some();
    if has_conversation && has_previous_response_id {
        warnings.push(Warning::Unsupported {
            feature: "conversation".to_string(),
            details: Some(
                "conversation and previousResponseId cannot be used together".to_string(),
            ),
        });
    }

    // -- System message mode --
    let system_message_mode = openai_option(provider_opts, "systemMessageMode")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .map(|s| match s.as_str() {
            "developer" => ResponsesSystemMessageMode::Developer,
            "remove" => ResponsesSystemMessageMode::Remove,
            _ => ResponsesSystemMessageMode::System,
        })
        .unwrap_or(if is_reasoning_model {
            ResponsesSystemMessageMode::Developer
        } else {
            caps.system_message_mode
        });

    // -- Input conversion --
    let store_bool = openai_option(provider_opts, "store")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let input_result = convert_to_responses_input(
        &options.prompt,
        system_message_mode,
        store_bool,
        has_previous_response_id,
    );
    warnings.extend(input_result.warnings);

    // -- Base body --
    let mut body = json!({
        "model": model_id,
        "input": input_result.input,
    });

    if stream {
        body["stream"] = json!(true);
    }

    // max_output_tokens
    if let Some(max_tokens) = options.max_output_tokens {
        body["max_output_tokens"] = json!(max_tokens);
    }

    // temperature / top_p (subject to reasoning-model restrictions)
    let mut temperature = options.temperature;
    let mut top_p = options.top_p;

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
    } else {
        if openai_option(provider_opts, "reasoningEffort").is_some() {
            warnings.push(Warning::Unsupported {
                feature: "reasoningEffort".to_string(),
                details: Some(
                    "reasoningEffort is not supported for non-reasoning models".to_string(),
                ),
            });
        }
        if openai_option(provider_opts, "reasoningSummary").is_some() {
            warnings.push(Warning::Unsupported {
                feature: "reasoningSummary".to_string(),
                details: Some(
                    "reasoningSummary is not supported for non-reasoning models".to_string(),
                ),
            });
        }
        if openai_option(provider_opts, "reasoningMode").is_some() {
            warnings.push(Warning::Unsupported {
                feature: "reasoningMode".to_string(),
                details: Some(
                    "reasoningMode is not supported for non-reasoning models".to_string(),
                ),
            });
        }
        if openai_option(provider_opts, "reasoningContext").is_some() {
            warnings.push(Warning::Unsupported {
                feature: "reasoningContext".to_string(),
                details: Some(
                    "reasoningContext is not supported for non-reasoning models".to_string(),
                ),
            });
        }
    }

    if let Some(t) = temperature {
        body["temperature"] = json!(t);
    }
    if let Some(p) = top_p {
        body["top_p"] = json!(p);
    }

    // -- Response format (text.format) --
    if let Some(ref rf) = options.response_format {
        match rf {
            ResponseFormat::Text => {}
            ResponseFormat::Json {
                schema,
                name,
                description,
            } => {
                let mut text = json!({});
                match schema {
                    Some(schema) => {
                        let strict_json = openai_option(provider_opts, "strictJsonSchema")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        text["format"] = json!({
                            "type": "json_schema",
                            "strict": strict_json,
                            "name": name.clone().unwrap_or_else(|| "response".to_string()),
                            "description": description,
                            "schema": schema,
                        });
                    }
                    None => {
                        text["format"] = json!({ "type": "json_object" });
                    }
                }
                body["text"] = text;
            }
        }
    }

    // textVerbosity
    if let Some(verbosity) = openai_option(provider_opts, "textVerbosity") {
        let text = body.get_mut("text").and_then(|t| t.as_object_mut());
        match text {
            Some(obj) => {
                obj.insert("verbosity".to_string(), verbosity);
            }
            None => {
                body["text"] = json!({ "verbosity": verbosity });
            }
        }
    }

    // -- include (computed) --
    let mut include: Option<Vec<Value>> =
        openai_option(provider_opts, "include").and_then(|v| v.as_array().cloned());

    let add_include = |key: &str, inc: &mut Option<Vec<Value>>| {
        let already = inc
            .as_ref()
            .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(key)));
        if !already {
            match inc {
                Some(arr) => arr.push(json!(key)),
                None => *inc = Some(vec![json!(key)]),
            }
        }
    };

    // store defaults to true; only the explicit `false` triggers encrypted_content.
    let store_explicit = openai_option(provider_opts, "store").and_then(|v| v.as_bool());
    if store_explicit == Some(false) && is_reasoning_model {
        add_include("reasoning.encrypted_content", &mut include);
    }

    if let Some(inc) = include {
        body["include"] = json!(inc);
    }

    // -- store (only sent when explicitly set) --
    if let Some(s) = openai_option(provider_opts, "store").and_then(|v| v.as_bool()) {
        body["store"] = json!(s);
    }

    // -- Other provider options (only sent when set) --
    if let Some(v) = openai_option(provider_opts, "conversation") {
        body["conversation"] = v;
    }
    if let Some(v) = openai_option(provider_opts, "maxToolCalls") {
        body["max_tool_calls"] = v;
    }
    if let Some(v) = openai_option(provider_opts, "metadata") {
        body["metadata"] = v;
    }
    if let Some(v) = openai_option(provider_opts, "parallelToolCalls") {
        body["parallel_tool_calls"] = v;
    }
    if let Some(v) = openai_option(provider_opts, "previousResponseId") {
        body["previous_response_id"] = v;
    }
    if let Some(v) = openai_option(provider_opts, "user") {
        body["user"] = v;
    }
    if let Some(v) = openai_option(provider_opts, "instructions") {
        body["instructions"] = v;
    }
    if let Some(v) = openai_option(provider_opts, "promptCacheKey") {
        body["prompt_cache_key"] = v;
    }
    if let Some(v) = openai_option(provider_opts, "promptCacheOptions") {
        body["prompt_cache_options"] = v;
    }
    if let Some(v) = openai_option(provider_opts, "promptCacheRetention") {
        body["prompt_cache_retention"] = v;
    }
    if let Some(v) = openai_option(provider_opts, "safetyIdentifier") {
        body["safety_identifier"] = v;
    }
    if let Some(v) = openai_option(provider_opts, "truncation") {
        body["truncation"] = v;
    }

    // -- service_tier (with capability validation) --
    if let Some(st) =
        openai_option(provider_opts, "serviceTier").and_then(|v| v.as_str().map(|s| s.to_string()))
    {
        match st.as_str() {
            "flex" if !caps.supports_flex_processing => {
                warnings.push(Warning::Unsupported {
                    feature: "serviceTier".to_string(),
                    details: Some(
                        "flex processing is only available for o3, o4-mini, and gpt-5 models"
                            .to_string(),
                    ),
                });
            }
            "priority" if !caps.supports_priority_processing => {
                warnings.push(Warning::Unsupported {
                    feature: "serviceTier".to_string(),
                    details: Some("priority processing is only available for supported models (gpt-4, gpt-5, gpt-5-mini, o3, o4-mini) and requires Enterprise access. gpt-5-nano is not supported".to_string()),
                });
            }
            _ => {
                body["service_tier"] = json!(st);
            }
        }
    }

    // -- reasoning block (reasoning models only) --
    if is_reasoning_model {
        let effort = resolved_reasoning_effort.as_ref();
        let summary = resolved_reasoning_summary.as_ref();
        let mode = openai_option(provider_opts, "reasoningMode")
            .and_then(|v| v.as_str().map(|s| s.to_string()));
        let context = openai_option(provider_opts, "reasoningContext")
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        if effort.is_some() || summary.is_some() || mode.is_some() || context.is_some() {
            let mut reasoning = json!({});
            if let Some(e) = effort {
                reasoning["effort"] = json!(e);
            }
            if let Some(s) = summary {
                reasoning["summary"] = json!(s);
            }
            if let Some(m) = mode {
                reasoning["mode"] = json!(m);
            }
            if let Some(c) = context {
                reasoning["context"] = json!(c);
            }
            body["reasoning"] = reasoning;
        }
    }

    // -- Tools --
    let prepared = prepare_responses_tools(&options.tools, Some(&options.tool_choice));
    if let Some(tools) = prepared.tools {
        body["tools"] = json!(tools);
        if let Some(tc) = prepared.tool_choice {
            body["tool_choice"] = tc;
        }
    }
    for tw in prepared.tool_warnings {
        warnings.push(tw);
    }

    ResponsesRequestBodyResult { body, warnings }
}

// -- Usage conversion --------------------------------------------------------

/// Convert a Responses API usage into the core `Usage` type.
///
/// Mirrors TS `convertOpenAIResponsesUsage`:
/// - `input.total = input_tokens`
/// - `input.noCache = input_tokens - cached_tokens - cache_write_tokens`
/// - `input.cacheRead = cached_tokens`
/// - `input.cacheWrite = cache_write_tokens`
/// - `output.total = output_tokens`
/// - `output.text = output_tokens - reasoning_tokens`
/// - `output.reasoning = reasoning_tokens`
pub fn convert_responses_usage(usage: Option<&ResponsesUsage>) -> Usage {
    let Some(usage) = usage else {
        return Usage::default();
    };

    let input_tokens = usage.input_tokens;
    let output_tokens = usage.output_tokens;

    let cached_tokens = usage
        .input_tokens_details
        .as_ref()
        .and_then(|d| d.cached_tokens)
        .unwrap_or(0);
    let cache_write = usage
        .input_tokens_details
        .as_ref()
        .and_then(|d| d.cache_write_tokens);
    let reasoning_tokens = usage
        .output_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens)
        .unwrap_or(0);

    let no_cache = input_tokens - cached_tokens - cache_write.unwrap_or(0);
    let text_tokens = output_tokens - reasoning_tokens;

    Usage {
        input_tokens: aimux_core::types::TokenUsage {
            total: Some(input_tokens),
            no_cache: Some(no_cache),
            cache_read: Some(cached_tokens),
            cache_write,
            ..Default::default()
        },
        output_tokens: aimux_core::types::TokenUsage {
            total: Some(output_tokens),
            text: Some(text_tokens),
            reasoning: Some(reasoning_tokens),
            ..Default::default()
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: Some(serde_json::to_value(usage).unwrap_or(serde_json::Value::Null)),
    }
}

/// Helper: build a `ResponsesUsage` from a raw JSON `usage` object.
pub fn parse_usage(raw: &Value) -> Option<ResponsesUsage> {
    serde_json::from_value(raw.clone()).ok()
}

// -- Finish reason -----------------------------------------------------------

/// Map a Responses API finish reason into the unified `FinishReason`.
///
/// Mirrors TS `mapOpenAIResponseFinishReason`. When `finish_reason` is
/// `None`/null, the unified reason is `tool-calls` if there were function
/// calls, else `stop`. `"max_output_tokens"` -> `length`,
/// `"content_filter"` -> `content-filter`; otherwise `tool-calls` if there
/// were function calls, else `other`.
pub fn map_responses_finish_reason(
    finish_reason: Option<&str>,
    has_function_call: bool,
) -> FinishReason {
    let unified = match finish_reason {
        None => {
            if has_function_call {
                FinishReasonUnified::ToolCalls
            } else {
                FinishReasonUnified::Stop
            }
        }
        Some("max_output_tokens") => FinishReasonUnified::Length,
        Some("content_filter") => FinishReasonUnified::ContentFilter,
        Some(_) => {
            if has_function_call {
                FinishReasonUnified::ToolCalls
            } else {
                FinishReasonUnified::Other
            }
        }
    };
    FinishReason {
        unified,
        raw: finish_reason.map(|s| s.to_string()),
    }
}
