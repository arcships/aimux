//! The DeepSeek chat language model (`deepseek-chat-language-model.ts`).

use aimux_core::tool::RawToolCall;
use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, StreamResult};
use aimux_core::shared::{RequestInfo, ResponseInfo, StreamResponseInfo, provider_namespace};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, ReasoningEffort, ResponseMetadata, Warning,
};
use aimux_provider_utils::{StreamingToolCallDelta, StreamingToolCallTracker, generate_id};

use super::convert::convert_to_deepseek_chat_messages;
use super::finish_reason::map_deepseek_finish_reason;
use super::is_v4_model::is_deepseek_v4_model;
use super::options::{ProviderReasoningEffort, ThinkingType, parse_chat_options};
use super::prepare_tools::prepare_tools;
use super::types::{ChatChunk, ChatResponse};
use super::usage::convert_deepseek_usage;
use crate::shared::EndpointConfig;

/// What a DeepSeek chat model needs to talk to the API (`DeepSeekChatConfig`).
pub(crate) struct DeepSeekChatConfig {
    /// Identity (`"{name}.chat"`), endpoint, headers, transport.
    pub(crate) endpoint: EndpointConfig,
    /// Whether the base URL is the beta one, which accepts assistant prefix
    /// completion.
    pub(crate) supports_assistant_prefix_completion: bool,
    /// Whether the base URL is the beta one, which accepts strict tool calls.
    pub(crate) supports_strict_tool_calls: bool,
}

/// A DeepSeek chat model.
pub struct DeepSeekChatLanguageModel {
    model_id: String,
    config: DeepSeekChatConfig,
}

/// A request body and the warnings raised while building it.
struct RequestBodyResult {
    /// The JSON body.
    body: Value,
    /// Warnings raised while building it.
    warnings: Vec<Warning>,
}

fn timestamp(created: Option<f64>) -> Option<String> {
    created
        .and_then(|secs| chrono::DateTime::from_timestamp(secs as i64, 0))
        .map(|dt| dt.to_rfc3339())
}

/// The HTTP status a stream error code stands for: a number or a three-digit
/// string in `400..=599`.
fn http_status_code(code: Option<&Value>) -> Option<u16> {
    let status = match code? {
        Value::String(code) if code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit()) => {
            code.parse::<u64>().ok()?
        }
        Value::Number(code) => code.as_u64()?,
        _ => return None,
    };
    u16::try_from(status)
        .ok()
        .filter(|status| (400..=599).contains(status))
}

/// The HTTP status a named error kind stands for.
fn status_of_error_kind(kind: &str) -> Option<u16> {
    Some(match kind {
        "rate_limit_exceeded" | "rate_limit_error" => 429,
        "server_error" | "api_error" | "internal_server_error" => 500,
        "overloaded_error" | "service_unavailable" => 503,
        "timeout" | "timeout_error" => 504,
        "authentication_error" | "invalid_api_key" => 401,
        "permission_error" => 403,
        "not_found_error" | "model_not_found" => 404,
        "bad_request" | "context_length_exceeded" | "invalid_request_error" => 400,
        _ => return None,
    })
}

/// The error of an error event inside a stream (`createDeepSeekStreamError`).
fn deepseek_stream_error(
    event: &Value,
    url: &str,
    request_body_values: Value,
    response_headers: HashMap<String, String>,
) -> AiMuxError {
    let error = &event["error"];
    let code = error.get("code").filter(|code| !code.is_null());
    let kind = error.get("type").and_then(Value::as_str);
    let names = [code.and_then(Value::as_str), kind];
    let quota_exhausted = names.contains(&Some("insufficient_quota"));
    let status_code = if quota_exhausted {
        Some(429)
    } else {
        http_status_code(code)
            .or_else(|| names.into_iter().flatten().find_map(status_of_error_kind))
    };
    let provider_code = code
        .or_else(|| error.get("type"))
        .and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        });
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("DeepSeek stream failed before any output was generated")
        .to_owned();
    let mut error = aimux_provider_utils::stream_error_api_call(
        message,
        provider_code,
        status_code,
        event,
        url,
        request_body_values,
        response_headers,
    );
    if quota_exhausted && let AiMuxError::ApiCall(api_error) = &mut error {
        api_error.is_retryable = false;
    }
    error
}

impl DeepSeekChatLanguageModel {
    pub(crate) fn from_config(model_id: String, config: DeepSeekChatConfig) -> Self {
        Self { model_id, config }
    }

    /// The providerOptions namespace: the provider name.
    fn provider_options_name(&self) -> &str {
        self.config
            .endpoint
            .provider
            .split('.')
            .next()
            .unwrap_or_default()
            .trim()
    }

    /// The JSON body a call would send, with the warnings raised while
    /// building it (`getArgs`, plus the stream fields).
    fn request_body(
        &self,
        options: &CallOptions,
        stream: bool,
    ) -> Result<RequestBodyResult, AiMuxError> {
        let provider_options_name = self.provider_options_name();
        let deepseek_options =
            parse_chat_options(options.provider_options.as_ref(), provider_options_name)?;

        let converted = convert_to_deepseek_chat_messages(
            &options.prompt,
            options.response_format.as_ref(),
            &self.model_id,
            provider_options_name,
            self.config.supports_assistant_prefix_completion,
        )?;
        let mut warnings = converted.warnings;

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
        for (value, setting) in [
            (options.frequency_penalty, "frequencyPenalty"),
            (options.presence_penalty, "presencePenalty"),
        ] {
            if value.is_some() {
                warnings.push(Warning::Deprecated {
                    setting: setting.to_string(),
                    message: format!(
                        "{setting} is deprecated by DeepSeek and has been omitted. Remove {setting} from the request."
                    ),
                });
            }
        }

        let prepared = prepare_tools(
            options.tools.as_ref(),
            options.tool_choice.as_ref(),
            self.config.supports_strict_tool_calls,
        )?;

        let thinking_type = deepseek_options
            .thinking
            .as_ref()
            .and_then(|thinking| thinking.kind);
        if thinking_type == Some(ThinkingType::Adaptive) {
            warnings.push(Warning::Compatibility {
                feature: "thinking.type".to_string(),
                details: Some(
                    "thinking.type \"adaptive\" is not a canonical DeepSeek value. mapped to \"enabled\"."
                        .to_string(),
                ),
            });
        }

        let reasoning = options.reasoning.filter(|effort| effort.is_custom());
        let thinking = match (thinking_type, reasoning) {
            (Some(ThinkingType::Disabled), _) => Some("disabled"),
            (Some(_), _) => Some("enabled"),
            (None, Some(ReasoningEffort::None)) => Some("disabled"),
            (None, Some(_)) => Some("enabled"),
            (None, None) => None,
        };

        let is_thinking_enabled = thinking != Some("disabled")
            && (thinking.is_some()
                || self.model_id == "deepseek-reasoner"
                || is_deepseek_v4_model(&self.model_id));

        for (value, feature) in [
            (options.temperature, "temperature"),
            (options.top_p, "topP"),
        ] {
            if is_thinking_enabled && value.is_some() {
                warnings.push(Warning::Unsupported {
                    feature: feature.to_string(),
                    details: Some(format!(
                        "{feature} has no effect when DeepSeek thinking is enabled. Set providerOptions.deepseek.thinking.type to 'disabled' to use {feature}."
                    )),
                });
            }
        }

        let reasoning_effort = match (deepseek_options.reasoning_effort, reasoning) {
            (Some(effort), _) => {
                let mapped = match effort {
                    ProviderReasoningEffort::Medium => "high",
                    ProviderReasoningEffort::Xhigh => "max",
                    ProviderReasoningEffort::Low => "low",
                    ProviderReasoningEffort::High => "high",
                    ProviderReasoningEffort::Max => "max",
                };
                let given = match effort {
                    ProviderReasoningEffort::Low => "low",
                    ProviderReasoningEffort::Medium => "medium",
                    ProviderReasoningEffort::High => "high",
                    ProviderReasoningEffort::Xhigh => "xhigh",
                    ProviderReasoningEffort::Max => "max",
                };
                if mapped != given {
                    warnings.push(Warning::Compatibility {
                        feature: "reasoningEffort".to_string(),
                        details: Some(format!(
                            "reasoningEffort \"{given}\" is not a canonical DeepSeek value. mapped to \"{mapped}\"."
                        )),
                    });
                }
                Some(mapped)
            }
            (None, Some(ReasoningEffort::None) | None) => None,
            (None, Some(reasoning)) => {
                let mapped = match reasoning {
                    ReasoningEffort::Minimal | ReasoningEffort::Low => "low",
                    ReasoningEffort::Medium | ReasoningEffort::High => "high",
                    _ => "max",
                };
                if mapped != reasoning.to_string() {
                    warnings.push(Warning::Compatibility {
                        feature: "reasoning".to_string(),
                        details: Some(format!(
                            "reasoning \"{reasoning}\" is not directly supported by this model. mapped to effort \"{mapped}\"."
                        )),
                    });
                }
                Some(mapped)
            }
        };

        let mut body = Map::new();
        body.insert("model".into(), json!(self.model_id));
        if deepseek_options.logprobs == Some(true) || deepseek_options.top_logprobs.is_some() {
            body.insert("logprobs".into(), json!(true));
        }
        if let Some(top_logprobs) = deepseek_options.top_logprobs {
            body.insert("top_logprobs".into(), json!(top_logprobs));
        }
        if let Some(max_tokens) = options.max_output_tokens {
            body.insert("max_tokens".into(), json!(max_tokens));
        }
        if !is_thinking_enabled {
            for (key, value) in [
                ("temperature", options.temperature),
                ("top_p", options.top_p),
            ] {
                if let Some(value) = value {
                    body.insert(key.into(), json!(value));
                }
            }
        }
        if matches!(options.response_format, Some(ResponseFormat::Json { .. })) {
            body.insert("response_format".into(), json!({ "type": "json_object" }));
        }
        if let Some(stop) = &options.stop_sequences {
            body.insert("stop".into(), json!(stop));
        }
        body.insert("messages".into(), Value::Array(converted.messages));
        if let Some(tools) = prepared.tools {
            body.insert("tools".into(), Value::Array(tools));
        }
        if let Some(tool_choice) = prepared.tool_choice {
            body.insert("tool_choice".into(), tool_choice);
        }
        if let Some(thinking) = thinking {
            body.insert("thinking".into(), json!({ "type": thinking }));
        }
        if let Some(user_id) = deepseek_options.user_id {
            body.insert("user_id".into(), json!(user_id));
        }
        if let Some(reasoning_effort) = reasoning_effort
            && thinking != Some("disabled")
        {
            body.insert("reasoning_effort".into(), json!(reasoning_effort));
        }
        if stream {
            body.insert("stream".into(), json!(true));
            body.insert("stream_options".into(), json!({ "include_usage": true }));
        }

        warnings.extend(prepared.tool_warnings);
        Ok(RequestBodyResult {
            body: Value::Object(body),
            warnings,
        })
    }
}

#[async_trait]
impl LanguageModel for DeepSeekChatLanguageModel {
    /// `"{name}.chat"`.
    fn provider(&self) -> &str {
        &self.config.endpoint.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn supported_urls(&self) -> SupportedUrls {
        (self.config.endpoint.supported_urls)(&self.model_id)
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let exchange = self
            .config
            .endpoint
            .exchange(options.headers.as_ref())
            .await?;
        let built = self.request_body(options, false)?;
        let body = built.body;
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/chat/completions"), options),
            body.clone(),
            aimux_provider_utils::create_json_response_handler::<ChatResponse>(),
            aimux_provider_utils::create_standard_json_error_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let raw = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

        let choice = data.choices.into_iter().next().ok_or_else(|| {
            AiMuxError::InvalidResponseData("Response did not contain any choices.".to_string())
        })?;

        let mut metadata = Map::new();
        let usage_value = raw.get("usage").filter(|usage| !usage.is_null());
        for (key, name) in [
            ("prompt_cache_hit_tokens", "promptCacheHitTokens"),
            ("prompt_cache_miss_tokens", "promptCacheMissTokens"),
        ] {
            if let Some(value) = usage_value.and_then(|usage| usage.get(key)) {
                metadata.insert(name.into(), value.clone());
            }
        }
        if let Some(object) = &data.object {
            metadata.insert("responseObject".into(), json!(object));
        }
        if let Some(index) = choice.index {
            metadata.insert("choiceIndex".into(), json!(index));
        }
        if let Some(role) = &choice.message.role {
            metadata.insert("messageRole".into(), json!(role));
        }
        if let Some(tool_calls) = &choice.message.tool_calls {
            let types: Vec<&String> = tool_calls
                .iter()
                .filter_map(|call| call.r#type.as_ref())
                .collect();
            metadata.insert("toolCallTypes".into(), json!(types));
        }
        if let Some(logprobs) = choice.logprobs.filter(|logprobs| !logprobs.is_null()) {
            metadata.insert("logprobs".into(), logprobs);
        }
        if let Some(system_fingerprint) = &data.system_fingerprint {
            metadata.insert("systemFingerprint".into(), json!(system_fingerprint));
        }

        let mut content = Vec::new();

        // reasoning content (before text):
        if let Some(text) = choice
            .message
            .reasoning_content
            .filter(|text| !text.is_empty())
        {
            content.push(GenerateContent::Reasoning(ReasoningOutput {
                text,
                provider_metadata: None,
            }));
        }

        // tool calls:
        for call in choice.message.tool_calls.into_iter().flatten() {
            content.push(GenerateContent::ToolCall(RawToolCall {
                tool_call_id: call
                    .id
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(generate_id),
                tool_name: call.function.name,
                input: call.function.arguments,
                provider_executed: None,
                dynamic: None,
                provider_metadata: None,
            }));
        }

        // text content:
        if let Some(text) = choice.message.content.filter(|text| !text.is_empty()) {
            content.push(GenerateContent::Text {
                text,
                provider_metadata: None,
            });
        }

        Ok(GenerateResult {
            content,
            finish_reason: FinishReason {
                unified: map_deepseek_finish_reason(choice.finish_reason.as_deref()),
                raw: choice.finish_reason,
            },
            usage: convert_deepseek_usage(raw.get("usage")),
            warnings: built.warnings,
            provider_metadata: Some(provider_namespace(
                self.provider_options_name(),
                Value::Object(metadata),
            )?),
            response: Some(ResponseInfo {
                id: data.id,
                timestamp: timestamp(data.created),
                model_id: data.model,
                headers: Some(response_headers),
                body: Some(raw),
            }),
            request: Some(RequestInfo { body: Some(body) }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let exchange = self
            .config
            .endpoint
            .exchange(options.headers.as_ref())
            .await?;
        let built = self.request_body(options, true)?;
        let warnings = built.warnings;
        let body = built.body;
        let endpoint = exchange.url("/chat/completions");
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(endpoint.clone(), options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            aimux_provider_utils::create_standard_json_error_response_handler(),
        )
        .await?;
        let response_headers = resp.response_headers;
        let sse_stream = resp.value;

        let provider_options_name = self.provider_options_name().to_string();
        let emit_raw_chunks = options.include_raw_chunks == Some(true);
        let stream_error_body = body.clone();
        let stream_response_headers = response_headers.clone();

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings });

            let mut tool_calls = StreamingToolCallTracker::new().with_generate_id(generate_id);
            let mut tool_parts = Vec::new();
            let mut finish_reason = FinishReason { unified: FinishReasonUnified::Other, raw: None };
            let mut usage: Option<Value> = None;
            let mut system_fingerprint: Option<String> = None;
            let mut is_first_chunk = true;
            let mut is_active_reasoning = false;
            let mut is_active_text = false;
            let mut response_object: Option<String> = None;
            let mut choice_index: Option<u32> = None;
            let mut message_role: Option<String> = None;
            let mut tool_call_types: BTreeMap<usize, String> = BTreeMap::new();
            let mut content_logprobs: Vec<Value> = Vec::new();
            let mut reasoning_logprobs: Vec<Value> = Vec::new();

            let mut events = sse_stream;
            while let Some(event) = events.next().await {
                let parsed = match event {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        let recoverable = error.is_recoverable_stream_error();
                        yield Err(error);
                        if !recoverable {
                            return;
                        }
                        continue;
                    }
                };

                // Emit the raw chunk if requested (before anything else).
                if emit_raw_chunks {
                    yield Ok(StreamPart::Raw { raw_value: parsed.clone() });
                }

                // handle error chunks:
                if parsed.get("error").is_some() {
                    finish_reason = FinishReason { unified: FinishReasonUnified::Error, raw: None };
                    yield Ok(StreamPart::Error {
                        error: deepseek_stream_error(
                            &parsed,
                            &endpoint,
                            stream_error_body.clone(),
                            stream_response_headers.clone(),
                        ),
                    });
                    continue;
                }

                // handle failed chunk validation:
                let chunk: ChatChunk = match serde_json::from_value(parsed) {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        finish_reason = FinishReason { unified: FinishReasonUnified::Error, raw: None };
                        yield Ok(StreamPart::Error { error: error.into() });
                        continue;
                    }
                };

                if is_first_chunk {
                    is_first_chunk = false;
                    yield Ok(StreamPart::ResponseMetadata(ResponseMetadata {
                        id: chunk.id.clone(),
                        timestamp: timestamp(chunk.created),
                        model_id: chunk.model.clone(),
                    }));
                }

                if let Some(chunk_usage) = chunk.usage.filter(|usage| !usage.is_null()) {
                    usage = Some(chunk_usage);
                }

                if chunk.object.is_some() {
                    response_object = chunk.object;
                }

                // The fingerprint is repeated on stream chunks; keep the
                // latest non-null value in case it changes during the response.
                if chunk.system_fingerprint.is_some() {
                    system_fingerprint = chunk.system_fingerprint;
                }

                let Some(choice) = chunk.choices.into_iter().next() else {
                    continue;
                };

                if choice.index.is_some() {
                    choice_index = choice.index;
                }

                if let Some(reason) = choice.finish_reason {
                    finish_reason = FinishReason {
                        unified: map_deepseek_finish_reason(Some(&reason)),
                        raw: Some(reason),
                    };
                }

                if let Some(logprobs) = &choice.logprobs {
                    for (key, sink) in [
                        ("content", &mut content_logprobs),
                        ("reasoning_content", &mut reasoning_logprobs),
                    ] {
                        if let Some(entries) = logprobs.get(key).and_then(Value::as_array) {
                            sink.extend(entries.iter().cloned());
                        }
                    }
                }

                let Some(delta) = choice.delta else {
                    continue;
                };

                if delta.role.is_some() {
                    message_role = delta.role;
                }

                // enqueue reasoning before text deltas:
                if let Some(reasoning) = delta
                    .reasoning_content
                    .filter(|reasoning| !reasoning.is_empty())
                {
                    if !is_active_reasoning {
                        yield Ok(StreamPart::ReasoningStart {
                            id: "reasoning-0".to_string(),
                            provider_metadata: None,
                        });
                        is_active_reasoning = true;
                    }
                    yield Ok(StreamPart::ReasoningDelta {
                        id: "reasoning-0".to_string(),
                        delta: reasoning,
                        provider_metadata: None,
                    });
                }

                if let Some(text) = delta.content.filter(|text| !text.is_empty()) {
                    if !is_active_text {
                        yield Ok(StreamPart::TextStart {
                            id: "txt-0".to_string(),
                            provider_metadata: None,
                        });
                        is_active_text = true;
                    }

                    // end reasoning when text starts:
                    if is_active_reasoning {
                        yield Ok(StreamPart::ReasoningEnd {
                            id: "reasoning-0".to_string(),
                            provider_metadata: None,
                        });
                        is_active_reasoning = false;
                    }

                    yield Ok(StreamPart::TextDelta {
                        id: "txt-0".to_string(),
                        delta: text,
                        provider_metadata: None,
                    });
                }

                if let Some(deltas) = delta.tool_calls.filter(|deltas| !deltas.is_empty()) {
                    // end reasoning when tool calls start:
                    if is_active_reasoning {
                        yield Ok(StreamPart::ReasoningEnd {
                            id: "reasoning-0".to_string(),
                            provider_metadata: None,
                        });
                        is_active_reasoning = false;
                    }

                    for tool_call in &deltas {
                        if let Some(kind) = &tool_call.r#type {
                            tool_call_types.insert(tool_call.index, kind.clone());
                        }
                        let delta = StreamingToolCallDelta {
                            index: Some(tool_call.index),
                            id: tool_call.id.as_deref(),
                            r#type: tool_call.r#type.as_deref(),
                            name: tool_call.function.name.as_deref(),
                            arguments: tool_call.function.arguments.as_deref(),
                            provider_metadata: None,
                        };
                        if let Err(error) = tool_calls.process(delta, &mut tool_parts) {
                            yield Err(error.into());
                            return;
                        }
                        for part in tool_parts.drain(..) {
                            yield Ok(part);
                        }
                    }
                }
            }

            if is_active_reasoning {
                yield Ok(StreamPart::ReasoningEnd {
                    id: "reasoning-0".to_string(),
                    provider_metadata: None,
                });
            }
            if is_active_text {
                yield Ok(StreamPart::TextEnd {
                    id: "txt-0".to_string(),
                    provider_metadata: None,
                });
            }
            tool_calls.finish(&mut tool_parts);
            for part in tool_parts.drain(..) {
                yield Ok(part);
            }

            let mut metadata = Map::new();
            for (key, name) in [
                ("prompt_cache_hit_tokens", "promptCacheHitTokens"),
                ("prompt_cache_miss_tokens", "promptCacheMissTokens"),
            ] {
                if let Some(value) = usage
                    .as_ref()
                    .and_then(|usage| usage.get(key))
                    .filter(|value| !value.is_null())
                {
                    metadata.insert(name.into(), value.clone());
                }
            }
            if let Some(object) = response_object {
                metadata.insert("responseObject".into(), json!(object));
            }
            if let Some(index) = choice_index {
                metadata.insert("choiceIndex".into(), json!(index));
            }
            if let Some(role) = message_role {
                metadata.insert("messageRole".into(), json!(role));
            }
            if !tool_call_types.is_empty() {
                metadata.insert(
                    "toolCallTypes".into(),
                    json!(tool_call_types.into_values().collect::<Vec<_>>()),
                );
            }
            let mut logprobs = Map::new();
            if !content_logprobs.is_empty() {
                logprobs.insert("content".into(), Value::Array(content_logprobs));
            }
            if !reasoning_logprobs.is_empty() {
                logprobs.insert("reasoning_content".into(), Value::Array(reasoning_logprobs));
            }
            if !logprobs.is_empty() {
                metadata.insert("logprobs".into(), Value::Object(logprobs));
            }
            if let Some(system_fingerprint) = system_fingerprint {
                metadata.insert("systemFingerprint".into(), json!(system_fingerprint));
            }
            yield Ok(StreamPart::Finish {
                finish_reason,
                usage: convert_deepseek_usage(usage.as_ref()),
                provider_metadata: Some(provider_namespace(&provider_options_name, Value::Object(metadata)).expect("metadata payload is an object")),
            });
        };

        Ok(StreamResult {
            stream: Box::pin(stream),
            request: Some(RequestInfo { body: Some(body) }),
            response: Some(StreamResponseInfo {
                headers: Some(response_headers),
            }),
        })
    }
}
