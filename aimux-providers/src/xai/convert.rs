//! Conversion between `LanguageModelPrompt` and xAI API format.
//!
//! Mirrors the TS xai package's `convert-to-xai-chat-messages.ts`,
//! `convert-xai-chat-usage.ts`, `xai-prepare-tools.ts`,
//! `supports-reasoning-effort.ts`, `map-xai-finish-reason.ts`, and
//! `remove-additional-properties.ts`.

use std::collections::HashMap;

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::tool::Tool;
use aimux_core::types::{FinishReason, FinishReasonUnified, ReasoningEffort, Warning};

use serde_json::{Value, json};

use super::types::XaiUsageResponse;

// ── Finish reason ────────────────────────────────────────────────────────────

/// Map an xAI finish reason string to the unified enum.
pub fn map_xai_finish_reason(s: &str) -> FinishReasonUnified {
    match s {
        "stop" => FinishReasonUnified::Stop,
        "length" => FinishReasonUnified::Length,
        "tool_calls" | "function_call" => FinishReasonUnified::ToolCalls,
        "content_filter" => FinishReasonUnified::ContentFilter,
        _ => FinishReasonUnified::Other,
    }
}

pub fn parse_finish_reason(s: &str) -> FinishReason {
    FinishReason {
        unified: map_xai_finish_reason(s),
        raw: Some(s.to_string()),
    }
}

// ── Reasoning effort support ─────────────────────────────────────────────────

/// Models that reject the `reasoning_effort` parameter.
/// Matches `^grok-4\.20(-\d{4})?-(non-)?reasoning$`.
fn is_model_without_reasoning_effort(model_id: &str) -> bool {
    let rest = match model_id.strip_prefix("grok-4.20") {
        Some(r) => r,
        None => return false,
    };
    let rest = if rest.len() >= 5
        && rest.starts_with('-')
        && rest[1..5].chars().all(|c| c.is_ascii_digit())
    {
        &rest[5..]
    } else {
        rest
    };
    rest == "-reasoning" || rest == "-non-reasoning"
}

/// Whether the model accepts the `reasoning_effort` parameter.
pub fn supports_reasoning_effort(model_id: &str) -> bool {
    !is_model_without_reasoning_effort(model_id)
}

pub fn is_custom_reasoning(reasoning: &Option<ReasoningEffort>) -> bool {
    match reasoning {
        Some(ReasoningEffort::ProviderDefault) => false,
        Some(_) => true,
        None => false,
    }
}

// ── Usage conversion ─────────────────────────────────────────────────────────

pub fn convert_xai_usage(usage: &XaiUsageResponse) -> aimux_core::types::Usage {
    let prompt_tokens = usage.prompt_tokens.unwrap_or(0);
    let completion_tokens = usage.completion_tokens.unwrap_or(0);
    let cache_read = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|d| d.cached_tokens)
        .unwrap_or(0);
    let reasoning_tokens = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens)
        .unwrap_or(0);
    let prompt_includes_cached = cache_read <= prompt_tokens;
    let input_total = if prompt_includes_cached {
        prompt_tokens
    } else {
        prompt_tokens + cache_read
    };
    let input_no_cache = if prompt_includes_cached {
        prompt_tokens - cache_read
    } else {
        prompt_tokens
    };
    let output_total = completion_tokens + reasoning_tokens;

    aimux_core::types::Usage {
        input_tokens: aimux_core::types::TokenUsage {
            total: Some(input_total),
            no_cache: Some(input_no_cache),
            cache_read: Some(cache_read),
            cache_write: None,
            ..Default::default()
        },
        output_tokens: aimux_core::types::TokenUsage {
            total: Some(output_total),
            text: Some(completion_tokens),
            reasoning: Some(reasoning_tokens),
            ..Default::default()
        },
        raw: None,
    }
}

// ── Tool preparation ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PreparedTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub tool_warnings: Vec<Warning>,
}

pub fn prepare_tools(tools: &Option<Vec<Tool>>, tool_choice: Option<&ToolChoice>) -> PreparedTools {
    let non_empty = tools.as_ref().filter(|t| !t.is_empty());
    let mut tool_warnings: Vec<Warning> = Vec::new();

    let tools_opt = match non_empty {
        None => None,
        Some(tools) => {
            let xai_tools: Vec<Value> = tools
                .iter()
                .filter_map(|t| match t {
                    Tool::Function(ft) => {
                        let mut func = json!({
                            "name": ft.name,
                            "parameters": remove_additional_properties_false(&ft.input_schema),
                        });
                        if let Some(ref desc) = ft.description {
                            func["description"] = json!(desc);
                        }
                        if let Some(strict) = ft.strict {
                            func["strict"] = json!(strict);
                        }
                        Some(json!({ "type": "function", "function": func }))
                    }
                    Tool::Provider(pt) => {
                        tool_warnings.push(Warning::Unsupported {
                            feature: format!("provider-defined tool {}", pt.name),
                            details: None,
                        });
                        None
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

pub fn remove_additional_properties_false(value: &Value) -> Value {
    match value {
        Value::Array(arr) => {
            Value::Array(arr.iter().map(remove_additional_properties_false).collect())
        }
        Value::Object(obj) => {
            let mut result = serde_json::Map::new();
            for (key, val) in obj {
                if key == "additionalProperties" && val == &Value::Bool(false) {
                    continue;
                }
                result.insert(key.clone(), remove_additional_properties_false(val));
            }
            Value::Object(result)
        }
        other => other.clone(),
    }
}

// ── Provider options helpers ─────────────────────────────────────────────────

fn xai_option(options: &Option<HashMap<String, Value>>, key: &str) -> Option<Value> {
    options
        .as_ref()
        .and_then(|m| m.get("xai"))
        .and_then(|o| o.get(key))
        .cloned()
}

fn get_image_detail(provider_options: &Option<Value>) -> Option<Value> {
    provider_options
        .as_ref()
        .and_then(|po| po.get("xai"))
        .and_then(|o| o.get("imageDetail"))
        .cloned()
}

// ── Media type helpers ───────────────────────────────────────────────────────

fn get_top_level_media_type(media_type: &str) -> &str {
    media_type.split('/').next().unwrap_or("")
}

pub fn resolve_full_media_type(media_type: &str, b64_data: &str) -> String {
    let top_level = get_top_level_media_type(media_type);
    if top_level == "image" && media_type != "image" && !media_type.ends_with("/*") {
        return media_type.to_string();
    }
    if top_level == "image" {
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
        return "image/png".to_string();
    }
    media_type.to_string()
}

pub fn resolve_provider_reference(reference: &Value, provider: &str) -> Result<String, String> {
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

// ── Message conversion ───────────────────────────────────────────────────────

pub fn convert_to_xai_messages(
    prompt: &LanguageModelPrompt,
) -> Result<(Vec<Value>, Vec<Warning>), AiMuxError> {
    let mut messages = Vec::new();
    let warnings: Vec<Warning> = Vec::new();

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
                messages.push(json!({ "role": "system", "content": content }));
            }
            Role::User => {
                if msg.content.len() == 1
                    && let ContentPart::Text { text, .. } = &msg.content[0]
                {
                    messages.push(json!({ "role": "user", "content": text }));
                    continue;
                }
                let mut user_content = Vec::new();
                for (i, part) in msg.content.iter().enumerate() {
                    user_content.push(convert_user_part(part, i)?);
                }
                messages.push(json!({ "role": "user", "content": user_content }));
            }
            Role::Assistant => {
                let mut text = String::new();
                let mut tool_calls: Vec<Value> = Vec::new();
                for part in &msg.content {
                    match part {
                        ContentPart::Text { text: t, .. } => text.push_str(t),
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
                            tool_calls.push(json!({ "id": tool_call_id, "type": "function", "function": { "name": tool_name, "arguments": arguments } }));
                        }
                        _ => {}
                    }
                }
                let mut msg_obj = json!({ "role": "assistant", "content": text });
                if !tool_calls.is_empty() {
                    msg_obj["tool_calls"] = json!(tool_calls);
                }
                messages.push(msg_obj);
            }
            Role::Tool => {
                for part in &msg.content {
                    if let ContentPart::ToolResult {
                        tool_call_id,
                        result,
                        ..
                    } = part
                    {
                        let content = match result {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        messages.push(json!({ "role": "tool", "tool_call_id": tool_call_id, "content": content }));
                    }
                }
            }
        }
    }
    Ok((messages, warnings))
}

fn convert_user_part(part: &ContentPart, _index: usize) -> Result<Value, AiMuxError> {
    match part {
        ContentPart::Text { text, .. } => Ok(json!({ "type": "text", "text": text })),
        ContentPart::Image {
            image,
            media_type,
            provider_options,
        } => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(image);
            convert_image_part(media_type, Some(&b64), None, provider_options)
        }
        ContentPart::File {
            data,
            media_type,
            provider_options,
            ..
        } => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(data);
            convert_image_part(media_type, Some(&b64), None, provider_options)
        }
        ContentPart::FileBase64 {
            data,
            media_type,
            provider_options,
            ..
        } => convert_image_part(media_type, Some(data), None, provider_options),
        ContentPart::FileUrl {
            url,
            media_type,
            provider_options,
        } => convert_image_part(media_type, None, Some(url), provider_options),
        ContentPart::FileReference {
            media_type,
            reference,
            provider_options,
            ..
        } => {
            let _ = media_type;
            let _ = provider_options;
            let file_id = resolve_provider_reference(reference, "xai")
                .map_err(AiMuxError::InvalidArgument)?;
            Ok(json!({ "type": "file", "file": { "file_id": file_id } }))
        }
        _ => Ok(Value::Null),
    }
}

fn convert_image_part(
    media_type: &str,
    b64_data: Option<&str>,
    url: Option<&str>,
    provider_options: &Option<Value>,
) -> Result<Value, AiMuxError> {
    let top_level = get_top_level_media_type(media_type);
    if top_level != "image" {
        return Err(AiMuxError::Unsupported(format!(
            "file part media type {}",
            media_type
        )));
    }
    let image_url = if let Some(url_str) = url {
        json!({ "url": url_str })
    } else if let Some(b64) = b64_data {
        let full_mt = resolve_full_media_type(media_type, b64);
        json!({ "url": format!("data:{};base64,{}", full_mt, b64) })
    } else {
        json!({ "url": "" })
    };
    let mut image_url_obj = image_url;
    if let Some(detail) = get_image_detail(provider_options) {
        image_url_obj["detail"] = detail;
    }
    Ok(json!({ "type": "image_url", "image_url": image_url_obj }))
}

// ── Request body ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RequestBodyResult {
    pub body: Value,
    pub warnings: Vec<Warning>,
}

pub fn build_request_body_with_warnings(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<RequestBodyResult, AiMuxError> {
    let mut warnings: Vec<Warning> = Vec::new();
    let xai_opts = &options.provider_options;

    if options.top_k.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "topK".to_string(),
            details: None,
        });
    }
    if options.frequency_penalty.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "frequencyPenalty".to_string(),
            details: None,
        });
    }
    if options.presence_penalty.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "presencePenalty".to_string(),
            details: None,
        });
    }
    if options.stop_sequences.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "stopSequences".to_string(),
            details: None,
        });
    }

    let (messages, message_warnings) = convert_to_xai_messages(&options.prompt)?;
    warnings.extend(message_warnings);

    let prepared = prepare_tools(&options.tools, Some(&options.tool_choice));
    for tw in &prepared.tool_warnings {
        warnings.push(tw.clone());
    }

    let mut reasoning_effort: Option<String> = xai_option(xai_opts, "reasoningEffort")
        .map(|v| v.as_str().map(|s| s.to_string()).unwrap_or(v.to_string()));

    if reasoning_effort.is_none() && is_custom_reasoning(&options.reasoning) {
        let reasoning = options.reasoning.unwrap();
        if !supports_reasoning_effort(model_id) {
            warnings.push(Warning::Unsupported {
                feature: "reasoning".to_string(),
                details: Some(format!(
                    "reasoning \"{}\" is not supported by this model.",
                    reasoning
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

    let mut body = json!({ "model": model_id, "messages": messages });
    if stream {
        body["stream"] = json!(true);
        body["stream_options"] = json!({ "include_usage": true });
    }
    if let Some(max_tokens) = options.max_output_tokens {
        body["max_completion_tokens"] = json!(max_tokens);
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

    let logprobs = xai_option(xai_opts, "logprobs")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let top_logprobs = xai_option(xai_opts, "topLogprobs");
    if logprobs || top_logprobs.is_some() {
        body["logprobs"] = json!(true);
    }
    if let Some(tlp) = top_logprobs {
        body["top_logprobs"] = tlp;
    }

    if let Some(effort) = &reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }
    if let Some(val) = xai_option(xai_opts, "parallel_function_calling") {
        body["parallel_function_calling"] = val;
    }

    if let Some(ref rf) = options.response_format {
        match rf {
            ResponseFormat::Text => {}
            ResponseFormat::Json {
                schema,
                name,
                description: _,
            } => {
                if schema.is_some() {
                    let mut schema_obj = json!({});
                    if let Some(s) = schema {
                        schema_obj["schema"] = s.clone();
                    }
                    schema_obj["name"] =
                        json!(name.clone().unwrap_or_else(|| "response".to_string()));
                    schema_obj["strict"] = json!(true);
                    body["response_format"] =
                        json!({ "type": "json_schema", "json_schema": schema_obj });
                } else {
                    body["response_format"] = json!({ "type": "json_object" });
                }
            }
        }
    }

    if let Some(sp) = xai_option(xai_opts, "searchParameters") {
        body["search_parameters"] = convert_search_parameters(&sp);
    }
    if let Some(tools) = &prepared.tools {
        body["tools"] = json!(tools);
    }
    if let Some(tc) = prepared.tool_choice {
        body["tool_choice"] = tc;
    }

    Ok(RequestBodyResult { body, warnings })
}

fn convert_search_parameters(sp: &Value) -> Value {
    let mut result = json!({});
    if let Some(v) = sp.get("mode") {
        result["mode"] = v.clone();
    }
    if let Some(v) = sp.get("returnCitations") {
        result["return_citations"] = v.clone();
    }
    if let Some(v) = sp.get("fromDate") {
        result["from_date"] = v.clone();
    }
    if let Some(v) = sp.get("toDate") {
        result["to_date"] = v.clone();
    }
    if let Some(v) = sp.get("maxSearchResults") {
        result["max_search_results"] = v.clone();
    }
    if let Some(sources) = sp.get("sources").and_then(|s| s.as_array()) {
        result["sources"] = json!(
            sources
                .iter()
                .map(convert_search_source)
                .collect::<Vec<_>>()
        );
    }
    result
}

fn convert_search_source(source: &Value) -> Value {
    let mut result = json!({});
    if let Some(t) = source.get("type") {
        result["type"] = t.clone();
    }
    let st = source.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match st {
        "web" => {
            if let Some(v) = source.get("country") {
                result["country"] = v.clone();
            }
            if let Some(v) = source.get("excludedWebsites") {
                result["excluded_websites"] = v.clone();
            }
            if let Some(v) = source.get("allowedWebsites") {
                result["allowed_websites"] = v.clone();
            }
            if let Some(v) = source.get("safeSearch") {
                result["safe_search"] = v.clone();
            }
        }
        "x" => {
            if let Some(v) = source.get("excludedXHandles") {
                result["excluded_x_handles"] = v.clone();
            }
            if let Some(v) = source.get("includedXHandles") {
                result["included_x_handles"] = v.clone();
            } else if let Some(v) = source.get("xHandles") {
                result["included_x_handles"] = v.clone();
            }
            if let Some(v) = source.get("postFavoriteCount") {
                result["post_favorite_count"] = v.clone();
            }
            if let Some(v) = source.get("postViewCount") {
                result["post_view_count"] = v.clone();
            }
        }
        "news" => {
            if let Some(v) = source.get("country") {
                result["country"] = v.clone();
            }
            if let Some(v) = source.get("excludedWebsites") {
                result["excluded_websites"] = v.clone();
            }
            if let Some(v) = source.get("safeSearch") {
                result["safe_search"] = v.clone();
            }
        }
        "rss" => {
            if let Some(v) = source.get("links") {
                result["links"] = v.clone();
            }
        }
        _ => {}
    }
    result
}
