//! Conversion between `LanguageModelPrompt` and Mistral API format.
//!
//! Mirrors the TS `convert-to-mistral-chat-messages.ts`,
//! `mistral-prepare-tools.ts`, and `map-mistral-finish-reason.ts`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, LanguageModelMessage, LanguageModelPrompt, TextPart, ToolCallPart, ToolPart,
    ToolResultOutput, ToolResultPart, UserPart,
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
            let has_reasoning = content.iter().any(|part| matches!(part, AssistantPart::Reasoning(_)));
            let mut content_parts = Vec::new();
            for part in content {
                match part {
                    AssistantPart::Text(part) => content_parts.push(json!({ "type": "text", "text": part.text })),
                    AssistantPart::Reasoning(part) => content_parts.push(json!({ "type": "thinking", "thinking": [{ "type": "text", "text": part.text }], "closed": true })),
                    AssistantPart::ToolCall(_) => {},
                    _ => return Err(AiMuxError::UnsupportedFunctionality("assistant content part".to_string())),
                }
            }
            let has_tool_calls = content
                .iter()
                .any(|p| matches!(p, AssistantPart::ToolCall(_)));

            let mut msg_json = json!({ "role": "assistant", "content": text });
            if has_reasoning { msg_json["content"] = json!(content_parts); }

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
                            let arguments = input.to_string();
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
            .filter_map(|part| {
                let ToolPart::ToolResult(ToolResultPart { tool_call_id, tool_name, output, .. }) = part else { return None; };
                Some(json!({ "role": "tool", "name": tool_name, "tool_call_id": tool_call_id, "content": tool_result_to_content(output) }))
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

fn tool_result_to_content(output: &ToolResultOutput) -> Value {
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
        ToolResultOutput::Content { value } => {
            crate::openai::convert::tool_result_content_value(value).to_string()
        }
    })
}

fn convert_part_to_mistral(part: &UserPart) -> Result<Value, AiMuxError> {
    Ok(match part {
        UserPart::Text(TextPart { text, .. }) => json!({ "type": "text", "text": text }),
        UserPart::File(file) => {
            use base64::Engine;
            let is_image = file.media_type.split('/').next() == Some("image");
            let url = match &file.data {
                FileData::Data { data } => {
                    let b64 = match data {
                        FileBytes::Binary(bytes) => {
                            base64::engine::general_purpose::STANDARD.encode(bytes)
                        }
                        FileBytes::Base64(data) => data.clone(),
                    };
                    let media_type = crate::google::convert::tool_file_media_type(file)?;
                    if !is_image && media_type != "application/pdf" {
                        return Err(AiMuxError::UnsupportedFunctionality(
                            "Only images and PDF file parts are supported".to_string(),
                        ));
                    }
                    format!("data:{media_type};base64,{b64}")
                }
                FileData::Url { url, .. } => {
                    if !is_image && file.media_type != "application/pdf" {
                        return Err(AiMuxError::UnsupportedFunctionality(
                            "Only images and PDF file parts are supported".to_string(),
                        ));
                    }
                    url.clone()
                }
                FileData::Reference { .. } => {
                    return Err(AiMuxError::UnsupportedFunctionality(
                        "file parts with provider references".to_string(),
                    ));
                }
                FileData::Text { .. } => {
                    return Err(AiMuxError::UnsupportedFunctionality(
                        "text file parts".to_string(),
                    ));
                }
            };
            if is_image {
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
    let provider_options = options
        .provider_options
        .as_ref()
        .and_then(|namespaces| namespaces.get("mistral"))
        .map(|namespace| {
            crate::openai::convert::parse_option_fields(namespace, "mistral", |key, value| {
                let valid = match key {
                    "safePrompt" | "structuredOutputs" | "strictJsonSchema"
                    | "parallelToolCalls" => value.is_boolean(),
                    "documentImageLimit" | "documentPageLimit" => value.is_number(),
                    "promptCacheKey" => value.is_string(),
                    "reasoningEffort" => value
                        .as_str()
                        .is_some_and(|value| matches!(value, "high" | "none")),
                    _ => return None,
                };
                Some(if valid { Ok(value.clone()) } else { Err(()) })
            })
        })
        .transpose()?
        .unwrap_or_default();
    let messages = convert_prompt_to_mistral_messages(&options.prompt)?;

    let mut body = json!({
        "model": model_id,
        "messages": messages,
    });

    if stream {
        body["stream"] = json!(true);
    }

    if let Some(safe_prompt) = provider_options.get("safePrompt") {
        body["safe_prompt"] = safe_prompt.clone();
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
    let prepared = prepare_tools(&function_tools, options.tool_choice.as_ref());
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
