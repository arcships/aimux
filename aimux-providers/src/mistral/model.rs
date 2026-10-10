//! Mistral language model — implements `LanguageModel` trait.
//!
//! Mirrors the TS `mistral-chat-language-model.ts`. Key differences from the
//! OpenAI model:
//! - `content` in responses can be a string or an array of typed parts
//!   (text, thinking, image_url). Thinking parts are extracted as reasoning
//!   in streaming mode.
//! - Streamed tool calls are assembled by the shared `StreamingToolCallTracker`.
//! - Usage supports `num_cached_tokens` / `prompt_tokens_details.cached_tokens`.
//! - Finish reasons include `model_length`.

use aimux_core::tool::RawToolCall;
use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage, Warning};

use aimux_provider_utils::{
    StreamingToolCallDelta, StreamingToolCallTracker, TransformStreamController, Transformer,
    pipe_through,
};

use crate::shared::EndpointConfig;

use super::convert::{build_request_body, parse_finish_reason, request_warnings};
use super::types::{ChatCompletionResponse, StreamChunk, UsageResponse};

/// A Mistral language model.
pub struct MistralModel {
    model_id: String,
    config: EndpointConfig,
    generate_id: aimux_provider_utils::IdGenerator,
}

impl MistralModel {
    pub(crate) fn from_config(
        model_id: String,
        config: EndpointConfig,
        generate_id_fn: Option<aimux_provider_utils::IdGenerator>,
    ) -> Self {
        Self {
            model_id,
            config,
            generate_id: generate_id_fn.unwrap_or(aimux_provider_utils::generate_id),
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

    fn supported_urls(&self) -> aimux_core::language_model::SupportedUrls {
        self.config.supported_urls(&self.model_id)
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = build_request_body(&self.model_id, options, false)?;
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
            .unwrap_or(FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            });

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
                    .and_then(|secs| chrono::DateTime::from_timestamp(secs as i64, 0))
                    .map(|dt| dt.to_rfc3339()),
                model_id: data.model,
                headers: Some(response_headers),
                body: resp.raw_value,
            }),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = build_request_body(&self.model_id, options, true)?;
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
            MistralChatStream {
                warnings: request_warnings(options, &self.model_id),
                include_raw_chunks: options.include_raw_chunks.unwrap_or(false),
                stream_error_url: endpoint,
                stream_error_body: body.clone(),
                stream_response_headers: response_headers.clone(),
                generate_id: self.generate_id,
                text_started: false,
                reasoning_started: false,
                tool_calls: StreamingToolCallTracker::new().with_generate_id(self.generate_id),
                tool_parts: Vec::new(),
                reasoning_id: None,
                final_usage: Usage::default(),
                final_finish_reason: None,
                response_metadata_emitted: false,
            },
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
struct MistralChatStream {
    warnings: Vec<Warning>,
    include_raw_chunks: bool,
    stream_error_url: String,
    stream_error_body: Value,
    stream_response_headers: HashMap<String, String>,
    generate_id: aimux_provider_utils::IdGenerator,
    text_started: bool,
    reasoning_started: bool,
    tool_calls: StreamingToolCallTracker,
    tool_parts: Vec<StreamPart>,
    reasoning_id: Option<String>,
    final_usage: Usage,
    final_finish_reason: Option<FinishReason>,
    response_metadata_emitted: bool,
}

impl MistralChatStream {
    /// Handle one parsed SSE `data:` payload.
    fn transform_chunk(
        &mut self,
        parsed: Value,
        controller: &mut TransformStreamController<StreamPart>,
    ) {
        if self.include_raw_chunks {
            controller.enqueue(StreamPart::Raw {
                raw_value: parsed.clone(),
            });
        }

        if let Some(err_obj) = parsed.get("error") {
            controller.enqueue(StreamPart::Error {
                error: super::mistral_stream_error(
                    err_obj,
                    &self.stream_error_url,
                    self.stream_error_body.clone(),
                    self.stream_response_headers.clone(),
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
                    .and_then(|secs| chrono::DateTime::from_timestamp(secs as i64, 0))
                    .map(|dt| dt.to_rfc3339()),
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
            if !self.reasoning_started {
                // End any active text before starting reasoning.
                if self.text_started {
                    controller.enqueue(StreamPart::TextEnd {
                        id: TEXT_ID.to_string(),
                        provider_metadata: None,
                    });
                    self.text_started = false;
                }
                let rid = (self.generate_id)();
                self.reasoning_id = Some(rid.clone());
                self.reasoning_started = true;
                controller.enqueue(StreamPart::ReasoningStart {
                    id: rid,
                    provider_metadata: None,
                });
            }
            if let Some(ref rid) = self.reasoning_id {
                controller.enqueue(StreamPart::ReasoningDelta {
                    id: rid.clone(),
                    delta: reasoning_delta,
                    provider_metadata: None,
                });
            }
        }

        // Text content.
        if let Some(text_delta) = extract_text_content(&choice.delta.content)
            && !text_delta.is_empty()
        {
            if !self.text_started {
                // End reasoning before starting text.
                if self.reasoning_started {
                    if let Some(ref rid) = self.reasoning_id {
                        controller.enqueue(StreamPart::ReasoningEnd {
                            id: rid.clone(),
                            provider_metadata: None,
                        });
                    }
                    self.reasoning_started = false;
                    self.reasoning_id = None;
                }
                controller.enqueue(StreamPart::TextStart {
                    id: TEXT_ID.to_string(),
                    provider_metadata: None,
                });
                self.text_started = true;
            }
            controller.enqueue(StreamPart::TextDelta {
                id: TEXT_ID.to_string(),
                delta: text_delta,
                provider_metadata: None,
            });
        }

        // Tool calls: assembled by the shared tracker.
        if let Some(tool_call_deltas) = choice.delta.tool_calls {
            for dtc in &tool_call_deltas {
                let delta = StreamingToolCallDelta {
                    index: dtc.index,
                    id: dtc.id.as_deref(),
                    name: dtc.function.name.as_deref(),
                    arguments: dtc.function.arguments.as_deref(),
                    ..Default::default()
                };
                // A new call without a function name is invalid response
                // data, as in the AI SDK; the stream ends.
                if let Err(error) = self.tool_calls.process(delta, &mut self.tool_parts) {
                    controller.error(error.into());
                    return;
                }
                for part in self.tool_parts.drain(..) {
                    controller.enqueue(part);
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

    fn start(&mut self, controller: &mut TransformStreamController<StreamPart>) {
        controller.enqueue(StreamPart::StreamStart {
            warnings: std::mem::take(&mut self.warnings),
        });
    }

    fn transform(
        &mut self,
        event: Result<Value, AiMuxError>,
        controller: &mut TransformStreamController<StreamPart>,
    ) {
        match event {
            Ok(parsed) => self.transform_chunk(parsed, controller),
            Err(error) => {
                if self.include_raw_chunks {
                    controller.enqueue(StreamPart::Raw {
                        raw_value: Value::Null,
                    });
                }
                controller.enqueue(StreamPart::Error { error });
            }
        }
    }

    fn flush(mut self, controller: &mut TransformStreamController<StreamPart>) {
        // Close any remaining open segments.
        if self.text_started {
            controller.enqueue(StreamPart::TextEnd {
                id: TEXT_ID.to_string(),
                provider_metadata: None,
            });
        }
        if self.reasoning_started
            && let Some(rid) = self.reasoning_id.take()
        {
            controller.enqueue(StreamPart::ReasoningEnd {
                id: rid,
                provider_metadata: None,
            });
        }

        self.tool_calls.finish(&mut self.tool_parts);
        for part in self.tool_parts.drain(..) {
            controller.enqueue(part);
        }

        controller.enqueue(StreamPart::Finish {
            finish_reason: self.final_finish_reason.unwrap_or(FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            }),
            usage: self.final_usage,
            provider_metadata: None,
        });
    }
}
