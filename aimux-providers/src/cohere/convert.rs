//! Conversion between `LanguageModelPrompt` and Cohere API format.
//!
//! Mirrors the TS `convert-to-cohere-chat-prompt.ts`,
//! `cohere-prepare-tools.ts`, and `map-cohere-finish-reason.ts`.

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::tool::Tool;
use aimux_core::types::{FinishReason, FinishReasonUnified, ReasoningEffort, Warning};
use serde_json::{Value, json};

// ── Prepared tools ──────────────────────────────────────────────────────────

/// The result of preparing tools for a Cohere request body.
#[derive(Debug, Clone)]
pub struct PreparedTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub tool_warnings: Vec<Warning>,
}

/// Prepare `Tool`s into the Cohere `tools` / `tool_choice` JSON shape.
///
/// Mirrors the TS `cohere-prepare-tools.ts`. Provider-defined tools are not
/// supported by Cohere and emit an `Unsupported` warning (the tool is dropped).
///
/// Key differences from OpenAI:
/// - `ToolChoice::Auto` → omit `tool_choice` (Cohere default)
/// - `ToolChoice::None` → `"NONE"`
/// - `ToolChoice::Required` → `"REQUIRED"`
/// - `ToolChoice::Tool` → filter tools and use `"REQUIRED"`
pub fn prepare_tools(tools: &Option<Vec<Tool>>, tool_choice: Option<&ToolChoice>) -> PreparedTools {
    let non_empty = tools.as_ref().filter(|t| !t.is_empty());

    let mut tool_warnings: Vec<Warning> = Vec::new();

    let tools_opt = match non_empty {
        None => None,
        Some(tools) => {
            let mut cohere_tools: Vec<Value> = Vec::new();
            for t in tools {
                match t {
                    Tool::Function(ft) => {
                        let mut func = json!({
                            "name": ft.name,
                            "parameters": ft.input_schema,
                        });
                        if let Some(ref desc) = ft.description {
                            func["description"] = json!(desc);
                        }
                        cohere_tools.push(json!({ "type": "function", "function": func }));
                    }
                    Tool::Provider(pt) => {
                        // Provider-defined tools are not supported by Cohere.
                        tool_warnings.push(Warning::Unsupported {
                            feature: format!("provider-defined tool {}", pt.id),
                            details: None,
                        });
                    }
                }
            }
            if cohere_tools.is_empty() {
                None
            } else {
                Some(cohere_tools)
            }
        }
    };

    // Handle ToolChoice::Tool which filters tools.
    if let (Some(tools), Some(ToolChoice::Tool { tool_name })) = (&tools_opt, tool_choice) {
        let filtered: Vec<Value> = tools
            .iter()
            .filter(|t| t["function"]["name"].as_str() == Some(tool_name.as_str()))
            .cloned()
            .collect();
        return PreparedTools {
            tools: Some(filtered),
            tool_choice: Some(json!("REQUIRED")),
            tool_warnings,
        };
    }

    let tool_choice_opt = match (&tools_opt, tool_choice) {
        (None, _) => None,
        (Some(_), None) => None,
        (Some(_), Some(tc)) => match tc {
            ToolChoice::Auto => None,
            ToolChoice::None => Some(json!("NONE")),
            ToolChoice::Required => Some(json!("REQUIRED")),
            ToolChoice::Tool { .. } => Some(json!("REQUIRED")),
        },
    };

    PreparedTools {
        tools: tools_opt,
        tool_choice: tool_choice_opt,
        tool_warnings,
    }
}

// ── Prompt conversion ───────────────────────────────────────────────────────

/// The result of converting a prompt to Cohere format.
pub struct ConvertedPrompt {
    pub messages: Vec<Value>,
    pub documents: Vec<Value>,
}

/// Convert a `LanguageModelPrompt` to Cohere `messages` + `documents`.
///
/// Differences from OpenAI:
/// - System content is a plain string.
/// - User content (no images) is a plain string; with images, an array of parts.
/// - Non-image file parts are extracted as `documents` (RAG), not in the message.
/// - Assistant content is a plain string; with tool calls, `content` is omitted.
/// - Tool messages are flat `{role:"tool", content, tool_call_id}`.
pub fn convert_prompt_to_cohere(prompt: &LanguageModelPrompt) -> ConvertedPrompt {
    let mut messages = Vec::new();
    let mut documents = Vec::new();

    for msg in prompt {
        match msg.role {
            Role::System => {
                let text = join_text_parts(&msg.content);
                messages.push(json!({ "role": "system", "content": text }));
            }
            Role::User => {
                let mut parts: Vec<Value> = Vec::new();
                let mut has_image = false;

                for part in &msg.content {
                    match part {
                        ContentPart::Text { text, .. } => {
                            if !text.is_empty() {
                                parts.push(json!({ "type": "text", "text": text }));
                            }
                        }
                        ContentPart::Image {
                            image, media_type, ..
                        } => {
                            has_image = true;
                            use base64::Engine;
                            let b64 = base64::engine::general_purpose::STANDARD.encode(image);
                            parts.push(json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": format!("data:{};base64,{}", media_type, b64),
                                }
                            }));
                        }
                        ContentPart::File {
                            data,
                            media_type,
                            filename,
                            ..
                        } => {
                            let top_level = media_type.split('/').next().unwrap_or("");
                            if top_level == "image" {
                                has_image = true;
                                use base64::Engine;
                                let b64 = base64::engine::general_purpose::STANDARD.encode(data);
                                parts.push(json!({
                                    "type": "image_url",
                                    "image_url": {
                                        "url": format!("data:{};base64,{}", media_type, b64),
                                    }
                                }));
                            } else {
                                // Non-image files become RAG documents.
                                // Mirrors TS: `documents.push({ data: { text, title:
                                // part.filename } })` — `title` is only present when a
                                // filename is supplied.
                                let text = String::from_utf8_lossy(data).to_string();
                                let mut doc_data = json!({ "text": text });
                                if let Some(fname) = filename {
                                    doc_data["title"] = json!(fname);
                                }
                                documents.push(json!({ "data": doc_data }));
                            }
                        }
                        _ => {}
                    }
                }

                if has_image {
                    messages.push(json!({ "role": "user", "content": parts }));
                } else {
                    let text: String = parts
                        .iter()
                        .filter_map(|p| {
                            if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                                p.get("text")
                                    .and_then(|t| t.as_str())
                                    .map(|s| s.to_string())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    messages.push(json!({ "role": "user", "content": text }));
                }
            }
            Role::Assistant => {
                let text = join_text_parts(&msg.content);
                let has_tool_calls = msg
                    .content
                    .iter()
                    .any(|p| matches!(p, ContentPart::ToolCall { .. }));

                if has_tool_calls {
                    let tool_calls: Vec<Value> = msg
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
                                    "id": tool_call_id,
                                    "type": "function",
                                    "function": {
                                        "name": tool_name,
                                        "arguments": arguments,
                                    }
                                }))
                            }
                            _ => None,
                        })
                        .collect();
                    messages.push(json!({
                        "role": "assistant",
                        "tool_calls": tool_calls,
                    }));
                } else {
                    messages.push(json!({ "role": "assistant", "content": text }));
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
                        let content = tool_result_to_content(result);
                        messages.push(json!({
                            "role": "tool",
                            "content": content,
                            "tool_call_id": tool_call_id,
                        }));
                    }
                }
            }
        }
    }

    ConvertedPrompt {
        messages,
        documents,
    }
}

fn join_text_parts(content: &[ContentPart]) -> String {
    content
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn tool_result_to_content(output: &Value) -> Value {
    match output {
        Value::String(s) => Value::String(s.clone()),
        other => Value::String(other.to_string()),
    }
}

// ── Request body ────────────────────────────────────────────────────────────

/// Result of building a Cohere request body, including warnings collected from
/// tool preparation (provider-defined tools are unsupported).
#[derive(Debug, Clone)]
pub struct RequestBodyResult {
    pub body: Value,
    pub warnings: Vec<Warning>,
}

/// Convert `CallOptions` to a Cohere request body.
///
/// Mirrors the TS `getArgs` in `cohere-chat-language-model.ts`: assembles the
/// model id, messages, documents, sampling settings, response format, tools,
/// and the `thinking` config resolved from `reasoning` / provider options.
pub fn build_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> RequestBodyResult {
    let converted = convert_prompt_to_cohere(&options.prompt);
    let mut warnings: Vec<Warning> = Vec::new();

    let mut body = json!({
        "model": model_id,
        "messages": converted.messages,
    });

    if stream {
        body["stream"] = json!(true);
    }

    if !converted.documents.is_empty() {
        body["documents"] = json!(converted.documents);
    }

    if let Some(max_tokens) = options.max_output_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(temp) = options.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(top_p) = options.top_p {
        body["p"] = json!(top_p);
    }
    if let Some(top_k) = options.top_k {
        body["k"] = json!(top_k);
    }
    if let Some(seed) = options.seed {
        body["seed"] = json!(seed);
    }
    if let Some(ref stop) = options.stop_sequences {
        body["stop_sequences"] = json!(stop);
    }
    if let Some(fp) = options.frequency_penalty {
        body["frequency_penalty"] = json!(fp);
    }
    if let Some(pp) = options.presence_penalty {
        body["presence_penalty"] = json!(pp);
    }

    // Response format.
    if let Some(ref rf) = options.response_format {
        match rf {
            ResponseFormat::Text => {}
            ResponseFormat::Json { schema, .. } => {
                body["response_format"] = json!({
                    "type": "json_object",
                    "json_schema": schema,
                });
            }
        }
    }

    // Tools.
    let prepared = prepare_tools(&options.tools, Some(&options.tool_choice));
    warnings.extend(prepared.tool_warnings);
    if let Some(tools) = prepared.tools {
        body["tools"] = json!(tools);
        if let Some(tc) = prepared.tool_choice {
            body["tool_choice"] = tc;
        }
    }

    // Reasoning / thinking.
    if let Some(thinking) = resolve_cohere_thinking(options.reasoning, &options.provider_options) {
        body["thinking"] = thinking;
    }

    RequestBodyResult { body, warnings }
}

// ── Reasoning / thinking ────────────────────────────────────────────────────

/// Default token budget used when reasoning is enabled without an explicit
/// `max_output_tokens`. Mirrors the TS `maxOutputTokens: 32768`.
const DEFAULT_REASONING_MAX_TOKENS: u32 = 32768;

/// Default reasoning budget percentages (of the max output token budget),
/// mirroring the TS `mapReasoningToProviderBudget`.
fn reasoning_budget_percentage(reasoning: ReasoningEffort) -> Option<f64> {
    match reasoning {
        ReasoningEffort::Minimal => Some(0.02),
        ReasoningEffort::Low => Some(0.10),
        ReasoningEffort::Medium => Some(0.30),
        ReasoningEffort::High => Some(0.60),
        ReasoningEffort::Xhigh => Some(0.90),
        ReasoningEffort::ProviderDefault | ReasoningEffort::None => None,
    }
}

/// Resolve the Cohere `thinking` config from the top-level `reasoning` option
/// and `providerOptions.cohere.thinking`.
///
/// Mirrors the TS `resolveCohereThinking`:
/// - `providerOptions.cohere.thinking` wins over top-level `reasoning`.
/// - `reasoning: None` (not specified) → no `thinking` field.
/// - `reasoning: "none"` → `{ type: "disabled" }`.
/// - other reasoning levels → `{ type: "enabled", token_budget: <n> }`.
pub fn resolve_cohere_thinking(
    reasoning: Option<ReasoningEffort>,
    provider_options: &Option<std::collections::HashMap<String, Value>>,
) -> Option<Value> {
    // Provider options take precedence.
    if let Some(po) = provider_options
        && let Some(cohere) = po.get("cohere")
        && let Some(thinking) = cohere.get("thinking")
    {
        let t_type = thinking
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("enabled")
            .to_string();
        let mut obj = json!({ "type": t_type });
        if let Some(budget) = thinking.get("tokenBudget").and_then(|v| v.as_u64()) {
            obj["token_budget"] = json!(budget);
        }
        return Some(obj);
    }

    let reasoning = reasoning?;
    if reasoning == ReasoningEffort::ProviderDefault {
        return None;
    }
    if reasoning == ReasoningEffort::None {
        return Some(json!({ "type": "disabled" }));
    }

    let pct = reasoning_budget_percentage(reasoning)?;
    let max_tokens = DEFAULT_REASONING_MAX_TOKENS;
    let raw = (max_tokens as f64 * pct).round() as u32;
    let budget = max_tokens.min(1024.max(raw));
    Some(json!({ "type": "enabled", "token_budget": budget }))
}

/// Parse Cohere finish reason string into `FinishReason`.
pub fn parse_finish_reason(s: &str) -> FinishReason {
    let unified = match s {
        "COMPLETE" | "STOP_SEQUENCE" => FinishReasonUnified::Stop,
        "MAX_TOKENS" => FinishReasonUnified::Length,
        "ERROR" => FinishReasonUnified::Error,
        "TOOL_CALL" => FinishReasonUnified::ToolCalls,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(s.to_string()),
    }
}
