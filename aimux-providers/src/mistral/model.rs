//! Mistral language model — implements `LanguageModel` trait.
//!
//! Mirrors the TS `mistral-chat-language-model.ts`. Key differences from the
//! OpenAI model:
//! - `content` in responses can be a string or an array of typed parts
//!   (text, thinking, image_url). Thinking parts are extracted as reasoning
//!   in streaming mode.
//! - Streamed tool calls are assembled by the shared `ChatToolCalls`.
//! - Usage supports `num_cached_tokens` / `prompt_tokens_details.cached_tokens`.
//! - Finish reasons include `model_length`.

use aimux_core::tool::RawToolCall;

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, ResponseMetadata, Usage};

use aimux_provider_utils::{Transformer, pipe_through};

use crate::shared::EndpointConfig;
use crate::stream::{ChatToolCalls, Controller, Emit, FINISH_OTHER, StreamContext, ToolCallDelta};

use super::convert::{build_request_body, parse_finish_reason, request_warnings};
use super::types::{ChatCompletionResponse, StreamChunk, UsageResponse};

/// A Mistral language model.
pub struct MistralModel {
    model_id: String,
    config: EndpointConfig,
    generate_id: Arc<dyn Fn() -> String + Send + Sync>,
}

impl MistralModel {
    pub(crate) fn from_config(
        model_id: String,
        config: EndpointConfig,
        generate_id_fn: Option<Arc<dyn Fn() -> String + Send + Sync>>,
    ) -> Self {
        Self {
            model_id,
            config,
            generate_id: generate_id_fn
                .unwrap_or_else(|| Arc::new(aimux_provider_utils::generate_id)),
        }
    }
}

// ── Usage conversion ─────────────────────────────────────────────────────────

/// Convert a Mistral `UsageResponse` into the core `Usage` type.
///
/// Mirrors the TS `convertMistralUsage`:
/// - `input.total = prompt_tokens`
/// - `input.noCache = prompt_tokens - cache_read_tokens`
/// - `input.cacheRead = cache_read_tokens` (or undefined when 0)
/// - `output.total = completion_tokens`
fn convert_usage(usage: &UsageResponse) -> Usage {
    let raw = usage.raw.clone();
    let usage = &usage.fields;
    let prompt_tokens = usage.prompt_tokens;
    let completion_tokens = usage.completion_tokens;

    let cache_read = usage
        .num_cached_tokens
        .or_else(|| {
            usage
                .prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens)
        })
        .or_else(|| {
            usage
                .prompt_token_details
                .as_ref()
                .and_then(|d| d.cached_tokens)
        })
        .unwrap_or(0);

    let no_cache = prompt_tokens - cache_read;

    Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: Some(prompt_tokens),
            no_cache: Some(no_cache),
            cache_read: if cache_read > 0 {
                Some(cache_read)
            } else {
                None
            },
            cache_write: None,
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: Some(completion_tokens),
            text: Some(completion_tokens),
            ..Default::default()
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: raw.as_object().cloned(),
    }
}

// ── Content extraction helpers ───────────────────────────────────────────────

/// Extract text content from a Mistral `content` field (string or array).
///
/// When the content is an array, only `text` parts are joined; `thinking`,
/// `image_url`, and `reference` parts are skipped.
fn extract_text_content(content: &Option<Value>) -> Option<String> {
    match content {
        None => None,
        Some(Value::String(s)) => {
            if s.is_empty() {
                None
            } else {
                Some(s.clone())
            }
        }
        Some(Value::Array(arr)) => {
            let text: String = arr
                .iter()
                .filter_map(|part| {
                    if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                        part.get("text")
                            .and_then(|t| t.as_str())
                            .map(std::string::ToString::to_string)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("");
            if text.is_empty() { None } else { Some(text) }
        }
        _ => None,
    }
}

/// Extract the text chunks of one thinking part.
fn extract_reasoning_part(part: &Value) -> Option<String> {
    if part.get("type").and_then(Value::as_str) != Some("thinking") {
        return None;
    }
    let text: String = part
        .get("thinking")?
        .as_array()?
        .iter()
        .filter(|chunk| chunk.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|chunk| chunk.get("text").and_then(Value::as_str))
        .collect();
    (!text.is_empty()).then_some(text)
}

// ── LanguageModel impl ───────────────────────────────────────────────────────

#[async_trait]
impl LanguageModel for MistralModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    async fn supported_urls(&self) -> aimux_core::language_model::SupportedUrls {
        self.config.supported_urls(&self.model_id)
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(build_request_body(&self.model_id, options, false)?);
        let endpoint = exchange.url("/chat/completions");
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(endpoint.clone(), options),
            body.clone(),
            aimux_provider_utils::create_json_response_handler(),
            super::mistral_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let data: ChatCompletionResponse = resp.value;

        let choice =
            data.choices.into_iter().next().ok_or_else(|| {
                AiMuxError::InvalidResponseData("no choices in response".to_string())
            })?;

        // Build content array.
        let mut content = Vec::new();

        if let Some(Value::Array(parts)) = &choice.message.content {
            for part in parts {
                match part.get("type").and_then(Value::as_str) {
                    Some("thinking") => {
                        if let Some(text) = extract_reasoning_part(part) {
                            content.push(GenerateContent::Reasoning(ReasoningOutput {
                                text,
                                provider_metadata: None,
                            }));
                        }
                    }
                    Some("text") => {
                        if let Some(text) = part
                            .get("text")
                            .and_then(Value::as_str)
                            .filter(|text| !text.is_empty())
                        {
                            content.push(GenerateContent::Text {
                                text: text.to_string(),
                                provider_metadata: None,
                            });
                        }
                    }
                    _ => {}
                }
            }
        } else if let Some(text) = extract_text_content(&choice.message.content) {
            content.push(GenerateContent::Text {
                text,
                provider_metadata: None,
            });
        }

        // Tool calls.
        if let Some(tool_calls) = choice.message.tool_calls {
            for tc in tool_calls {
                let input = tc.function.arguments;
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: tc.id,
                    tool_name: tc.function.name,
                    input,
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: None,
                }));
            }
        }

        let finish_reason = choice
            .finish_reason
            .as_deref()
            .map(parse_finish_reason)
            .unwrap_or(FINISH_OTHER);

        let usage = convert_usage(&data.usage);

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: request_warnings(options, &self.model_id),
            provider_metadata: None,
            response: Some(aimux_core::shared::ResponseInfo {
                id: data.id,
                timestamp: data
                    .created
                    .and_then(|secs| aimux_core::Timestamp::from_unix_seconds(secs as f64)),
                model_id: data.model,
                headers: Some(response_headers),
                body: resp.raw_value,
            }),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(build_request_body(&self.model_id, options, true)?);
        let endpoint = exchange.url("/chat/completions");
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(endpoint.clone(), options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            super::mistral_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let stream = pipe_through(
            resp.value,
            MistralChatStream::new(StreamContext {
                generate_id: self.generate_id.clone(),
                ..StreamContext::new(
                    options,
                    request_warnings(options, &self.model_id),
                    endpoint,
                    &body,
                    &response_headers,
                )
            }),
        );

        Ok(StreamResult {
            stream: Box::pin(stream),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
            response: Some(aimux_core::shared::StreamResponseInfo {
                headers: Some(response_headers),
            }),
        })
    }
}

/// The id of the (single) text block of a streamed response.
const TEXT_ID: &str = "0";

/// The `TransformStream` of the Mistral chat model's `doStream`.
#[derive(Default)]
struct MistralChatStream {
    /// Its id generator names reasoning blocks and id-less tool calls.
    ctx: StreamContext,
    tool_calls: ChatToolCalls,
    text_started: bool,
    /// The open reasoning block.
    reasoning_id: Option<String>,
    final_usage: Usage,
    final_finish_reason: Option<FinishReason>,
    response_metadata_emitted: bool,
}

/// A chat stream with an empty request context and tool-call ids from
/// `generate_id`, for tests that drive it chunk by chunk.
#[cfg(test)]
pub(crate) fn chat_test_stream(
    generate_id: Arc<dyn Fn() -> String + Send + Sync>,
) -> impl Transformer<Input = Result<Value, AiMuxError>, Output = StreamPart> {
    MistralChatStream::new(StreamContext {
        generate_id,
        ..StreamContext::default()
    })
}

impl MistralChatStream {
    fn new(ctx: StreamContext) -> Self {
        Self {
            tool_calls: ChatToolCalls::new(ctx.generate_id.clone()),
            ctx,
            ..Self::default()
        }
    }

    /// Handle one parsed SSE `data:` payload.
    fn transform_chunk(&mut self, parsed: Value, controller: &mut Controller) {
        self.ctx.raw(&parsed, controller);

        if let Some(err_obj) = parsed.get("error") {
            controller.enqueue(StreamPart::Error {
                error: super::mistral_stream_error(
                    err_obj,
                    &self.ctx.url,
                    self.ctx.request_body.clone(),
                    self.ctx.response_headers.clone(),
                ),
            });
            return;
        }

        let chunk: StreamChunk = match serde_json::from_value(parsed) {
            Ok(c) => c,
            Err(e) => {
                controller.enqueue(StreamPart::Error { error: e.into() });
                return;
            }
        };

        // Emit ResponseMetadata from the first valid chunk.
        if !self.response_metadata_emitted {
            self.response_metadata_emitted = true;
            controller.enqueue(StreamPart::ResponseMetadata(ResponseMetadata {
                id: chunk.id.clone(),
                timestamp: chunk
                    .created
                    .and_then(|secs| aimux_core::Timestamp::from_unix_seconds(secs as f64)),
                model_id: chunk.model.clone(),
            }));
        }

        // Update usage.
        if let Some(usage) = &chunk.usage {
            self.final_usage = convert_usage(usage);
        }

        // Upstream processes only the first choice.
        let Some(choice) = chunk.choices.into_iter().next() else {
            return;
        };

        // Reasoning content (from thinking parts in array content).
        for reasoning_delta in choice
            .delta
            .content
            .as_ref()
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(extract_reasoning_part)
        {
            if self.reasoning_id.is_none() {
                // End any active text before starting reasoning.
                if self.text_started {
                    controller.text_end(TEXT_ID);
                    self.text_started = false;
                }
                let rid = (self.ctx.generate_id)();
                self.reasoning_id = Some(rid.clone());
                controller.reasoning_start(rid);
            }
            if let Some(ref rid) = self.reasoning_id {
                controller.reasoning_delta(rid.clone(), reasoning_delta);
            }
        }

        // Text content.
        if let Some(text_delta) = extract_text_content(&choice.delta.content)
            && !text_delta.is_empty()
        {
            if !self.text_started {
                // End reasoning before starting text.
                if let Some(rid) = self.reasoning_id.take() {
                    controller.reasoning_end(rid);
                }
                controller.text_start(TEXT_ID);
                self.text_started = true;
            }
            controller.text_delta(TEXT_ID, text_delta);
        }

        // Tool calls: assembled by the shared tracker.
        if let Some(tool_call_deltas) = choice.delta.tool_calls {
            for dtc in &tool_call_deltas {
                let delta = ToolCallDelta {
                    index: dtc.index,
                    id: dtc.id.as_deref(),
                    name: dtc.function.name.as_deref(),
                    arguments: dtc.function.arguments.as_deref(),
                    ..ToolCallDelta::default()
                };
                // A new call without a function name is invalid response
                // data, as in the AI SDK; the stream ends.
                if !self.tool_calls.delta(delta, controller) {
                    return;
                }
            }
        }

        // Finish reason.
        if let Some(reason) = choice.finish_reason {
            self.final_finish_reason = Some(parse_finish_reason(&reason));
        }
    }
}

impl Transformer for MistralChatStream {
    type Input = Result<Value, AiMuxError>;
    type Output = StreamPart;

    fn start(&mut self, controller: &mut Controller) {
        self.ctx.start(controller);
    }

    fn transform(&mut self, event: Result<Value, AiMuxError>, controller: &mut Controller) {
        match event {
            Ok(parsed) => self.transform_chunk(parsed, controller),
            Err(error) => {
                self.ctx.raw(&Value::Null, controller);
                controller.enqueue(StreamPart::Error { error });
            }
        }
    }

    fn flush(mut self, controller: &mut Controller) {
        // Close any remaining open segments.
        if self.text_started {
            controller.text_end(TEXT_ID);
        }
        if let Some(rid) = self.reasoning_id.take() {
            controller.reasoning_end(rid);
        }

        self.tool_calls.finish(controller);

        controller.enqueue(StreamPart::Finish {
            finish_reason: self.final_finish_reason.unwrap_or(FINISH_OTHER),
            usage: self.final_usage,
            provider_metadata: None,
        });
    }
}
