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
// `GptVersion` / `get_gpt_version` / `get_o_series_version` /
// `ModelCapabilities` / `SystemMessageMode` / `get_model_capabilities` live in
// `crate::openai::convert_common` and are shared with the Chat Completions
// converter (issue M10).
use crate::openai::convert_common::{ModelCapabilities, SystemMessageMode, get_model_capabilities};

/// Compatibility alias: the Responses-specific enum name was merged into the
/// shared [`SystemMessageMode`] during M10. Keeping the alias lets existing
/// import paths (`openai::responses::convert::ResponsesSystemMessageMode`)
/// keep compiling while the module signature uses the shared type.
#[doc(hidden)]
pub use crate::openai::convert_common::SystemMessageMode as ResponsesSystemMessageMode;

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
    system_message_mode: SystemMessageMode,
    store: bool,
    has_previous_response_id: bool,
) -> ResponsesInputResult {
    let mut input: Vec<Value> = Vec::new();
    let mut warnings: Vec<Warning> = Vec::new();

    for msg in prompt {
        match msg.role {
            Role::System => match system_message_mode {
                SystemMessageMode::System => {
                    input.push(json!({
                        "role": "system",
                        "content": join_text_parts(&msg.content),
                    }));
                }
                SystemMessageMode::Developer => {
                    input.push(json!({
                        "role": "developer",
                        "content": join_text_parts(&msg.content),
                    }));
                }
                SystemMessageMode::Remove => {
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
        .map(std::string::ToString::to_string)
}

/// Read the `phase` from a content part's `providerOptions.openai.phase`.
fn phase_from_provider_options(provider_options: &Option<Value>) -> Option<String> {
    provider_options
        .as_ref()
        .and_then(|v| v.get("openai"))
        .and_then(|o| o.get("phase"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string)
}

/// Read the `namespace` from a content part's `providerOptions.openai.namespace`.
fn namespace_from_provider_options(provider_options: &Option<Value>) -> Option<String> {
    provider_options
        .as_ref()
        .and_then(|v| v.get("openai"))
        .and_then(|o| o.get("namespace"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string)
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
#[must_use]
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
/// Compatibility warnings for call options the Responses API does not carry.
fn push_unsupported_call_option_warnings(options: &CallOptions, warnings: &mut Vec<Warning>) {
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
}

/// Resolve the Responses reasoning config: `reasoningEffort` (provider option
/// wins over top-level `reasoning`), `reasoningSummary` (defaults to "detailed"
/// when an effort other than "none" applies), and whether the model reasons.
fn resolve_responses_reasoning(
    provider_opts: &Option<HashMap<String, Value>>,
    options: &CallOptions,
    caps: &ModelCapabilities,
) -> (Option<String>, Option<String>, bool) {
    let resolved_reasoning_effort: Option<String> = openai_option(provider_opts, "reasoningEffort")
        .map(|v| {
            v.as_str()
                .map(std::string::ToString::to_string)
                .unwrap_or_else(|| v.to_string())
        })
        .or_else(|| {
            if options.reasoning.is_some_and(ReasoningEffort::is_custom) {
                options.reasoning.map(|r| r.to_string())
            } else {
                None
            }
        });

    let resolved_reasoning_summary: Option<String> =
        openai_option(provider_opts, "reasoningSummary")
            .map(|v| {
                v.as_str()
                    .map(std::string::ToString::to_string)
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

    (
        resolved_reasoning_effort,
        resolved_reasoning_summary,
        is_reasoning_model,
    )
}

/// Warn when `conversation` and `previousResponseId` are both set.
fn warn_conversation_conflict(
    provider_opts: &Option<HashMap<String, Value>>,
    warnings: &mut Vec<Warning>,
) {
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
}

fn resolve_responses_system_message_mode(
    provider_opts: &Option<HashMap<String, Value>>,
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

/// temperature / top_p, subject to reasoning-model restrictions.
fn apply_responses_sampling(
    body: &mut Value,
    options: &CallOptions,
    provider_opts: &Option<HashMap<String, Value>>,
    caps: &ModelCapabilities,
    is_reasoning_model: bool,
    resolved_reasoning_effort: &Option<String>,
    warnings: &mut Vec<Warning>,
) {
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
        for key in [
            "reasoningEffort",
            "reasoningSummary",
            "reasoningMode",
            "reasoningContext",
        ] {
            if openai_option(provider_opts, key).is_some() {
                warnings.push(Warning::Unsupported {
                    feature: key.to_string(),
                    details: Some(format!("{key} is not supported for non-reasoning models")),
                });
            }
        }
    }

    if let Some(t) = temperature {
        body["temperature"] = json!(t);
    }
    if let Some(p) = top_p {
        body["top_p"] = json!(p);
    }
}

/// `text.format` (json_schema / json_object) plus `verbosity`.
fn apply_responses_text_format(
    body: &mut Value,
    options: &CallOptions,
    provider_opts: &Option<HashMap<String, Value>>,
) {
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
}

/// The computed `include` list (store=false on reasoning models adds
/// `reasoning.encrypted_content`).
fn resolve_responses_include(
    provider_opts: &Option<HashMap<String, Value>>,
    is_reasoning_model: bool,
) -> Option<Vec<Value>> {
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

    include
}

/// Pass-through of the remaining Responses provider options (only sent when
/// set).
fn apply_responses_provider_options(
    body: &mut Value,
    provider_opts: &Option<HashMap<String, Value>>,
) {
    let mut set = |key: &str, body_key: &str| {
        if let Some(v) = openai_option(provider_opts, key) {
            body[body_key] = v;
        }
    };
    set("conversation", "conversation");
    set("maxToolCalls", "max_tool_calls");
    set("metadata", "metadata");
    set("parallelToolCalls", "parallel_tool_calls");
    set("previousResponseId", "previous_response_id");
    set("user", "user");
    set("instructions", "instructions");
    set("promptCacheKey", "prompt_cache_key");
    set("promptCacheOptions", "prompt_cache_options");
    set("promptCacheRetention", "prompt_cache_retention");
    set("safetyIdentifier", "safety_identifier");
    set("truncation", "truncation");
}

/// `service_tier` with model-capability validation.
fn apply_responses_service_tier(
    body: &mut Value,
    provider_opts: &Option<HashMap<String, Value>>,
    caps: &ModelCapabilities,
    warnings: &mut Vec<Warning>,
) {
    if let Some(st) = openai_option(provider_opts, "serviceTier")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string))
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
}

/// `reasoning` block for reasoning models.
fn apply_responses_reasoning_block(
    body: &mut Value,
    provider_opts: &Option<HashMap<String, Value>>,
    is_reasoning_model: bool,
    resolved_reasoning_effort: &Option<String>,
    resolved_reasoning_summary: &Option<String>,
) {
    if !is_reasoning_model {
        return;
    }
    let effort = resolved_reasoning_effort.as_ref();
    let summary = resolved_reasoning_summary.as_ref();
    let mode = openai_option(provider_opts, "reasoningMode")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string));
    let context = openai_option(provider_opts, "reasoningContext")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string));

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

/// Build the OpenAI Responses API request body (without warnings).
///
/// Splits the original ~380-line function into focused helpers (issue M11);
/// behavior is unchanged.
#[must_use]
pub fn build_responses_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> ResponsesRequestBodyResult {
    let mut warnings: Vec<Warning> = Vec::new();
    let caps = get_model_capabilities(model_id);
    let provider_opts = &options.provider_options;

    // -- Warnings for unsupported call options --
    push_unsupported_call_option_warnings(options, &mut warnings);

    // -- Reasoning resolution --
    let (resolved_reasoning_effort, resolved_reasoning_summary, is_reasoning_model) =
        resolve_responses_reasoning(provider_opts, options, &caps);

    // -- conversation + previousResponseId conflict --
    warn_conversation_conflict(provider_opts, &mut warnings);

    // -- System message mode --
    let system_message_mode =
        resolve_responses_system_message_mode(provider_opts, is_reasoning_model, &caps);

    // -- Input conversion --
    let store_bool = openai_option(provider_opts, "store")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let has_previous_response_id = openai_option(provider_opts, "previousResponseId").is_some();
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

    if let Some(max_tokens) = options.max_output_tokens {
        body["max_output_tokens"] = json!(max_tokens);
    }

    // temperature / top_p (subject to reasoning-model restrictions)
    apply_responses_sampling(
        &mut body,
        options,
        provider_opts,
        &caps,
        is_reasoning_model,
        &resolved_reasoning_effort,
        &mut warnings,
    );

    // -- Response format (text.format) + verbosity --
    apply_responses_text_format(&mut body, options, provider_opts);

    // -- include (computed) --
    let include = resolve_responses_include(provider_opts, is_reasoning_model);
    if let Some(inc) = include {
        body["include"] = json!(inc);
    }

    // -- store (only sent when explicitly set) --
    if let Some(s) = openai_option(provider_opts, "store").and_then(|v| v.as_bool()) {
        body["store"] = json!(s);
    }

    // -- Other provider options (only sent when set) --
    apply_responses_provider_options(&mut body, provider_opts);

    // -- service_tier (with capability validation) --
    apply_responses_service_tier(&mut body, provider_opts, &caps, &mut warnings);

    // -- reasoning block (reasoning models only) --
    apply_responses_reasoning_block(
        &mut body,
        provider_opts,
        is_reasoning_model,
        &resolved_reasoning_effort,
        &resolved_reasoning_summary,
    );

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
#[must_use]
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
#[must_use]
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
#[must_use]
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
        raw: finish_reason.map(std::string::ToString::to_string),
    }
}
