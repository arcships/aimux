//! Conversion functions for the xAI Responses API.
//!
//! Mirrors the TS sources:
//! - `convert-to-xai-responses-input.ts`
//! - `convert-xai-responses-usage.ts`
//! - `map-xai-responses-finish-reason.ts`
//! - `xai-responses-prepare-tools.ts`

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::tool::Tool;
use aimux_core::types::{FinishReasonUnified, ReasoningEffort, Warning};

use serde_json::{Value, json};

use super::types::XaiResponsesUsage;
use crate::xai::convert::{
    remove_additional_properties_false, resolve_full_media_type, resolve_provider_reference,
    supports_reasoning_effort,
};

// ── Finish reason ────────────────────────────────────────────────────────────

#[must_use]
pub fn map_xai_responses_finish_reason(s: &str) -> FinishReasonUnified {
    match s {
        "stop" | "completed" => FinishReasonUnified::Stop,
        "length" | "max_output_tokens" => FinishReasonUnified::Length,
        "tool_calls" | "function_call" => FinishReasonUnified::ToolCalls,
        "content_filter" => FinishReasonUnified::ContentFilter,
        "error" => FinishReasonUnified::Error,
        _ => FinishReasonUnified::Other,
    }
}

// ── Usage conversion ─────────────────────────────────────────────────────────

#[must_use]
pub fn convert_xai_responses_usage(usage: &XaiResponsesUsage) -> aimux_core::types::Usage {
    let cache_read = usage
        .input_tokens_details
        .as_ref()
        .and_then(|d| d.cached_tokens)
        .unwrap_or(0);
    let reasoning = usage
        .output_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens)
        .unwrap_or(0);

    let input_includes_cached = cache_read <= usage.input_tokens;
    let input_total = if input_includes_cached {
        usage.input_tokens
    } else {
        usage.input_tokens + cache_read
    };
    let input_no_cache = if input_includes_cached {
        usage.input_tokens - cache_read
    } else {
        usage.input_tokens
    };

    aimux_core::types::Usage {
        input_tokens: aimux_core::types::TokenUsage {
            total: Some(input_total as u32),
            no_cache: Some(input_no_cache as u32),
            cache_read: Some(cache_read as u32),
            cache_write: None,
            ..Default::default()
        },
        output_tokens: aimux_core::types::TokenUsage {
            total: Some(usage.output_tokens as u32),
            text: Some((usage.output_tokens - reasoning) as u32),
            reasoning: Some(reasoning as u32),
            ..Default::default()
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: Some(serde_json::to_value(usage).unwrap_or(serde_json::Value::Null)),
    }
}

// ── Tool preparation ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PreparedResponsesTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub tool_warnings: Vec<Warning>,
    /// Names of provider-executed tools, keyed by tool id (e.g. "xai.web_search").
    pub provider_tool_names: std::collections::HashMap<String, String>,
}

#[must_use]
pub fn prepare_responses_tools(
    tools: &Option<Vec<Tool>>,
    tool_choice: Option<&ToolChoice>,
) -> PreparedResponsesTools {
    let non_empty = tools.as_ref().filter(|t| !t.is_empty());
    let mut tool_warnings: Vec<Warning> = Vec::new();
    let mut provider_tool_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    let tools_opt = match non_empty {
        None => None,
        Some(tools) => {
            let xai_tools: Vec<Value> = tools
                .iter()
                .filter_map(|t| match t {
                    Tool::Function(ft) => {
                        let mut func = json!({
                            "type": "function",
                            "name": ft.name,
                            "parameters": remove_additional_properties_false(&ft.input_schema),
                        });
                        if let Some(ref desc) = ft.description {
                            func["description"] = json!(desc);
                        }
                        if let Some(strict) = ft.strict {
                            func["strict"] = json!(strict);
                        }
                        Some(func)
                    }
                    Tool::Provider(pt) => {
                        provider_tool_names.insert(pt.id.clone(), pt.name.clone());
                        prepare_provider_tool(pt, &mut tool_warnings)
                    }
                })
                .collect();
            if xai_tools.is_empty() {
                None
            } else {
                Some(xai_tools)
            }
        }
    };

    let tool_choice_opt = match (&tools_opt, tool_choice) {
        (None, _) => None,
        (Some(_), None) => None,
        (Some(_), Some(tc)) => {
            // Build a map of tool name -> is_provider_tool from the original tools.
            let non_empty_tools = non_empty.unwrap();
            match tc {
                ToolChoice::Auto => Some(json!("auto")),
                ToolChoice::None => Some(json!("none")),
                ToolChoice::Required => Some(json!("required")),
                ToolChoice::Tool { tool_name } => {
                    let selected = non_empty_tools.iter().find(|t| match t {
                        Tool::Function(ft) => ft.name == *tool_name,
                        Tool::Provider(pt) => pt.name == *tool_name,
                    });

                    match selected {
                        None => None,
                        Some(Tool::Function(_)) => {
                            Some(json!({ "type": "function", "name": tool_name }))
                        }
                        Some(Tool::Provider(pt)) => {
                            // Server-side tools cannot be forced via toolChoice.
                            tool_warnings.push(Warning::Unsupported {
                                feature: format!("toolChoice for server-side tool \"{}\"", pt.name),
                                details: None,
                            });
                            None
                        }
                    }
                }
            }
        }
    };

    PreparedResponsesTools {
        tools: tools_opt,
        tool_choice: tool_choice_opt,
        tool_warnings,
        provider_tool_names,
    }
}

fn prepare_provider_tool(
    pt: &aimux_core::tool::ProviderTool,
    warnings: &mut Vec<Warning>,
) -> Option<Value> {
    let args = &pt.args;
    match pt.id.as_str() {
        "xai.web_search" => {
            let mut tool = json!({ "type": "web_search" });
            if let Some(v) = args.get("allowedDomains") {
                tool["allowed_domains"] = v.clone();
            }
            if let Some(v) = args.get("excludedDomains") {
                tool["excluded_domains"] = v.clone();
            }
            if let Some(v) = args.get("enableImageSearch") {
                tool["enable_image_search"] = v.clone();
            }
            if let Some(v) = args.get("enableImageUnderstanding") {
                tool["enable_image_understanding"] = v.clone();
            }
            Some(tool)
        }
        "xai.x_search" => {
            let mut tool = json!({ "type": "x_search" });
            if let Some(v) = args.get("allowedXHandles") {
                tool["allowed_x_handles"] = v.clone();
            }
            if let Some(v) = args.get("excludedXHandles") {
                tool["excluded_x_handles"] = v.clone();
            }
            if let Some(v) = args.get("fromDate") {
                tool["from_date"] = v.clone();
            }
            if let Some(v) = args.get("toDate") {
                tool["to_date"] = v.clone();
            }
            if let Some(v) = args.get("enableImageUnderstanding") {
                tool["enable_image_understanding"] = v.clone();
            }
            if let Some(v) = args.get("enableVideoUnderstanding") {
                tool["enable_video_understanding"] = v.clone();
            }
            Some(tool)
        }
        "xai.code_execution" => Some(json!({ "type": "code_interpreter" })),
        "xai.view_image" => Some(json!({ "type": "view_image" })),
        "xai.view_x_video" => Some(json!({ "type": "view_x_video" })),
        "xai.file_search" => {
            let mut tool = json!({ "type": "file_search" });
            if let Some(v) = args.get("vectorStoreIds") {
                tool["vector_store_ids"] = v.clone();
            }
            if let Some(v) = args.get("maxNumResults") {
                tool["max_num_results"] = v.clone();
            }
            Some(tool)
        }
        "xai.mcp" => {
            let mut tool = json!({ "type": "mcp" });
            if let Some(v) = args.get("serverUrl") {
                tool["server_url"] = v.clone();
            }
            if let Some(v) = args.get("serverLabel") {
                tool["server_label"] = v.clone();
            }
            if let Some(v) = args.get("serverDescription") {
                tool["server_description"] = v.clone();
            }
            if let Some(v) = args.get("allowedTools") {
                tool["allowed_tools"] = v.clone();
            }
            if let Some(v) = args.get("headers") {
                tool["headers"] = v.clone();
            }
            if let Some(v) = args.get("authorization") {
                tool["authorization"] = v.clone();
            }
            Some(tool)
        }
        _ => {
            warnings.push(Warning::Unsupported {
                feature: format!("provider-defined tool {}", pt.name),
                details: None,
            });
            None
        }
    }
}

// ── Provider options helpers ─────────────────────────────────────────────────

fn xai_option(
    options: &Option<std::collections::HashMap<String, Value>>,
    key: &str,
) -> Option<Value> {
    options
        .as_ref()
        .and_then(|m| m.get("xai"))
        .and_then(|o| o.get(key))
        .cloned()
}

// ── Input conversion ─────────────────────────────────────────────────────────

/// Convert a [`LanguageModelPrompt`] into the xAI Responses `input` array,
/// returning warnings.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when a part cannot be converted.
pub fn convert_to_xai_responses_input(
    prompt: &LanguageModelPrompt,
) -> Result<(Vec<Value>, Vec<Warning>), AiMuxError> {
    let mut input = Vec::new();
    let mut warnings: Vec<Warning> = Vec::new();

    for msg in prompt {
        match msg.role {
            Role::System => {
                let content: String = msg
                    .content
                    .iter()
                    .filter_map(|p| match p {
                        ContentPart::Text { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                input.push(json!({ "role": "system", "content": content }));
            }
            Role::User => {
                let mut content_parts = Vec::new();
                for part in &msg.content {
                    match part {
                        ContentPart::Text { text, .. } => {
                            content_parts.push(json!({ "type": "input_text", "text": text }));
                        }
                        ContentPart::Image {
                            image,
                            media_type,
                            provider_options,
                        } => {
                            use base64::Engine;
                            let b64 = base64::engine::general_purpose::STANDARD.encode(image);
                            content_parts.push(convert_image_part(
                                media_type,
                                Some(&b64),
                                None,
                                provider_options,
                            ));
                        }
                        ContentPart::File {
                            data,
                            media_type,
                            provider_options,
                            ..
                        } => {
                            use base64::Engine;
                            let b64 = base64::engine::general_purpose::STANDARD.encode(data);
                            content_parts.push(convert_image_part(
                                media_type,
                                Some(&b64),
                                None,
                                provider_options,
                            ));
                        }
                        ContentPart::FileBase64 {
                            data,
                            media_type,
                            provider_options,
                            ..
                        } => {
                            content_parts.push(convert_image_part(
                                media_type,
                                Some(data),
                                None,
                                provider_options,
                            ));
                        }
                        ContentPart::FileUrl {
                            url,
                            media_type,
                            provider_options,
                        } => {
                            let top_level = media_type.split('/').next().unwrap_or("");
                            if top_level == "image" {
                                content_parts.push(convert_image_part(
                                    media_type,
                                    None,
                                    Some(url),
                                    provider_options,
                                ));
                            } else {
                                content_parts
                                    .push(json!({ "type": "input_file", "file_url": url }));
                            }
                        }
                        ContentPart::FileReference {
                            media_type,
                            reference,
                            provider_options,
                            ..
                        } => {
                            let _ = (media_type, provider_options);
                            let file_id = resolve_provider_reference(reference, "xai")
                                .map_err(AiMuxError::InvalidArgument)?;
                            content_parts.push(json!({ "type": "input_file", "file_id": file_id }));
                        }
                        _ => {
                            warnings.push(Warning::Other {
                                message: "xAI Responses API does not support this content type in user messages".to_string(),
                            });
                        }
                    }
                }
                input.push(json!({ "role": "user", "content": content_parts }));
            }
            Role::Assistant => {
                for part in &msg.content {
                    match part {
                        ContentPart::Text {
                            text,
                            provider_options,
                            ..
                        } => {
                            let id = provider_options
                                .as_ref()
                                .and_then(|po| po.get("xai"))
                                .and_then(|x| x.get("itemId"))
                                .and_then(|v| v.as_str())
                                .map(std::string::ToString::to_string);
                            let mut msg_obj = json!({ "role": "assistant", "content": text });
                            if let Some(id) = id {
                                msg_obj["id"] = json!(id);
                            }
                            input.push(msg_obj);
                        }
                        ContentPart::ToolCall {
                            tool_call_id,
                            tool_name,
                            input: tool_input,
                            provider_options,
                            ..
                        } => {
                            // Skip provider-executed tool calls.
                            let is_provider_executed = provider_options
                                .as_ref()
                                .and_then(|po| po.get("xai"))
                                .and_then(|x| x.get("providerExecuted"))
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false);
                            if is_provider_executed {
                                continue;
                            }
                            let id = provider_options
                                .as_ref()
                                .and_then(|po| po.get("xai"))
                                .and_then(|x| x.get("itemId"))
                                .and_then(|v| v.as_str())
                                .map(std::string::ToString::to_string);
                            let item_id = id.unwrap_or_else(|| tool_call_id.clone());
                            let arguments = if tool_input.is_null() {
                                "{}".to_string()
                            } else {
                                tool_input.to_string()
                            };
                            input.push(json!({
                                "type": "function_call",
                                "id": item_id,
                                "call_id": tool_call_id,
                                "name": tool_name,
                                "arguments": arguments,
                                "status": "completed"
                            }));
                        }
                        ContentPart::ToolResult { .. } => {}
                        ContentPart::Reasoning {
                            text,
                            provider_options,
                            ..
                        } => {
                            let item_id = provider_options
                                .as_ref()
                                .and_then(|po| po.get("xai"))
                                .and_then(|x| x.get("itemId"))
                                .and_then(|v| v.as_str())
                                .map(std::string::ToString::to_string);
                            let encrypted_content = provider_options
                                .as_ref()
                                .and_then(|po| po.get("xai"))
                                .and_then(|x| x.get("reasoningEncryptedContent"))
                                .and_then(|v| v.as_str())
                                .map(std::string::ToString::to_string);

                            if item_id.is_some() || encrypted_content.is_some() {
                                let mut summary: Vec<Value> = Vec::new();
                                if !text.is_empty() {
                                    summary.push(json!({ "type": "summary_text", "text": text }));
                                }
                                let mut reasoning_obj = json!({
                                    "type": "reasoning",
                                    "id": item_id.clone().unwrap_or_default(),
                                    "summary": summary,
                                    "status": "completed"
                                });
                                if let Some(ec) = encrypted_content {
                                    reasoning_obj["encrypted_content"] = json!(ec);
                                }
                                input.push(reasoning_obj);
                            } else {
                                warnings.push(Warning::Other {
                                    message: "Reasoning parts without itemId or encrypted content cannot be sent back to xAI. Skipping.".to_string(),
                                });
                            }
                        }
                        _ => {
                            warnings.push(Warning::Other {
                                message: "xAI Responses API does not support this content type in assistant messages".to_string(),
                            });
                        }
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
                        let output_value = match result {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        input.push(json!({
                            "type": "function_call_output",
                            "call_id": tool_call_id,
                            "output": output_value
                        }));
                    }
                }
            }
        }
    }

    Ok((input, warnings))
}

fn convert_image_part(
    media_type: &str,
    b64_data: Option<&str>,
    url: Option<&str>,
    provider_options: &Option<Value>,
) -> Value {
    let image_url = if let Some(url_str) = url {
        url_str.to_string()
    } else if let Some(b64) = b64_data {
        let full_mt = resolve_full_media_type(media_type, b64);
        format!("data:{full_mt};base64,{b64}")
    } else {
        String::new()
    };

    let mut part = json!({
        "type": "input_image",
        "image_url": image_url,
    });

    // Image detail provider option.
    if let Some(detail) = provider_options
        .as_ref()
        .and_then(|po| po.get("xai"))
        .and_then(|x| x.get("imageDetail"))
    {
        part["detail"] = detail.clone();
    }

    part
}

// ── Request body builder ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ResponsesRequestBodyResult {
    pub body: Value,
    pub warnings: Vec<Warning>,
    pub provider_tool_names: std::collections::HashMap<String, String>,
}

/// Build the xAI Responses request body, returning warnings and provider tool
/// names.
///
/// # Errors
///
/// Propagates conversion errors from `convert_to_xai_responses_input`.
pub fn build_responses_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<ResponsesRequestBodyResult, AiMuxError> {
    let mut warnings: Vec<Warning> = Vec::new();
    let xai_opts = &options.provider_options;

    if options.stop_sequences.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "stopSequences".to_string(),
            details: None,
        });
    }

    let (input, input_warnings) = convert_to_xai_responses_input(&options.prompt)?;
    warnings.extend(input_warnings);

    let prepared = prepare_responses_tools(&options.tools, Some(&options.tool_choice));
    for tw in &prepared.tool_warnings {
        warnings.push(tw.clone());
    }

    // Reasoning effort.
    let mut reasoning_effort: Option<String> = xai_option(xai_opts, "reasoningEffort").map(|v| {
        v.as_str()
            .map(std::string::ToString::to_string)
            .unwrap_or(v.to_string())
    });

    if reasoning_effort.is_none() && options.reasoning.is_some_and(ReasoningEffort::is_custom) {
        let reasoning = options.reasoning.unwrap();
        if !supports_reasoning_effort(model_id) {
            warnings.push(Warning::Unsupported {
                feature: "reasoning".to_string(),
                details: Some(format!(
                    "reasoning \"{reasoning}\" is not supported by this model."
                )),
            });
        } else if reasoning == ReasoningEffort::None {
            reasoning_effort = Some("none".to_string());
        } else {
            reasoning_effort = Some(match reasoning {
                ReasoningEffort::Minimal => "low".to_string(),
                ReasoningEffort::Low => "low".to_string(),
                ReasoningEffort::Medium => "medium".to_string(),
                ReasoningEffort::High => "high".to_string(),
                ReasoningEffort::Xhigh => "high".to_string(),
                _ => reasoning.to_string(),
            });
        }
    }

    let reasoning_summary = xai_option(xai_opts, "reasoningSummary");

    // Store option.
    let store = xai_option(xai_opts, "store").and_then(|v| v.as_bool());

    // Include array.
    let mut include: Option<Vec<String>> = xai_option(xai_opts, "include").and_then(|v| {
        v.as_array().map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(std::string::ToString::to_string))
                .collect()
        })
    });

    if store == Some(false) {
        include = match include {
            None => Some(vec!["reasoning.encrypted_content".to_string()]),
            Some(mut v) => {
                v.push("reasoning.encrypted_content".to_string());
                Some(v)
            }
        };
    }

    let previous_response_id = xai_option(xai_opts, "previousResponseId")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string));

    // Logprobs.
    let logprobs = xai_option(xai_opts, "logprobs")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let top_logprobs = xai_option(xai_opts, "topLogprobs");

    // Build body.
    let mut body = json!({ "model": model_id, "input": input });

    if stream {
        body["stream"] = json!(true);
    }
    if let Some(max_tokens) = options.max_output_tokens {
        body["max_output_tokens"] = json!(max_tokens);
    }
    if let Some(temp) = options.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(tp) = options.top_p {
        body["top_p"] = json!(tp);
    }
    if let Some(seed) = options.seed {
        body["seed"] = json!(seed);
    }
    if logprobs || top_logprobs.is_some() {
        body["logprobs"] = json!(true);
    }
    if let Some(tlp) = top_logprobs {
        body["top_logprobs"] = tlp;
    }

    // Reasoning object.
    if reasoning_effort.is_some() || reasoning_summary.is_some() {
        let mut reasoning_obj = json!({});
        if let Some(effort) = &reasoning_effort {
            reasoning_obj["effort"] = json!(effort);
        }
        if let Some(summary) = &reasoning_summary {
            reasoning_obj["summary"] = json!(summary);
        }
        body["reasoning"] = reasoning_obj;
    }

    // Store.
    if store == Some(false) {
        body["store"] = json!(false);
    }

    // Include.
    if let Some(inc) = &include {
        body["include"] = json!(inc);
    }

    // Previous response ID.
    if let Some(prev_id) = &previous_response_id {
        body["previous_response_id"] = json!(prev_id);
    }

    // Response format.
    if let Some(ref rf) = options.response_format
        && let ResponseFormat::Json {
            schema,
            name,
            description,
        } = rf
    {
        let format = if schema.is_some() {
            let mut fmt = json!({ "type": "json_schema", "strict": true });
            fmt["name"] = json!(name.clone().unwrap_or_else(|| "response".to_string()));
            if let Some(desc) = description {
                fmt["description"] = json!(desc);
            }
            if let Some(s) = schema {
                fmt["schema"] = s.clone();
            }
            fmt
        } else {
            json!({ "type": "json_object" })
        };
        body["text"] = json!({ "format": format });
    }

    // Tools.
    if let Some(tools) = &prepared.tools {
        body["tools"] = json!(tools);
    }
    if let Some(tc) = prepared.tool_choice {
        body["tool_choice"] = tc;
    }

    Ok(ResponsesRequestBodyResult {
        body,
        warnings,
        provider_tool_names: prepared.provider_tool_names,
    })
}

// ── Tool name resolution helpers ─────────────────────────────────────────────

/// Sub-tool names that map to web_search.
pub const WEB_SEARCH_SUB_TOOLS: &[&str] =
    &["web_search", "web_search_with_snippets", "browse_page"];
/// Sub-tool names that map to x_search.
pub const X_SEARCH_SUB_TOOLS: &[&str] = &[
    "x_user_search",
    "x_keyword_search",
    "x_semantic_search",
    "x_thread_fetch",
];

/// Resolve the tool name for a server-side tool call, using the user-provided
/// provider tool name if available, falling back to a default.
#[must_use]
pub fn resolve_tool_name(
    part_type: &str,
    part_name: Option<&str>,
    provider_tool_names: &std::collections::HashMap<String, String>,
) -> String {
    let name = part_name.unwrap_or("");
    let default_web = provider_tool_names
        .get("xai.web_search")
        .cloned()
        .unwrap_or_else(|| "web_search".to_string());
    let default_x = provider_tool_names
        .get("xai.x_search")
        .cloned()
        .unwrap_or_else(|| "x_search".to_string());
    let default_code = provider_tool_names
        .get("xai.code_execution")
        .cloned()
        .unwrap_or_else(|| "code_execution".to_string());
    let default_mcp = provider_tool_names
        .get("xai.mcp")
        .cloned()
        .unwrap_or_else(|| "mcp".to_string());
    let default_file_search = provider_tool_names
        .get("xai.file_search")
        .cloned()
        .unwrap_or_else(|| "file_search".to_string());

    if WEB_SEARCH_SUB_TOOLS.contains(&name) || part_type == "web_search_call" {
        default_web
    } else if X_SEARCH_SUB_TOOLS.contains(&name) || part_type == "x_search_call" {
        default_x
    } else if name == "code_execution"
        || part_type == "code_interpreter_call"
        || part_type == "code_execution_call"
    {
        default_code
    } else if part_type == "mcp_call" {
        default_mcp
    } else if part_type == "file_search_call" {
        default_file_search
    } else if !name.is_empty() {
        name.to_string()
    } else {
        String::new()
    }
}

/// Get the tool input for a server-side tool call.
#[must_use]
pub fn get_tool_input(part_type: &str, part: &Value) -> String {
    match part_type {
        "custom_tool_call" => part
            .get("input")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "mcp_call" => part
            .get("arguments")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => part
            .get("arguments")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    }
}
