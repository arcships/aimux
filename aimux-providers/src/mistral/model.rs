//! Mistral language model — implements `LanguageModel` trait.
//!
//! Mirrors the TS `mistral-chat-language-model.ts`. Key differences from the
//! OpenAI model:
//! - `content` in responses can be a string or an array of typed parts
//!   (text, thinking, image_url). Thinking parts are extracted as reasoning
//!   in streaming mode.
//! - Streaming tool calls arrive complete in a single chunk (no index-based
//!   incremental accumulation).
//! - Usage supports `num_cached_tokens` / `prompt_tokens_details.cached_tokens`.
//! - Finish reasons include `model_length`.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

use aimux_provider_utils::HttpRequest;

use super::MistralConfig;
use super::convert::{build_request_body, parse_finish_reason};
use super::types::{ChatCompletionResponse, StreamChunk, UsageResponse};

/// An Mistral language model.
pub struct MistralModel {
    model_id: String,
    config: MistralConfig,
}

impl MistralModel {
    pub fn new(model_id: String, config: MistralConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.config.base_url)
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
        input_tokens: aimux_core::types::TokenUsage {
            total: Some(prompt_tokens),
            no_cache: Some(no_cache),
            cache_read: if cache_read > 0 {
                Some(cache_read)
            } else {
                None
            },
            cache_write: None,
            ..Default::default()
        },
        output_tokens: aimux_core::types::TokenUsage {
            total: Some(completion_tokens),
            ..Default::default()
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: Some(serde_json::to_value(usage).unwrap_or(serde_json::Value::Null)),
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
        "mistral"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn retry_config(&self) -> aimux_core::retry::RetryConfig {
        self.config.retry_config
    }

    fn config_snapshot(&self) -> aimux_core::recording::ProviderRecord {
        use aimux_core::recording::ProviderRecord;
        ProviderRecord {
            provider: self.provider().to_string(),
            model_id: self.model_id.clone(),
            base_url: Some(self.config.base_url.clone()),
            api_key_source: self
                .config
                .api_key_source
                .clone()
                .unwrap_or_else(|| "explicit".to_string()),
            profile: None,
            provider_options: None,
        }
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let body = build_request_body(&self.model_id, options, false);
        let headers = self.build_headers(options.headers.as_ref());
        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: self.endpoint(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
                recording_context: options.recording_context.clone(),
                response_timeout: None,
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
            body.clone(),
            aimux_provider_utils::create_json_response_handler(),
            super::mistral_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers.unwrap_or_default();
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
            content.push(GenerateContent::Reasoning {
                text: r,
                provider_metadata: None,
            });
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
                let input: Value = serde_json::from_str(&tc.function.arguments)
                    .unwrap_or_else(|_| Value::String(tc.function.arguments.clone()));
                content.push(GenerateContent::ToolCall {
                    tool_call_id: tc.id,
                    tool_name: tc.function.name,
                    input,
                    provider_executed: None,
                    dynamic: None,
                    thought_signature: None,
                    provider_metadata: None,
                });
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
            response: ResponseMetadata {
                id: data.id,
                timestamp: data
                    .created
                    .and_then(|secs| chrono::DateTime::from_timestamp(secs as i64, 0))
                    .map(|dt| dt.to_rfc3339()),
                model_id: data.model,
            },
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let body = build_request_body(&self.model_id, options, true);
        let headers = self.build_headers(options.headers.as_ref());
        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: self.endpoint(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
                recording_context: options.recording_context.clone(),
                response_timeout: None,
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            super::mistral_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers.unwrap_or_default();
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
                &self.endpoint(),
                body.clone(),
                response_headers.clone(),
            ));
        }

        let stream_error_url = self.endpoint();
        let stream_error_body = body.clone();
        let stream_response_headers = response_headers.clone();

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings: vec![] });

            let text_id = 0usize;
            let mut text_started = false;
            let mut reasoning_started = false;
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
                            yield Ok(StreamPart::ResponseMetadata {
                                id: chunk.id.clone(),
                                timestamp: chunk
                                    .created
                                    .and_then(|secs| {
                                        chrono::DateTime::from_timestamp(secs as i64, 0)
                                    })
                                    .map(|dt| dt.to_rfc3339()),
                                model_id: chunk.model.clone(),
                            });
                        }

                        // Update usage.
                        if let Some(usage) = &chunk.usage {
                            final_usage = convert_usage(usage);
                        }

                        // Process choices.
                        for choice in chunk.choices {
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

                            // Tool calls (complete in a single chunk).
                            if let Some(tool_call_deltas) = choice.delta.tool_calls {
                                for dtc in tool_call_deltas {
                                    let tool_id = dtc.id;
                                    let tool_name = dtc.function.name;
                                    let args = dtc.function.arguments;

                                    yield Ok(StreamPart::ToolInputStart {
                                        id: tool_id.clone(),
                                        tool_name: tool_name.clone(),
                                        provider_executed: None,
                                        dynamic: None,
                                        title: None,
                                        provider_metadata: None,
                                    });

                                    if !args.is_empty() {
                                        yield Ok(StreamPart::ToolInputDelta {
                                            id: tool_id.clone(),
                                            delta: args.clone(),
                                            provider_metadata: None,
                                        });
                                    }

                                    yield Ok(StreamPart::ToolInputEnd {
                                        id: tool_id.clone(),
                                        provider_metadata: None,
                                    });

                                    let input: Value = serde_json::from_str(&args)
                                        .unwrap_or_else(|_| Value::String(args.clone()));
                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: tool_id,
                                        tool_name,
                                        input,
                                        provider_executed: None,
                                        dynamic: None,
                                        thought_signature: None,
                                        provider_metadata: None,
                                    });
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
                provider_metadata: Some(serde_json::json!({ "mistral": {} })),
            });
        };

        Ok(StreamResult {
            stream: Box::pin(stream),
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_snapshot_reports_config() {
        let model = MistralModel::new(
            "mistral-small-latest".to_string(),
            MistralConfig::new("test-key"),
        );
        let snapshot = model.config_snapshot();
        assert_eq!(snapshot.provider, "mistral");
        assert_eq!(snapshot.model_id, "mistral-small-latest");
        assert_eq!(
            snapshot.base_url.as_deref(),
            Some("https://api.mistral.ai/v1")
        );
        assert_eq!(snapshot.api_key_source, "explicit");
    }
}
