//! Conversion between `LanguageModelPrompt` and Mistral API format.
//!
//! Mirrors the TS `convert-to-mistral-chat-messages.ts`,
//! `mistral-prepare-tools.ts`, and `map-mistral-finish-reason.ts`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, LanguageModelMessage, LanguageModelPrompt, TextPart, ToolCallPart, ToolPart,
    ToolResultPart, UserPart,
};
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::{FinishReason, FinishReasonUnified};
use serde_json::{Value, json};

// ── Prepared tools ──────────────────────────────────────────────────────────

/// The result of preparing tools for a Mistral request body.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub tool_warnings: Vec<String>,
}

/// Prepare `FunctionTool`s into the Mistral `tools` / `tool_choice` JSON shape.
///
/// Key difference from OpenAI: `ToolChoice::Required` maps to `"any"` (not
/// `"required"`), and `ToolChoice::Tool` filters the tools array and also uses
/// `"any"`.
#[must_use]
pub fn prepare_tools(
    tools: &Option<Vec<FunctionTool>>,
    tool_choice: Option<&ToolChoice>,
) -> PreparedTools {
    let non_empty = tools.as_ref().filter(|t| !t.is_empty());

    let tool_warnings: Vec<String> = Vec::new();

    let tools_opt = match non_empty {
        None => None,
        Some(tools) => {
            let mistral_tools: Vec<Value> = tools
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
            Some(mistral_tools)
        }
    };

    // Handle ToolChoice::Tool which needs to filter the tools array.
    if let (Some(tools), Some(ToolChoice::Tool { tool_name })) = (&tools_opt, tool_choice) {
        let filtered: Vec<Value> = tools
            .iter()
            .filter(|t| t["function"]["name"].as_str() == Some(tool_name.as_str()))
            .cloned()
            .collect();
        return PreparedTools {
            tools: Some(filtered),
            tool_choice: Some(json!("any")),
            tool_warnings,
        };
    }

    let tool_choice_opt = match (&tools_opt, tool_choice) {
        (None, _) => None,
        (Some(_), None) => None,
        (Some(_), Some(tc)) => match tc {
            ToolChoice::Auto => Some(json!("auto")),
            ToolChoice::None => Some(json!("none")),
            ToolChoice::Required => Some(json!("any")),
            ToolChoice::Tool { .. } => Some(json!("any")),
        },
    };

    PreparedTools {
        tools: tools_opt,
        tool_choice: tool_choice_opt,
        tool_warnings,
    }
}

// ── Message conversion ──────────────────────────────────────────────────────

/// Convert a `LanguageModelPrompt` to Mistral `messages` array.
///
/// Differences from OpenAI:
/// - System content is a plain string.
/// - User content is always an array of typed parts.
/// - Assistant content is a plain string; `prefix: true` is set on the last
///   message if it is an assistant message (continuation mode).
/// - Tool messages include `tool_call_id` (no `name` — the Rust data model
///   does not carry the tool name on `ToolResult` parts).
///
/// # Errors
/// Returns an error for unsupported text file data.
pub fn convert_prompt_to_mistral_messages(
    prompt: &LanguageModelPrompt,
) -> Result<Vec<Value>, AiMuxError> {
    let mut result = Vec::new();
    let last_idx = prompt.len().saturating_sub(1);
    for (i, msg) in prompt.iter().enumerate() {
        let is_last = i == last_idx;
        for value in convert_message_to_mistral(msg, is_last)? {
            result.push(value);
        }
    }
    Ok(result)
}

fn convert_message_to_mistral(
    msg: &LanguageModelMessage,
    is_last: bool,
) -> Result<Vec<Value>, AiMuxError> {
    Ok(match msg {
        LanguageModelMessage::System { content, .. } => {
            vec![json!({ "role": "system", "content": content })]
        }
        LanguageModelMessage::User { content, .. } => {
            let parts: Vec<Value> = content
                .iter()
                .map(convert_part_to_mistral)
                .collect::<Result<_, _>>()?;
            vec![json!({ "role": "user", "content": parts })]
        }
        LanguageModelMessage::Assistant { content, .. } => {
            let text = join_text_parts(content);
            let has_tool_calls = content
                .iter()
                .any(|p| matches!(p, AssistantPart::ToolCall(_)));

            let mut msg_json = json!({ "role": "assistant", "content": text });

            if has_tool_calls {
                let tool_calls: Vec<Value> = content
                    .iter()
                    .filter_map(|p| match p {
                        AssistantPart::ToolCall(ToolCallPart {
                            tool_call_id,
                            tool_name,
                            input,
                            ..
                        }) => {
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
                msg_json["tool_calls"] = json!(tool_calls);
            }

            if is_last {
                msg_json["prefix"] = json!(true);
            }
            vec![msg_json]
        }
        LanguageModelMessage::Tool { content, .. } => content
            .iter()
            .map(|part| {
                let ToolPart::ToolResult(ToolResultPart {
                    tool_call_id,
                    result,
                    ..
                }) = part;
                let content = tool_result_to_content(result);
                json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": content,
                })
            })
            .collect(),
    })
}

fn join_text_parts(content: &[AssistantPart]) -> String {
    content
        .iter()
        .filter_map(|p| match p {
            AssistantPart::Text(TextPart { text, .. }) => Some(text.as_str()),
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

fn convert_part_to_mistral(part: &UserPart) -> Result<Value, AiMuxError> {
    Ok(match part {
        UserPart::Text(TextPart { text, .. }) => json!({ "type": "text", "text": text }),
        UserPart::File(file) => {
            use base64::Engine;
            let url = match &file.data {
                FileData::Data { data } => {
                    let b64 = match data {
                        FileBytes::Binary(bytes) => {
                            base64::engine::general_purpose::STANDARD.encode(bytes)
                        }
                        FileBytes::Base64(data) => data.clone(),
                    };
                    format!("data:{};base64,{}", file.media_type, b64)
                }
                FileData::Url { url } => url.clone(),
                FileData::Reference { .. } => return Ok(Value::Null),
                FileData::Text { .. } => {
                    return Err(AiMuxError::UnsupportedFunctionality(
                        "text file parts".to_string(),
                    ));
                }
            };
            if file.media_type.split('/').next() == Some("image") {
                json!({ "type": "image_url", "image_url": url })
            } else {
                json!({ "type": "document_url", "document_url": url })
            }
        }
    })
}

// ── Request body ────────────────────────────────────────────────────────────

/// Convert `CallOptions` to a Mistral request body.
///
/// # Errors
/// Returns an error for unsupported text file data.
pub fn build_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<Value, AiMuxError> {
    let messages = convert_prompt_to_mistral_messages(&options.prompt)?;

    let mut body = json!({
        "model": model_id,
        "messages": messages,
    });

    if stream {
        body["stream"] = json!(true);
    }

    if let Some(max_tokens) = options.max_output_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(temp) = options.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(top_p) = options.top_p {
        body["top_p"] = json!(top_p);
    }
    if let Some(ref stop) = options.stop_sequences {
        body["stop"] = json!(stop);
    }
    if let Some(seed) = options.seed {
        body["random_seed"] = json!(seed);
    }
    if let Some(fp) = options.frequency_penalty {
        body["frequency_penalty"] = json!(fp);
    }
    if let Some(pp) = options.presence_penalty {
        body["presence_penalty"] = json!(pp);
    }

    // Response format — Mistral uses json_schema / json_object.
    if let Some(ref rf) = options.response_format {
        match rf {
            ResponseFormat::Text => {}
            ResponseFormat::Json {
                schema,
                name,
                description,
            } => {
                if schema.is_some() {
                    let mut schema_obj = json!({});
                    if let Some(s) = schema {
                        schema_obj["schema"] = s.clone();
                    }
                    schema_obj["name"] =
                        json!(name.clone().unwrap_or_else(|| "response".to_string()));
                    if let Some(d) = description {
                        schema_obj["description"] = json!(d);
                    }
                    schema_obj["strict"] = json!(false);
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

    // Tools (delegated to `prepare_tools`).
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

    Ok(body)
}

/// Parse Mistral finish reason string into `FinishReason`.
///
/// Differences from OpenAI: `model_length` is also mapped to `Length`.
#[must_use]
pub fn parse_finish_reason(s: &str) -> FinishReason {
    let unified = match s {
        "stop" => FinishReasonUnified::Stop,
        "length" | "model_length" => FinishReasonUnified::Length,
        "tool_calls" => FinishReasonUnified::ToolCalls,
        "content_filter" => FinishReasonUnified::ContentFilter,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(s.to_string()),
    }
}
