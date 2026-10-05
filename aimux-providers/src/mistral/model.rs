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
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

use aimux_provider_utils::{
    StreamingToolCallDelta, StreamingToolCallFunction, StreamingToolCallTracker, generate_id,
};

use crate::shared::EndpointConfig;

use super::convert::{build_request_body, parse_finish_reason};
use super::types::{ChatCompletionResponse, StreamChunk, UsageResponse};

/// A Mistral language model.
pub struct MistralModel {
    model_id: String,
    config: EndpointConfig,
}

impl MistralModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
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
    let prompt_tokens = usage.prompt_tokens.unwrap_or(0);
    let completion_tokens = usage.completion_tokens.unwrap_or(0);

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
            ..Default::default()
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: serde_json::to_value(usage)
            .ok()
            .and_then(|value| value.as_object().cloned()),
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

/// Extract reasoning content from a Mistral `content` array's `thinking` parts.
///
/// Each `thinking` part has a `thinking` array of `{type:"text", text}` chunks.
fn extract_reasoning_content(content: &Option<Value>) -> Option<String> {
    match content {
        Some(Value::Array(arr)) => {
            let text: String = arr
                .iter()
                .filter_map(|part| {
                    if part.get("type").and_then(|t| t.as_str()) == Some("thinking") {
                        part.get("thinking")
                            .and_then(|t| t.as_array())
                            .map(|thinking| {
                                thinking
                                    .iter()
                                    .filter_map(|chunk| {
                                        if chunk.get("type").and_then(|t| t.as_str())
                                            == Some("text")
                                        {
                                            chunk
                                                .get("text")
                                                .and_then(|t| t.as_str())
                                                .map(std::string::ToString::to_string)
                                        } else {
                                            None
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join("")
                            })
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

// ── LanguageModel impl ───────────────────────────────────────────────────────

#[async_trait]
impl LanguageModel for MistralModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn supported_urls(&self) -> aimux_core::language_model::SupportedUrls {
        (self.config.supported_urls)(&self.model_id)
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

        let response_body = resp.raw_value;
        let response_headers = resp.response_headers;
        let data: ChatCompletionResponse = resp.value;

        let choice =
            data.choices.into_iter().next().ok_or_else(|| {
                AiMuxError::InvalidResponseData("no choices in response".to_string())
            })?;

        // Build content array.
        let mut content = Vec::new();

        // Reasoning content (from thinking parts in array content).
        // Pushed before text to match upstream ordering (reasoning → text).
        if let Some(r) = extract_reasoning_content(&choice.message.content)
            && !r.is_empty()
        {
            content.push(GenerateContent::Reasoning(ReasoningOutput {
                text: r,
                provider_metadata: None,
            }));
        }

        // Content can be a string (legacy) or an array of typed parts.
        if let Some(text) = extract_text_content(&choice.message.content)
            && !text.is_empty()
        {
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
                    thought_signature: None,
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
            warnings: Vec::new(),
            provider_metadata: None,
            response: Some(aimux_core::shared::ResponseInfo {
                id: data.id,
                timestamp: data
                    .created
                    .and_then(|secs| chrono::DateTime::from_timestamp(secs as i64, 0))
                    .map(|dt| dt.to_rfc3339()),
                model_id: data.model,
                headers: Some(response_headers),
                body: response_body,
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
        let mut sse_stream = resp.value;

        // Peek at the first SSE event to detect early errors.
        let first_event = match sse_stream.next().await {
            Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
            first_event => first_event,
        };
        if let Some(Ok(ref event)) = first_event
            && let Some(err_obj) = event.get("error")
        {
            return Err(super::mistral_stream_error(
                err_obj,
                &endpoint,
                body.clone(),
                response_headers.clone(),
            ));
        }

        let stream_error_url = endpoint;
        let stream_error_body = body.clone();
        let stream_response_headers = response_headers.clone();

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings: vec![] });

            let text_id = 0usize;
            let mut text_started = false;
            let mut reasoning_started = false;
            let mut tool_calls = StreamingToolCallTracker::new().with_generate_id(generate_id);
            let mut reasoning_id: Option<String> = None;
            let mut final_usage = Usage::default();
            let mut final_finish_reason: Option<FinishReason> = None;
            let mut response_metadata_emitted = false;

            let mut event_iter =
                futures::stream::iter(first_event.into_iter()).chain(sse_stream);

            let mut stream_errored = false;

            while let Some(event) = event_iter.next().await {
                if stream_errored {
                    break;
                }

                match event {
                    Ok(parsed) => {

                        if let Some(err_obj) = parsed.get("error") {
                            yield Ok(StreamPart::Error {
                                error: super::mistral_stream_error(
                                    err_obj,
                                    &stream_error_url,
                                    stream_error_body.clone(),
                                    stream_response_headers.clone(),
                                ),
                            });
                            stream_errored = true;
                            break;
                        }

                        let chunk: StreamChunk = match serde_json::from_value(parsed) {
                            Ok(c) => c,
                            Err(e) => {
                                yield Err(e.into());
                                continue;
                            }
                        };

                        // Emit ResponseMetadata from the first valid chunk.
                        if !response_metadata_emitted
                            && (chunk.id.is_some() || chunk.model.is_some())
                        {
                            response_metadata_emitted = true;
                            yield Ok(StreamPart::ResponseMetadata(ResponseMetadata {
                                id: chunk.id.clone(),
                                timestamp: chunk
                                    .created
                                    .and_then(|secs| {
                                        chrono::DateTime::from_timestamp(secs as i64, 0)
                                    })
                                    .map(|dt| dt.to_rfc3339()),
                                model_id: chunk.model.clone(),
                            }));
                        }

                        // Update usage.
                        if let Some(usage) = &chunk.usage {
                            final_usage = convert_usage(usage);
                        }

                        // Upstream processes only the first choice.
                        if let Some(choice) = chunk.choices.into_iter().next() {
                            // Reasoning content (from thinking parts in array content).
                            if let Some(reasoning_delta) =
                                extract_reasoning_content(&choice.delta.content)
                                && !reasoning_delta.is_empty() {
                                    if !reasoning_started {
                                        // End any active text before starting reasoning.
                                        if text_started {
                                            yield Ok(StreamPart::TextEnd {
                                                id: format!("{text_id}"),
                                                provider_metadata: None,
                                            });
                                            text_started = false;
                                        }
                                        let rid = format!(
                                            "rc-{}",
                                            std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .map(|d| d.as_nanos())
                                                .unwrap_or(0)
                                        );
                                        reasoning_id = Some(rid.clone());
                                        reasoning_started = true;
                                        yield Ok(StreamPart::ReasoningStart { id: rid,
                provider_metadata: None,
            });
                                    }
                                    if let Some(ref rid) = reasoning_id {
                                        yield Ok(StreamPart::ReasoningDelta {
                                            id: rid.clone(),
                                            delta: reasoning_delta,
                provider_metadata: None,
            });
                                    }
                                }

                            // Text content.
                            if let Some(text_delta) =
                                extract_text_content(&choice.delta.content)
                                && !text_delta.is_empty() {
                                    if !text_started {
                                        // End reasoning before starting text.
                                        if reasoning_started {
                                            if let Some(ref rid) = reasoning_id {
                                                yield Ok(StreamPart::ReasoningEnd {
                                                    id: rid.clone(),
                provider_metadata: None,
            });
                                            }
                                            reasoning_started = false;
                                            reasoning_id = None;
                                        }
                                        yield Ok(StreamPart::TextStart {
                                            id: format!("{text_id}"),
                                            provider_metadata: None,
                                        });
                                        text_started = true;
                                    }
                                    yield Ok(StreamPart::TextDelta {
                                        id: format!("{text_id}"),
                                        delta: text_delta,
                                        provider_metadata: None,
                                    });
                                }

                            // Tool calls: assembled by the shared tracker.
                            if let Some(tool_call_deltas) = choice.delta.tool_calls {
                                for dtc in tool_call_deltas {
                                    let delta = StreamingToolCallDelta {
                                        index: dtc.index,
                                        id: dtc.id,
                                        r#type: None,
                                        function: Some(StreamingToolCallFunction {
                                            name: dtc.function.name,
                                            arguments: dtc.function.arguments,
                                        }),
                                        extra: Value::Null,
                                    };
                                    match tool_calls.process_delta(&delta) {
                                        Ok(parts) => {
                                            for part in parts {
                                                yield Ok(part);
                                            }
                                        }
                                        // A new call without a function name
                                        // is invalid response data, as in the
                                        // AI SDK; the stream ends.
                                        Err(error) => {
                                            yield Err(error.into());
                                            return;
                                        }
                                    }
                                }
                            }

                            // Finish reason.
                            if let Some(reason) = choice.finish_reason {
                                if text_started {
                                    yield Ok(StreamPart::TextEnd {
                                        id: format!("{text_id}"),
                                        provider_metadata: None,
                                    });
                                    text_started = false;
                                }
                                if reasoning_started {
                                    if let Some(ref rid) = reasoning_id {
                                        yield Ok(StreamPart::ReasoningEnd {
                                            id: rid.clone(),
                provider_metadata: None,
            });
                                    }
                                    reasoning_started = false;
                                    reasoning_id = None;
                                }
                                final_finish_reason = Some(parse_finish_reason(&reason));
                            }
                        }
                    }
                    Err(error) => {
                        let recoverable = error.is_recoverable_stream_error();
                        yield Err(error);
                        if !recoverable {
                            return;
                        }
                    }
                }
            }

            // Close any remaining open segments.
            if text_started {
                yield Ok(StreamPart::TextEnd {
                    id: format!("{text_id}"),
                    provider_metadata: None,
                });
            }
            if reasoning_started
                && let Some(ref rid) = reasoning_id {
                    yield Ok(StreamPart::ReasoningEnd {
                        id: rid.clone(),
                provider_metadata: None,
            });
                }

            for part in tool_calls.flush() {
                yield Ok(part);
            }

            yield Ok(StreamPart::Finish {
                finish_reason: if stream_errored {
                    FinishReason {
                        unified: FinishReasonUnified::Error,
                        raw: None,
                    }
                } else {
                    final_finish_reason.unwrap_or(FinishReason {
                        unified: FinishReasonUnified::Stop,
                        raw: None,
                    })
                },
                usage: if stream_errored {
                    Usage::default()
                } else {
                    final_usage
                },
                provider_metadata: Some(super::options::mistral_metadata(serde_json::json!({}))),
            });
        };

        Ok(StreamResult {
            stream: Box::pin(stream),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
            response: Some(aimux_core::shared::StreamResponseInfo {
                headers: Some(response_headers),
            }),
        })
    }
}
