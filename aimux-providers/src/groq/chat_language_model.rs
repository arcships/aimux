//! The Groq chat language model (`{name}.chat`).
//!
//! Mirrors `groq-chat-language-model.ts`: Groq's own implementation of the
//! chat completions endpoint. Streamed tool calls go through the shared
//! [`ChatToolCalls`].

use aimux_core::tool::RawToolCall;
use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, StreamResult};
use aimux_core::shared::{RequestInfo, ResponseInfo, StreamResponseInfo};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, ReasoningEffort, ResponseMetadata, Warning};
use aimux_provider_utils::{Transformer, generate_id, pipe_through};

use crate::shared::EndpointConfig;
use crate::stream::{
    ChatToolCalls, Controller, Emit, FINISH_ERROR, FINISH_OTHER, StreamContext, ToolCallDelta,
};

use super::api_types::{GroqChatChunk, GroqChatResponse, GroqUsage};
use super::chat_language_model_options::{
    self as options, GroqLanguageModelChatOptions, parse_groq_options,
};
use super::convert_to_chat_messages::convert_to_groq_chat_messages;
use super::convert_usage::convert_groq_usage;
use super::error::{GroqErrorData, groq_failed_response_handler};
use super::map_finish_reason::map_groq_finish_reason;
use super::prepare_tools::prepare_tools;

/// A Groq chat language model.
pub struct GroqChatLanguageModel {
    model_id: String,
    config: EndpointConfig,
}

/// `getResponseMetadata`.
fn response_metadata(
    id: Option<String>,
    created: Option<u64>,
    model_id: Option<String>,
) -> ResponseMetadata {
    ResponseMetadata {
        id,
        timestamp: created.and_then(|secs| aimux_core::Timestamp::from_unix_seconds(secs as f64)),
        model_id,
    }
}

/// `getGroqStreamErrorMetadata`: the HTTP status an error type stands for.
/// Whether the error is retryable follows from that status.
fn groq_stream_error_status(error_type: &str) -> Option<u16> {
    match error_type {
        "rate_limit_error" => Some(429),
        "api_error" | "internal_server_error" | "server_error" => Some(500),
        "overloaded_error" | "service_unavailable" => Some(503),
        "timeout" | "timeout_error" => Some(504),
        "authentication_error" | "invalid_api_key" => Some(401),
        "permission_error" => Some(403),
        "not_found_error" | "model_not_found" => Some(404),
        "bad_request" | "context_length_exceeded" | "invalid_request_error" => Some(400),
        _ => None,
    }
}

/// `createGroqStreamError`.
fn create_groq_stream_error(
    data: &Value,
    url: &str,
    request_body_values: Value,
    response_headers: HashMap<String, String>,
) -> AiMuxError {
    match serde_json::from_value::<GroqErrorData>(data.clone()) {
        Ok(GroqErrorData { error }) => aimux_provider_utils::create_provider_stream_error(
            aimux_provider_utils::ProviderStreamError {
                status_code: groq_stream_error_status(&error.r#type),
                message: error.message,
                provider_code: Some(error.r#type),
                is_retryable: None,
                data: data.clone(),
            },
            data,
            url,
            request_body_values,
            response_headers,
        ),
        Err(error) => error.into(),
    }
}

impl GroqChatLanguageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }

    /// The request body and the warnings raised while building it (`getArgs`).
    fn get_args(
        &self,
        options: &CallOptions,
    ) -> Result<(Map<String, Value>, Vec<Warning>), AiMuxError> {
        let mut warnings = Vec::new();

        let groq_options: GroqLanguageModelChatOptions = parse_groq_options(options)?;

        let structured_outputs = groq_options.structured_outputs.unwrap_or(true);
        let strict_json_schema = groq_options.strict_json_schema.unwrap_or(true);

        if options.top_k.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "topK".to_string(),
                details: None,
            });
        }

        if let Some(ResponseFormat::Json {
            schema: Some(_), ..
        }) = &options.response_format
            && !structured_outputs
        {
            warnings.push(Warning::Unsupported {
                feature: "responseFormat".to_string(),
                details: Some(
                    "JSON response format schema is only supported with structuredOutputs"
                        .to_string(),
                ),
            });
        }

        let prepared = prepare_tools(options, &self.model_id);

        let mut reasoning_effort = groq_options.reasoning_effort;
        if reasoning_effort.is_none()
            && let Some(reasoning) = options.reasoning.filter(|effort| effort.is_custom())
        {
            reasoning_effort = match reasoning {
                ReasoningEffort::None if self.model_id == "qwen/qwen3.6-27b" => {
                    Some(options::ReasoningEffort::None)
                }
                ReasoningEffort::None => {
                    warnings.push(Warning::Unsupported {
                        feature: "reasoning".to_string(),
                        details: Some(format!(
                            "reasoning \"{reasoning}\" is not supported by this model."
                        )),
                    });
                    None
                }
                _ => {
                    // `mapReasoningToProviderEffort` with
                    // minimal -> low, low, medium, high, xhigh -> high.
                    let (mapped, name) = match reasoning {
                        ReasoningEffort::Minimal | ReasoningEffort::Low => {
                            (options::ReasoningEffort::Low, "low")
                        }
                        ReasoningEffort::Medium => (options::ReasoningEffort::Medium, "medium"),
                        _ => (options::ReasoningEffort::High, "high"),
                    };
                    if reasoning.to_string() != name {
                        warnings.push(Warning::Compatibility {
                            feature: "reasoning".to_string(),
                            details: Some(format!(
                                "reasoning \"{reasoning}\" is not directly supported by this model. mapped to effort \"{name}\"."
                            )),
                        });
                    }
                    Some(mapped)
                }
            };
        }

        let mut body = Map::new();
        // model id:
        body.insert("model".into(), json!(self.model_id));

        // model specific settings:
        if let Some(user) = &groq_options.user {
            body.insert("user".into(), json!(user));
        }
        if let Some(parallel) = groq_options.parallel_tool_calls {
            body.insert("parallel_tool_calls".into(), json!(parallel));
        }

        // standardized settings:
        if let Some(max_tokens) = options.max_output_tokens {
            body.insert("max_tokens".into(), json!(max_tokens));
        }
        for (key, value) in [
            ("temperature", options.temperature),
            ("top_p", options.top_p),
            ("frequency_penalty", options.frequency_penalty),
            ("presence_penalty", options.presence_penalty),
        ] {
            if let Some(value) = value {
                body.insert(key.into(), json!(value));
            }
        }
        if let Some(stop) = &options.stop_sequences {
            body.insert("stop".into(), json!(stop));
        }
        if let Some(seed) = options.seed {
            body.insert("seed".into(), json!(seed));
        }

        // response format:
        if let Some(ResponseFormat::Json {
            schema,
            name,
            description,
        }) = &options.response_format
        {
            let response_format = match schema {
                Some(schema) if structured_outputs => {
                    let mut json_schema = Map::new();
                    json_schema.insert("schema".into(), schema.clone());
                    json_schema.insert("strict".into(), json!(strict_json_schema));
                    json_schema.insert(
                        "name".into(),
                        json!(name.clone().unwrap_or_else(|| "response".to_string())),
                    );
                    if let Some(description) = description {
                        json_schema.insert("description".into(), json!(description));
                    }
                    json!({ "type": "json_schema", "json_schema": json_schema })
                }
                _ => json!({ "type": "json_object" }),
            };
            body.insert("response_format".into(), response_format);
        }

        // provider options:
        if let Some(format) = groq_options.reasoning_format {
            body.insert("reasoning_format".into(), json!(format));
        }
        if let Some(effort) = reasoning_effort {
            body.insert("reasoning_effort".into(), json!(effort));
        }
        if let Some(tier) = groq_options.service_tier {
            body.insert("service_tier".into(), json!(tier));
        }

        // messages:
        body.insert(
            "messages".into(),
            Value::Array(convert_to_groq_chat_messages(&options.prompt)?),
        );

        // tools:
        if let Some(tools) = prepared.tools {
            body.insert("tools".into(), Value::Array(tools));
        }
        if let Some(tool_choice) = prepared.tool_choice {
            body.insert("tool_choice".into(), tool_choice);
        }

        warnings.extend(prepared.tool_warnings);
        Ok((body, warnings))
    }
}

#[async_trait]
impl LanguageModel for GroqChatLanguageModel {
    /// `"{name}.chat"`.
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn supported_urls(&self) -> SupportedUrls {
        let http = regex::Regex::new(r"^https?://.*$").expect("static pattern");
        SupportedUrls(std::iter::once(("image/*".to_string(), vec![http])).collect())
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let (args, warnings) = self.get_args(options)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(Value::Object(args));

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/chat/completions"), options),
            body.clone(),
            aimux_provider_utils::create_json_response_handler::<GroqChatResponse>(),
            groq_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let response = resp.value;

        let choice = response.choices.into_iter().next().ok_or_else(|| {
            AiMuxError::invalid_response_data("Response did not contain any choices.")
        })?;

        let mut content = Vec::new();

        // text content:
        if let Some(text) = choice.message.content.filter(|text| !text.is_empty()) {
            content.push(GenerateContent::Text {
                text,
                provider_metadata: None,
            });
        }

        // reasoning:
        if let Some(text) = choice.message.reasoning.filter(|text| !text.is_empty()) {
            content.push(GenerateContent::Reasoning(ReasoningOutput {
                text,
                provider_metadata: None,
            }));
        }

        // tool calls:
        for tool_call in choice.message.tool_calls.into_iter().flatten() {
            content.push(GenerateContent::ToolCall(RawToolCall {
                tool_call_id: tool_call
                    .id
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(generate_id),
                tool_name: tool_call.function.name,
                input: tool_call.function.arguments,
                provider_executed: None,
                dynamic: None,
                provider_metadata: None,
            }));
        }

        Ok(GenerateResult {
            content,
            finish_reason: FinishReason {
                unified: map_groq_finish_reason(choice.finish_reason.as_deref()),
                raw: choice.finish_reason,
            },
            usage: convert_groq_usage(response.usage.as_ref()),
            warnings,
            provider_metadata: None,
            response: Some(ResponseInfo {
                headers: Some(response_headers),
                body: resp.raw_value,
                ..response_metadata(response.id, response.created, response.model).into()
            }),
            request: Some(RequestInfo { body: Some(body) }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let (mut args, warnings) = self.get_args(options)?;
        args.insert("stream".into(), json!(true));
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(Value::Object(args));
        let endpoint = exchange.url("/chat/completions");

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(endpoint.clone(), options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            groq_failed_response_handler(),
        )
        .await?;
        let response_headers = resp.response_headers;

        let stream = pipe_through(
            resp.value,
            GroqChatStream {
                ctx: StreamContext::new(options, warnings, endpoint, &body, &response_headers),
                ..GroqChatStream::default()
            },
        );

        Ok(StreamResult {
            stream: Box::pin(stream),
            request: Some(RequestInfo { body: Some(body) }),
            response: Some(StreamResponseInfo {
                headers: Some(response_headers),
            }),
        })
    }
}

/// The `TransformStream` of `GroqChatLanguageModel.doStream`.
#[derive(Default)]
struct GroqChatStream {
    ctx: StreamContext,
    tool_calls: ChatToolCalls,
    /// `other` until a chunk says otherwise.
    finish_reason: Option<FinishReason>,
    usage: Option<GroqUsage>,
    metadata_emitted: bool,
    is_active_text: bool,
    is_active_reasoning: bool,
}

/// A chat stream with an empty request context and tool-call ids from
/// `generate_id`, for tests that drive it chunk by chunk.
#[cfg(test)]
pub(crate) fn chat_test_stream(
    generate_id: std::sync::Arc<dyn Fn() -> String + Send + Sync>,
) -> impl Transformer<Input = Result<Value, AiMuxError>, Output = StreamPart> {
    GroqChatStream {
        tool_calls: ChatToolCalls::new(generate_id),
        ..GroqChatStream::default()
    }
}

impl GroqChatStream {
    fn end_active_reasoning(&mut self, controller: &mut Controller) {
        if self.is_active_reasoning {
            controller.reasoning_end("reasoning-0");
            self.is_active_reasoning = false;
        }
    }
}

impl Transformer for GroqChatStream {
    type Input = Result<Value, AiMuxError>;
    type Output = StreamPart;

    fn start(&mut self, controller: &mut Controller) {
        self.ctx.start(controller);
    }

    fn transform(&mut self, event: Result<Value, AiMuxError>, controller: &mut Controller) {
        let parsed = match event {
            Ok(parsed) => parsed,
            // a chunk that fails to parse is reported as an error part
            // (`chunk.success === false`); a transport failure ends the stream.
            Err(error) => {
                if !error.is_recoverable_stream_error() {
                    controller.error(error);
                    return;
                }
                self.ctx.raw(&Value::Null, controller);
                self.finish_reason = Some(FINISH_ERROR);
                controller.enqueue(StreamPart::Error { error });
                return;
            }
        };

        // Emit the raw chunk if requested (before anything else).
        self.ctx.raw(&parsed, controller);

        // handle error chunks:
        if parsed.get("error").is_some() {
            self.finish_reason = Some(FINISH_ERROR);
            controller.enqueue(StreamPart::Error {
                error: create_groq_stream_error(
                    &parsed,
                    &self.ctx.url,
                    self.ctx.request_body.clone(),
                    self.ctx.response_headers.clone(),
                ),
            });
            return;
        }

        // handle failed chunk parsing / validation:
        let value: GroqChatChunk = match serde_json::from_value(parsed) {
            Ok(value) => value,
            Err(error) => {
                self.finish_reason = Some(FINISH_ERROR);
                controller.enqueue(StreamPart::Error {
                    error: error.into(),
                });
                return;
            }
        };

        if !self.metadata_emitted {
            self.metadata_emitted = true;
            let metadata = response_metadata(value.id, value.created, value.model);
            controller.enqueue(StreamPart::ResponseMetadata(metadata));
        }

        if let Some(chunk_usage) = value.x_groq.and_then(|x_groq| x_groq.usage) {
            self.usage = Some(chunk_usage);
        }

        let Some(choice) = value.choices.into_iter().next() else {
            return;
        };

        if let Some(reason) = choice.finish_reason {
            self.finish_reason = Some(FinishReason {
                unified: map_groq_finish_reason(Some(&reason)),
                raw: Some(reason),
            });
        }

        let Some(delta) = choice.delta else {
            return;
        };

        if let Some(reasoning) = delta.reasoning.filter(|text| !text.is_empty()) {
            if !self.is_active_reasoning {
                controller.reasoning_start("reasoning-0");
                self.is_active_reasoning = true;
            }
            controller.reasoning_delta("reasoning-0", reasoning);
        }

        if let Some(text) = delta.content.filter(|text| !text.is_empty()) {
            // end the active reasoning block before text starts
            self.end_active_reasoning(controller);
            if !self.is_active_text {
                controller.text_start("txt-0");
                self.is_active_text = true;
            }
            controller.text_delta("txt-0", text);
        }

        if let Some(deltas) = delta.tool_calls.filter(|deltas| !deltas.is_empty()) {
            // end the active reasoning block before tool calls start
            self.end_active_reasoning(controller);
            for tool_call in &deltas {
                let delta = ToolCallDelta {
                    index: Some(tool_call.index),
                    id: tool_call.id.as_deref(),
                    name: tool_call.function.name.as_deref(),
                    arguments: tool_call.function.arguments.as_deref(),
                    // A call must start with `type: 'function'`.
                    invalid_type: tool_call.r#type.as_deref() != Some("function"),
                    provider_metadata: None,
                };
                if !self.tool_calls.delta(delta, controller) {
                    return;
                }
            }
        }
    }

    fn flush(mut self, controller: &mut Controller) {
        self.end_active_reasoning(controller);
        if self.is_active_text {
            controller.text_end("txt-0");
        }
        self.tool_calls.finish(controller);

        controller.enqueue(StreamPart::Finish {
            finish_reason: self.finish_reason.unwrap_or(FINISH_OTHER),
            usage: convert_groq_usage(self.usage.as_ref()),
            provider_metadata: None,
        });
    }
}
