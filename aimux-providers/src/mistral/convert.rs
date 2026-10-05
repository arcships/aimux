//! Conversion between `LanguageModelPrompt` and Mistral API format.
//!
//! Mirrors the TS `convert-to-mistral-chat-messages.ts`,
//! `mistral-prepare-tools.ts`, and `map-mistral-finish-reason.ts`.

use aimux_core::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultPart, UserPart,
};
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::{FinishReason, FinishReasonUnified, ReasoningEffort, Warning};
use aimux_provider_utils::{get_top_level_media_type, resolve_full_media_type};
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

/// Convert provider prompt parts using the upstream Mistral message format.
///
/// # Errors
/// Rejects file formats and assistant content unsupported by Mistral.
pub fn convert_prompt_to_mistral_messages(
    prompt: &LanguageModelPrompt,
) -> Result<Vec<Value>, AiMuxError> {
    let mut messages = Vec::new();
    for (index, message) in prompt.iter().enumerate() {
        match message {
            LanguageModelMessage::System { content, .. } => {
                messages.push(json!({"role":"system", "content": content}))
            }
            LanguageModelMessage::User { content, .. } => {
                let parts = content
                    .iter()
                    .map(convert_user_part)
                    .collect::<Result<Vec<_>, _>>()?;
                messages.push(json!({"role":"user", "content":parts}));
            }
            LanguageModelMessage::Assistant { content, .. } => {
                let mut text = String::new();
                let mut parts = Vec::new();
                let mut calls = Vec::new();
                let mut has_reasoning = false;
                for part in content {
                    match part {
                        AssistantPart::Text(part) => {
                            text.push_str(&part.text);
                            parts.push(json!({"type":"text", "text":part.text}));
                        }
                        AssistantPart::Reasoning(ReasoningPart { text, .. }) => {
                            has_reasoning = true;
                            parts.push(json!({"type":"thinking", "thinking":[{"type":"text", "text":text}], "closed":true}));
                        }
                        AssistantPart::ToolCall(ToolCallPart { tool_call_id, tool_name, input, .. }) => calls.push(json!({
                            "id":tool_call_id, "type":"function", "function":{"name":tool_name,"arguments":input.to_string()}
                        })),
                        _ => return Err(AiMuxError::UnsupportedFunctionality("Unsupported content type in assistant message".into())),
                    }
                }
                let content = if has_reasoning {
                    json!(parts)
                } else {
                    json!(text)
                };
                let mut value = json!({"role":"assistant", "content":content});
                if index + 1 == prompt.len() {
                    value["prefix"] = json!(true);
                }
                if !calls.is_empty() {
                    value["tool_calls"] = json!(calls);
                }
                messages.push(value);
            }
            LanguageModelMessage::Tool { content, .. } => {
                for part in content {
                    let ToolPart::ToolResult(ToolResultPart {
                        tool_call_id,
                        tool_name,
                        result,
                        ..
                    }) = part;
                    let name = tool_name.as_deref().or_else(|| {
                        prompt.iter().find_map(|message| {
                            let LanguageModelMessage::Assistant { content, .. } = message else {
                                return None;
                            };
                            content.iter().find_map(|part| match part {
                                AssistantPart::ToolCall(ToolCallPart {
                                    tool_call_id: id,
                                    tool_name,
                                    ..
                                }) if id == tool_call_id => Some(tool_name.as_str()),
                                _ => None,
                            })
                        })
                    });
                    let mut value = json!({"role":"tool", "tool_call_id":tool_call_id, "content":tool_result_to_content(result)});
                    if let Some(name) = name {
                        value["name"] = json!(name);
                    }
                    messages.push(value);
                }
            }
        }
    }
    Ok(messages)
}

fn tool_result_to_content(output: &Value) -> String {
    // The unified Rust content type also accepts the SDK's tagged output shape.
    match output.get("type").and_then(Value::as_str) {
        Some("text" | "error-text") => output["value"].as_str().unwrap_or_default().to_owned(),
        Some("execution-denied") => output
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("Tool call execution denied.")
            .to_owned(),
        Some("json" | "error-json" | "content") => output["value"].to_string(),
        _ => output
            .as_str()
            .map_or_else(|| output.to_string(), str::to_owned),
    }
}

fn convert_user_part(part: &UserPart) -> Result<Value, AiMuxError> {
    use base64::Engine;
    let file = match part {
        UserPart::Text(TextPart { text, .. }) => return Ok(json!({"type":"text", "text":text})),
        UserPart::File(file) => file,
    };
    let url = match &file.data {
        FileData::Reference { .. } => {
            return Err(AiMuxError::UnsupportedFunctionality(
                "file parts with provider references".into(),
            ));
        }
        FileData::Text { .. } => {
            return Err(AiMuxError::UnsupportedFunctionality(
                "text file parts".into(),
            ));
        }
        FileData::Data { data } => {
            let full = resolve_full_media_type(file)?;
            let data = match data {
                FileBytes::Binary(bytes) => base64::engine::general_purpose::STANDARD.encode(bytes),
                FileBytes::Base64(data) => data.clone(),
            };
            format!("data:{full};base64,{data}")
        }
        FileData::Url { url } => url.clone(),
    };
    if get_top_level_media_type(&file.media_type) == "image" {
        Ok(json!({"type":"image_url", "image_url":url}))
    } else {
        let full = if matches!(file.data, FileData::Url { .. }) {
            file.media_type.clone()
        } else {
            resolve_full_media_type(file)?
        };
        if full != "application/pdf" {
            return Err(AiMuxError::UnsupportedFunctionality(
                "Only images and PDF file parts are supported".into(),
            ));
        }
        Ok(json!({"type":"document_url", "document_url":url}))
    }
}

// ── Request body ────────────────────────────────────────────────────────────

/// Convert `CallOptions` to a Mistral request body.
///
/// # Errors
/// Rejects invalid provider options and unsupported prompt parts.
pub fn build_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<Value, AiMuxError> {
    let provider_options =
        super::options::validated_mistral_options(options.provider_options.as_ref())?;
    let mut messages = convert_prompt_to_mistral_messages(&options.prompt)?;
    if matches!(
        options.response_format,
        Some(ResponseFormat::Json { schema: None, .. })
    ) {
        let instruction = "You MUST answer with JSON.";
        if messages.first().is_some_and(|m| m["role"] == "system") {
            let text = messages[0]["content"].as_str().unwrap_or_default();
            messages[0]["content"] = json!(if text.is_empty() {
                instruction.to_owned()
            } else {
                format!("{text}\n\n{instruction}")
            });
        } else {
            messages.insert(0, json!({"role":"system", "content":instruction}));
        }
    }

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
                if schema.is_some()
                    && provider_options
                        .and_then(|o| o.get("structuredOutputs"))
                        .and_then(Value::as_bool)
                        .unwrap_or(true)
                {
                    let mut schema_obj = json!({});
                    if let Some(s) = schema {
                        schema_obj["schema"] = s.clone();
                    }
                    schema_obj["name"] =
                        json!(name.clone().unwrap_or_else(|| "response".to_string()));
                    if let Some(d) = description {
                        schema_obj["description"] = json!(d);
                    }
                    schema_obj["strict"] = json!(
                        provider_options
                            .and_then(|o| o.get("strictJsonSchema"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    );
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

    if let Some(provider_options) = provider_options {
        for (key, wire_key) in [
            ("safePrompt", "safe_prompt"),
            ("documentImageLimit", "document_image_limit"),
            ("documentPageLimit", "document_page_limit"),
            ("promptCacheKey", "prompt_cache_key"),
        ] {
            if let Some(value) = provider_options.get(key) {
                body[wire_key] = value.clone();
            }
        }
    }
    if supports_reasoning_effort(model_id) {
        let effort = provider_options
            .and_then(|o| o.get("reasoningEffort"))
            .cloned()
            .or_else(|| {
                options.reasoning.filter(|r| r.is_custom()).map(|r| {
                    json!(if r == ReasoningEffort::None {
                        "none"
                    } else {
                        "high"
                    })
                })
            });
        if let Some(effort) = effort {
            body["reasoning_effort"] = effort;
        }
    }

    // Preserve the empty tools array when the supplied tools are all unsupported.
    if let Some(tools) = options.tools.as_ref().filter(|tools| !tools.is_empty()) {
        let function_tools = Some(
            tools
                .iter()
                .filter_map(|t| match t {
                    Tool::Function(ft) => Some(ft.clone()),
                    Tool::Provider(_) => None,
                })
                .collect(),
        );
        let mut prepared = prepare_tools(&function_tools, Some(&options.tool_choice));
        if prepared.tools.is_none() {
            prepared.tools = Some(Vec::new());
            prepared.tool_choice = Some(json!(match options.tool_choice {
                ToolChoice::Auto => "auto",
                ToolChoice::None => "none",
                _ => "any",
            }));
        }
        body["tools"] = json!(prepared.tools);
        if let Some(choice) = prepared.tool_choice {
            body["tool_choice"] = choice;
        }
        if let Some(value) = provider_options.and_then(|o| o.get("parallelToolCalls")) {
            body["parallel_tool_calls"] = value.clone();
        }
    }
    Ok(body)
}

fn supports_reasoning_effort(model_id: &str) -> bool {
    matches!(
        model_id,
        "glm-5-2"
            | "labs-leanstral-1-5"
            | "labs-leanstral-1-5-1"
            | "magistral-medium-latest"
            | "magistral-small-latest"
            | "mistral-medium"
            | "mistral-medium-2604"
            | "mistral-medium-3"
            | "mistral-medium-3-5"
            | "mistral-medium-3.5"
            | "mistral-medium-latest"
            | "mistral-small-2603"
            | "mistral-small-latest"
            | "mistral-vibe-cli-fast"
            | "mistral-vibe-cli-latest"
            | "mistral-vibe-cli-with-tools"
            | "zai-glm-5-2"
    )
}

/// The warnings emitted while preparing an upstream Mistral request.
#[must_use]
pub fn request_warnings(options: &CallOptions, model_id: &str) -> Vec<Warning> {
    let mut warnings = Vec::new();
    if options.top_k.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "topK".into(),
            details: None,
        });
    }
    if let Some(reasoning) = options.reasoning.filter(|r| r.is_custom()) {
        if !supports_reasoning_effort(model_id) {
            warnings.push(Warning::Unsupported {
                feature: "reasoning".into(),
                details: Some("This model does not support reasoning configuration.".into()),
            });
        } else if !matches!(reasoning, ReasoningEffort::None | ReasoningEffort::High)
            && super::options::mistral_options(options.provider_options.as_ref())
                .and_then(|o| o.get("reasoningEffort"))
                .is_none()
        {
            warnings.push(Warning::Compatibility { feature:"reasoning".into(), details:Some(format!("reasoning \"{reasoning}\" is not directly supported by this model. mapped to effort \"high\".")) });
        }
    }
    for tool in options.tools.iter().flatten() {
        if let Tool::Provider(tool) = tool {
            warnings.push(Warning::Unsupported {
                feature: format!("provider-defined tool {}", tool.id),
                details: None,
            });
        }
    }
    warnings
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
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(s.to_string()),
    }
}
