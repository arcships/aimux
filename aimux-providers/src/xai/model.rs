//! xAI language model — implements `LanguageModel` trait.
//!
//! Mirrors the TS `XaiChatLanguageModel`. Unlike the thin OpenAI-compatible
//! wrappers, xAI has enough provider-specific behaviour (reasoning content,
//! citations, search parameters, xai-keyed provider options, non-inclusive
//! cached tokens, reasoning-effort model gating, 200-status errors) to warrant
//! its own model implementation rather than reusing `OpenAIModel`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, send, send_stream};
use aimux_stream::SseStream;

use super::convert::{build_request_body_with_warnings, convert_xai_usage, parse_finish_reason};
use super::types::{XaiChatResponse, XaiStreamChunk};

/// Global counter for generating unique source IDs.
static SOURCE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique source ID.
fn generate_source_id() -> String {
    format!(
        "xai-source-{}",
        SOURCE_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
    )
}

/// An xAI language model (Grok).
///
/// Does **not** hold an HTTP client — `http::send` / `http::send_stream` use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct XaiModel {
    model_id: String,
    config: super::XAIConfig,
}

impl XaiModel {
    pub fn new(model_id: String, config: super::XAIConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key()),
        );
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.config.base_url())
    }
}

/// Build the header list for a JSON POST: auth/extra headers + `Content-Type`.
///
/// Returns a `Vec<(String, String)>` for `HttpRequest` — no reqwest types.
fn build_header_list(headers: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> =
        headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    list.push(("Content-Type".to_string(), "application/json".to_string()));
    list
}

#[async_trait]
impl LanguageModel for XaiModel {
    fn provider(&self) -> &str {
        "xai.chat"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let request_result = build_request_body_with_warnings(&self.model_id, options, false)?;
        let body = request_result.body;

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: build_header_list(&headers),
                body: HttpBody::Json(body.clone()),
            },
            self.config.retry_config(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        // Capture response headers.
        let response_headers = resp.headers;

        let raw_value: Value = serde_json::from_slice(&resp.body).unwrap_or(Value::Null);

        // Check for 200-status error (xAI sometimes returns errors with 200).
        if let Some(error_msg) = raw_value.get("error").and_then(|v| v.as_str()) {
            return Err(AiMuxError::ApiCall(error_msg.to_string()));
        }

        let data: XaiChatResponse = serde_json::from_value(raw_value.clone())
            .map_err(|e| AiMuxError::Http(format!("failed to parse response: {}", e)))?;

        // Handle error field
        if let Some(error_msg) = &data.error {
            return Err(AiMuxError::ApiCall(error_msg.clone()));
        }

        let choice = data
            .choices
            .and_then(|mut c| c.drain(..).next())
            .ok_or_else(|| AiMuxError::Provider("no choices in response".to_string()))?;

        // Build content array.
        let mut content = Vec::new();

        // Extract text content
        if let Some(text) = choice.message.content
            && !text.is_empty()
        {
            let mut text = text;
            // Skip if this content duplicates the last assistant message
            if let Some(last_msg) = body
                .get("messages")
                .and_then(|m| m.as_array())
                .and_then(|arr| arr.last())
                && last_msg.get("role").and_then(|v| v.as_str()) == Some("assistant")
                && last_msg.get("content").and_then(|v| v.as_str()) == Some(&text)
            {
                text = String::new();
            }
            if !text.is_empty() {
                content.push(GenerateContent::Text { text, provider_metadata: None});
            }
        }

        // Extract reasoning content
        if let Some(reasoning) = choice.message.reasoning_content
            && !reasoning.is_empty()
        {
            content.push(GenerateContent::Reasoning {
                text: reasoning,
                provider_metadata: None,
            });
        }

        // Extract tool calls
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
                    provider_metadata: None,
                });
            }
        }

        // Extract citations
        if let Some(citations) = &data.citations {
            for url in citations {
                content.push(GenerateContent::Source {
                    id: generate_source_id(),
                    source_type: "url".to_string(),
                    url: Some(url.clone()),
                    title: None,
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

        let usage = data.usage.as_ref().map(convert_xai_usage).unwrap_or(Usage {
            input_tokens: aimux_core::types::TokenUsage {
                total: Some(0),
                no_cache: Some(0),
                cache_read: Some(0),
                cache_write: Some(0),
                ..Default::default()
            },
            output_tokens: aimux_core::types::TokenUsage {
                total: Some(0),
                text: Some(0),
                reasoning: Some(0),
                ..Default::default()
            },
            raw: None,
        });

        let timestamp = data.created.map(|c| {
            chrono::DateTime::from_timestamp(c as i64, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        });

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: request_result.warnings,
            provider_metadata: None,
            response: ResponseMetadata {
                id: data.id,
                timestamp,
                model_id: data.model,
            },
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let request_result = build_request_body_with_warnings(&self.model_id, options, true)?;
        let body = request_result.body;
        let warnings = request_result.warnings;

        let resp = send_stream(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: build_header_list(&headers),
                body: HttpBody::Json(body.clone()),
            },
            self.config.retry_config(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;

        // Check if the response is JSON (not SSE) — xAI sometimes returns
        // errors with 200 status and content-type application/json.
        let content_type = response_headers
            .get("content-type")
            .map(|s| s.as_str())
            .unwrap_or("");
        if content_type.contains("application/json") {
            // Collect the (non-SSE) JSON body and check for an error object.
            let mut buf = Vec::new();
            let mut body_stream = resp.body;
            while let Some(chunk) = body_stream.next().await {
                if let Ok(bytes) = chunk {
                    buf.extend_from_slice(&bytes);
                }
            }
            if let Ok(val) = serde_json::from_slice::<Value>(&buf)
                && let Some(err_msg) = val.get("error").and_then(|v| v.as_str())
            {
                return Err(AiMuxError::ApiCall(err_msg.to_string()));
            }
            return Err(AiMuxError::Provider(
                "Expected SSE stream but got JSON response".to_string(),
            ));
        }

        let mut sse_stream = SseStream::new(resp.body);

        // Peek at the first SSE event to detect early errors.
        let first_event = sse_stream.next().await;
        if let Some(Ok(ref event)) = first_event
            && let Ok(val) = serde_json::from_str::<Value>(&event.data)
        {
            // Check for error in the first chunk (200-status error).
            if let Some(err_msg) = val.get("error").and_then(|v| v.as_str()) {
                return Err(AiMuxError::ApiCall(err_msg.to_string()));
            }
        }

        let messages_for_dedup = body
            .get("messages")
            .and_then(|m| m.as_array())
            .cloned()
            .unwrap_or_default();

        let stream = async_stream::stream! {
            // First part: StreamStart.
            yield Ok(StreamPart::StreamStart { warnings });

            let mut final_usage: Option<Usage> = None;
            let mut final_finish_reason: Option<FinishReason> = None;
            let mut response_metadata_emitted = false;

            // Content block tracking
            let mut content_blocks: HashMap<String, bool> = HashMap::new(); // block_id -> ended
            let mut last_reasoning_deltas: HashMap<String, String> = HashMap::new();
            let mut active_reasoning_block_id: Option<String> = None;

            // Tool-call accumulators keyed by index (for incremental streaming).
            let mut tool_calls: HashMap<usize, (String, String, String)> = HashMap::new(); // (id, name, arguments)
            let mut tool_call_order: Vec<usize> = Vec::new();

            let mut event_iter =
                futures::stream::iter(first_event.into_iter()).chain(sse_stream);

            while let Some(event) = event_iter.next().await {
                match event {
                    Ok(sse_event) => {
                        if sse_event.data == "[DONE]" {
                            break;
                        }

                        let parsed: Value = match serde_json::from_str(&sse_event.data) {
                            Ok(v) => v,
                            Err(e) => {
                                yield Ok(StreamPart::Error {
                                    error: AiMuxError::Json(e.to_string()),
                                });
                                break;
                            }
                        };

                        // Check for mid-stream error.
                        if let Some(err_obj) = parsed.get("error") {
                            let msg = err_obj
                                .as_str()
                                .unwrap_or("Unknown stream error")
                                .to_string();
                            yield Ok(StreamPart::Error {
                                error: AiMuxError::ApiCall(msg),
                            });
                            break;
                        }

                        let chunk: XaiStreamChunk = match serde_json::from_value(parsed) {
                            Ok(c) => c,
                            Err(e) => {
                                yield Ok(StreamPart::Error {
                                    error: AiMuxError::Json(e.to_string()),
                                });
                                break;
                            }
                        };

                        // Emit response metadata on first chunk.
                        if !response_metadata_emitted
                            && (chunk.id.is_some() || chunk.model.is_some())
                        {
                            response_metadata_emitted = true;
                            let timestamp = chunk.created.and_then(|c| {
                                chrono::DateTime::from_timestamp(c as i64, 0)
                                    .map(|dt| dt.to_rfc3339())
                            });
                            yield Ok(StreamPart::ResponseMetadata {
                                id: chunk.id.clone(),
                                timestamp,
                                model_id: chunk.model.clone(),
                            });
                        }

                        // Emit citations as sources.
                        if let Some(citations) = &chunk.citations {
                            for url in citations {
                                yield Ok(StreamPart::Source {
                                    id: generate_source_id(),
                                    source_type: "url".to_string(),
                                    url: Some(url.clone()),
                                    title: None,
                                    provider_metadata: None,
                                });
                            }
                        }

                        // Update usage.
                        if let Some(usage) = &chunk.usage {
                            final_usage = Some(convert_xai_usage(usage));
                        }

                        // Process choices.
                        for choice in &chunk.choices {
                            let choice_index = choice.index.unwrap_or(0);
                            let chunk_id = chunk.id.clone().unwrap_or_else(|| choice_index.to_string());

                            let delta = &choice.delta;

                            // Process text content.
                            if let Some(text_content) = &delta.content
                                && !text_content.is_empty()
                            {
                                // End active reasoning block.
                                if let Some(ref active_id) = active_reasoning_block_id {
                                    if !content_blocks.get(active_id).copied().unwrap_or(false) {
                                        yield Ok(StreamPart::ReasoningEnd {
                                            id: active_id.clone(),
                provider_metadata: None,
            });
                                        content_blocks.insert(active_id.clone(), true);
                                    }
                                    active_reasoning_block_id = None;
                                }

                                // Skip if duplicates last assistant message.
                                let last_msg = messages_for_dedup.last();
                                let is_dup = last_msg
                                    .and_then(|m| m.get("role").and_then(|v| v.as_str()))
                                    == Some("assistant")
                                    && last_msg
                                        .and_then(|m| m.get("content").and_then(|v| v.as_str()))
                                        == Some(text_content.as_str());

                                if !is_dup {
                                    let block_id = format!("text-{}", chunk_id);
                                    if !content_blocks.contains_key(&block_id) {
                                        content_blocks.insert(block_id.clone(), false);
                                        yield Ok(StreamPart::TextStart {
                                            id: block_id.clone(),
                                            provider_metadata: None,
                                        });
                                    }
                                    yield Ok(StreamPart::TextDelta {
                                        id: block_id,
                                        delta: text_content.clone(),
                                        provider_metadata: None,
                                    });
                                }
                            }

                            // Process reasoning content.
                            if let Some(reasoning_content) = &delta.reasoning_content
                                && !reasoning_content.is_empty()
                            {
                                let block_id = format!("reasoning-{}", chunk_id);

                                // Skip if duplicates last delta.
                                if last_reasoning_deltas.get(&block_id).map(|s| s.as_str())
                                    != Some(reasoning_content.as_str())
                                {
                                    last_reasoning_deltas
                                        .insert(block_id.clone(), reasoning_content.clone());

                                    if !content_blocks.contains_key(&block_id) {
                                        content_blocks.insert(block_id.clone(), false);
                                        active_reasoning_block_id = Some(block_id.clone());
                                        yield Ok(StreamPart::ReasoningStart {
                                            id: block_id.clone(),
                provider_metadata: None,
            });
                                    }
                                    yield Ok(StreamPart::ReasoningDelta {
                                        id: block_id,
                                        delta: reasoning_content.clone(),
                provider_metadata: None,
            });
                                }
                            }

                            // Process tool calls.
                            // xAI typically sends tool calls in one piece, but
                            // we also support incremental (OpenAI-style) streaming.
                            if let Some(tool_call_deltas) = &delta.tool_calls {
                                // End active reasoning block.
                                if let Some(ref active_id) = active_reasoning_block_id {
                                    if !content_blocks.get(active_id).copied().unwrap_or(false) {
                                        yield Ok(StreamPart::ReasoningEnd {
                                            id: active_id.clone(),
                provider_metadata: None,
            });
                                        content_blocks.insert(active_id.clone(), true);
                                    }
                                    active_reasoning_block_id = None;
                                }

                                for dtc in tool_call_deltas {
                                    let idx = dtc.index.unwrap_or(0);
                                    let func = dtc.function.clone().unwrap_or_default();

                                    let is_new = !tool_calls.contains_key(&idx);
                                    if is_new {
                                        let id = dtc.id.clone().unwrap_or_default();
                                        let name = func.name.unwrap_or_default();
                                        tool_calls.insert(idx, (id.clone(), name.clone(), String::new()));
                                        tool_call_order.push(idx);
                                        yield Ok(StreamPart::ToolInputStart {
                                            id,
                                            tool_name: name,
                                            provider_executed: None,
                                            dynamic: None,
                                            title: None,
                                            provider_metadata: None,
                                        });
                                    }

                                    if let Some(args) = func.arguments
                                        && (!is_new || !args.is_empty())
                                        && let Some(&mut (ref id, _, ref mut acc)) = tool_calls.get_mut(&idx)
                                    {
                                        acc.push_str(&args);
                                        yield Ok(StreamPart::ToolInputDelta {
                                            id: id.clone(),
                                            delta: args,
                                            provider_metadata: None,
                                        });
                                    }
                                }
                            }

                            // Finish reason: close any open tool calls.
                            if let Some(reason) = &choice.finish_reason {
                                final_finish_reason = Some(parse_finish_reason(reason));

                                // Close any open tool calls.
                                for &idx in &tool_call_order {
                                    if let Some((id, name, args)) = tool_calls.get(&idx) {
                                        yield Ok(StreamPart::ToolInputEnd {
                                            id: id.clone(),
                                            provider_metadata: None,
                                        });
                                        let input: Value = serde_json::from_str(args)
                                            .unwrap_or_else(|_| Value::String(args.clone()));
                                        yield Ok(StreamPart::ToolCall {
                                            tool_call_id: id.clone(),
                                            tool_name: name.clone(),
                                            input,
                                            provider_executed: None,
                                            dynamic: None,
                                            provider_metadata: None,
                                        });
                                    }
                                }
                                tool_calls.clear();
                                tool_call_order.clear();
                            }
                        }
                    }
                    Err(e) => {
                        yield Ok(StreamPart::Error {
                            error: AiMuxError::Stream(e.to_string()),
                        });
                        break;
                    }
                }
            }

            // End any remaining open blocks.
            for (block_id, ended) in &content_blocks {
                if !ended {
                    // Determine if it's a reasoning or text block by prefix.
                    if block_id.starts_with("reasoning-") {
                        yield Ok(StreamPart::ReasoningEnd {
                            id: block_id.clone(),
                provider_metadata: None,
            });
                    } else {
                        yield Ok(StreamPart::TextEnd {
                            id: block_id.clone(),
                            provider_metadata: None,
                        });
                    }
                }
            }

            // Close any remaining open tool calls (no finish_reason was received).
            for &idx in &tool_call_order {
                if let Some((id, name, args)) = tool_calls.get(&idx) {
                    yield Ok(StreamPart::ToolInputEnd {
                        id: id.clone(),
                        provider_metadata: None,
                    });
                    let input: Value = serde_json::from_str(args)
                        .unwrap_or_else(|_| Value::String(args.clone()));
                    yield Ok(StreamPart::ToolCall {
                        tool_call_id: id.clone(),
                        tool_name: name.clone(),
                        input,
                        provider_executed: None,
                        dynamic: None,
                        provider_metadata: None,
                    });
                }
            }

            // Final part: Finish.
            yield Ok(StreamPart::Finish {
                finish_reason: final_finish_reason.unwrap_or(FinishReason {
                    unified: FinishReasonUnified::Other,
                    raw: None,
                }),
                usage: final_usage.unwrap_or(Usage {
                    input_tokens: aimux_core::types::TokenUsage {
                        total: Some(0),
                        no_cache: Some(0),
                        cache_read: Some(0),
                        cache_write: Some(0),
                        ..Default::default()
                    },
                    output_tokens: aimux_core::types::TokenUsage {
                        total: Some(0),
                        text: Some(0),
                        reasoning: Some(0),
                        ..Default::default()
                    },
                    raw: None,
                }),
                provider_metadata: None,
            });
        };

        Ok(StreamResult {
            stream: Box::pin(stream),
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }
}
