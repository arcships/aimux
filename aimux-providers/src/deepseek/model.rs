//! The DeepSeek chat language model (`deepseek-chat-language-model.ts`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Warning};
use aimux_provider_utils::{
    StreamingToolCallDelta, StreamingToolCallFunction, StreamingToolCallTracker,
};

use super::convert::convert_to_deepseek_chat_messages;
use super::finish_reason::map_deepseek_finish_reason;
use super::is_v4_model::is_deepseek_v4_model;
use super::options::parse_chat_options;
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
}

/// A DeepSeek chat model.
pub struct DeepSeekChatLanguageModel {
    model_id: String,
    config: DeepSeekChatConfig,
}

/// A request body and the warnings raised while building it.
#[derive(Debug, Clone)]
pub struct RequestBodyResult {
    /// The JSON body, before the provider's `transform_request_body`.
    pub body: Value,
    /// Warnings raised while building it.
    pub warnings: Vec<Warning>,
}

/// A fallback id for a tool call the server sent without one.
fn generate_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!(
        "call_{nanos:x}{:x}",
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
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
    error: &Value,
    url: &str,
    request_body_values: Value,
    response_headers: HashMap<String, String>,
) -> AiMuxError {
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
        error,
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

    /// The JSON body a call would send, before the provider's
    /// `transform_request_body`, with the warnings raised while building it
    /// (`getArgs`, plus the stream fields). Nothing is read from the
    /// environment and nothing is sent: a key is not resolved here.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a malformed provider option, `InvalidPrompt` or
    /// `UnsupportedFunctionality` for a prompt DeepSeek cannot take.
    pub fn request_body(
        &self,
        options: &CallOptions,
        stream: bool,
    ) -> Result<RequestBodyResult, AiMuxError> {
        let provider_options_name = self.provider_options_name();
        let deepseek_options =
            parse_chat_options(options.provider_options.as_ref(), provider_options_name)?;

        let converted = convert_to_deepseek_chat_messages(
            &options.prompt,
            &self.model_id,
            provider_options_name,
            self.config.supports_assistant_prefix_completion,
        )?;
        let mut warnings = converted.warnings;

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
        if options.seed.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "seed".to_string(),
                details: None,
            });
        }

        let prepared = prepare_tools(options.tools.as_ref(), &options.tool_choice);

        let is_thinking_disabled = deepseek_options
            .thinking
            .as_ref()
            .and_then(|thinking| thinking.get("type"))
            .and_then(Value::as_str)
            == Some("disabled");
        let is_thinking_enabled = !is_thinking_disabled
            && (deepseek_options.thinking.is_some()
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

        let reasoning_effort = deepseek_options.reasoning_effort.clone().or_else(|| {
            options
                .reasoning
                .filter(|effort| effort.is_custom())
                .map(|effort| effort.to_string())
        });

        let mut body = deepseek_options.extra;
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
        if let Some(top_k) = options.top_k {
            body.insert("top_k".into(), json!(top_k));
        }
        if let Some(ResponseFormat::Json {
            schema,
            name,
            description,
        }) = &options.response_format
        {
            body.insert(
                "response_format".into(),
                match schema {
                    Some(schema) => {
                        let mut json_schema = Map::new();
                        json_schema.insert("schema".into(), schema.clone());
                        json_schema.insert(
                            "strict".into(),
                            json!(deepseek_options.strict_json_schema.unwrap_or(true)),
                        );
                        json_schema
                            .insert("name".into(), json!(name.as_deref().unwrap_or("response")));
                        if let Some(description) = description {
                            json_schema.insert("description".into(), json!(description));
                        }
                        json!({ "type": "json_schema", "json_schema": json_schema })
                    }
                    None => json!({ "type": "json_object" }),
                },
            );
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
        if let Some(thinking) = deepseek_options.thinking {
            body.insert("thinking".into(), thinking);
        }
        if let Some(user_id) = deepseek_options.user_id {
            body.insert("user_id".into(), json!(user_id));
        }
        if let Some(reasoning_effort) = reasoning_effort
            && !is_thinking_disabled
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

/// `{ "<provider>": { systemFingerprint?, logprobs? } }`.
fn provider_metadata(
    provider_options_name: &str,
    system_fingerprint: Option<&str>,
    logprobs: Option<Value>,
) -> Value {
    let mut metadata = Map::new();
    if let Some(logprobs) = logprobs {
        metadata.insert("logprobs".into(), logprobs);
    }
    if let Some(system_fingerprint) = system_fingerprint {
        metadata.insert("systemFingerprint".into(), json!(system_fingerprint));
    }
    json!({ provider_options_name: metadata })
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
        let body = exchange.transform_body(built.body);
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

        let mut content = Vec::new();

        // reasoning content (before text):
        if let Some(text) = choice
            .message
            .reasoning_content
            .or(choice.message.reasoning)
            .filter(|text| !text.is_empty())
        {
            content.push(GenerateContent::Reasoning {
                text,
                provider_metadata: None,
            });
        }

        // tool calls:
        for call in choice.message.tool_calls.into_iter().flatten() {
            content.push(GenerateContent::ToolCall {
                tool_call_id: call
                    .id
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(generate_id),
                tool_name: call.function.name,
                input: call.function.arguments,
                provider_executed: None,
                dynamic: None,
                thought_signature: None,
                provider_metadata: None,
            });
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
            provider_metadata: Some(provider_metadata(
                self.provider_options_name(),
                data.system_fingerprint.as_deref(),
                choice.logprobs.filter(|logprobs| !logprobs.is_null()),
            )),
            response: ResponseMetadata {
                id: data.id,
                timestamp: timestamp(data.created),
                model_id: data.model,
            },
            request_body: Some(body),
            response_headers: Some(response_headers),
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
        let body = exchange.transform_body(built.body);
        let endpoint = exchange.url("/chat/completions");
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(endpoint.clone(), options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            aimux_provider_utils::create_standard_json_error_response_handler(),
        )
        .await?;
        let response_headers = resp.response_headers;
        let mut sse_stream = resp.value;

        // Only the first event is checked before returning the stream: an
        // error reported immediately stays inside Core's operation-retry
        // boundary. A normal event is chained back and never consumed.
        let first_event = match sse_stream.next().await {
            Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
            first => first,
        };
        if let Some(Ok(event)) = &first_event
            && let Some(error) = event.get("error")
        {
            return Err(deepseek_stream_error(
                error,
                &endpoint,
                body.clone(),
                response_headers.clone(),
            ));
        }

        let provider_options_name = self.provider_options_name().to_string();
        let emit_raw_chunks = options.include_raw_chunks == Some(true);
        let stream_error_body = body.clone();
        let stream_response_headers = response_headers.clone();

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings });

            let mut tool_calls = StreamingToolCallTracker::new().with_generate_id(generate_id);
            let mut finish_reason = FinishReason { unified: FinishReasonUnified::Other, raw: None };
            let mut usage: Option<Value> = None;
            let mut system_fingerprint: Option<String> = None;
            let mut is_first_chunk = true;
            let mut is_active_reasoning = false;
            let mut is_active_text = false;
            let mut content_logprobs: Vec<Value> = Vec::new();
            let mut reasoning_logprobs: Vec<Value> = Vec::new();

            let mut events = futures::stream::iter(first_event).chain(sse_stream);
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
                if let Some(error) = parsed.get("error") {
                    finish_reason = FinishReason { unified: FinishReasonUnified::Error, raw: None };
                    yield Ok(StreamPart::Error {
                        error: deepseek_stream_error(
                            error,
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
                    yield Ok(StreamPart::ResponseMetadata {
                        id: chunk.id.clone(),
                        timestamp: timestamp(chunk.created),
                        model_id: chunk.model.clone(),
                    });
                }

                if let Some(chunk_usage) = chunk.usage.filter(|usage| !usage.is_null()) {
                    usage = Some(chunk_usage);
                }

                // The fingerprint is repeated on stream chunks; keep the
                // latest non-null value in case it changes during the response.
                if chunk.system_fingerprint.is_some() {
                    system_fingerprint = chunk.system_fingerprint;
                }

                let Some(choice) = chunk.choices.into_iter().next() else {
                    continue;
                };

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

                // enqueue reasoning before text deltas:
                if let Some(reasoning) = delta
                    .reasoning_content
                    .or(delta.reasoning)
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

                    for tool_call in deltas {
                        let delta = StreamingToolCallDelta {
                            index: Some(tool_call.index),
                            id: tool_call.id,
                            r#type: None,
                            function: Some(StreamingToolCallFunction {
                                name: tool_call.function.name,
                                arguments: tool_call.function.arguments,
                            }),
                            extra: Value::Null,
                        };
                        match tool_calls.process_delta(&delta) {
                            Ok(parts) => {
                                for part in parts {
                                    yield Ok(part);
                                }
                            }
                            Err(error) => {
                                yield Err(error.into());
                                return;
                            }
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
            for part in tool_calls.flush() {
                yield Ok(part);
            }

            let mut logprobs = Map::new();
            if !content_logprobs.is_empty() {
                logprobs.insert("content".into(), Value::Array(content_logprobs));
            }
            if !reasoning_logprobs.is_empty() {
                logprobs.insert("reasoning_content".into(), Value::Array(reasoning_logprobs));
            }
            yield Ok(StreamPart::Finish {
                finish_reason,
                usage: convert_deepseek_usage(usage.as_ref()),
                provider_metadata: Some(provider_metadata(
                    &provider_options_name,
                    system_fingerprint.as_deref(),
                    (!logprobs.is_empty()).then_some(Value::Object(logprobs)),
                )),
            });
        };

        Ok(StreamResult {
            stream: Box::pin(stream),
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }
}
