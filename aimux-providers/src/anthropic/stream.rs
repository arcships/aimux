//! Shared Anthropic request/streaming core.
//!
//! The standard Anthropic provider ([`crate::anthropic::model`]) and the
//! Anthropic-AWS provider ([`crate::anthropic_aws::model`]) speak the exact
//! same Messages API. The only differences are endpoint, authentication
//! (Bearer/x-api-key vs AWS SigV4) and the wire body encoding (`Json` vs
//! `Bytes`, the latter preventing re-serialization from invalidating the SigV4
//! signature).
//!
//! This module factors out the parts that are identical across both providers:
//! - [`build_anthropic_request`] — serialize the body once, build auth headers
//!   via a closure, and choose the wire encoding.
//! - [`anthropic_generate_core`] — non-streaming send + response parsing.
//! - [`anthropic_stream_core`] — streaming send + the Anthropic SSE event loop.
//! - [`parse_anthropic_content`] — shared content-block → `GenerateContent`
//!   mapping used by the non-streaming path.

use std::collections::HashMap;

use futures::StreamExt;

use aimux_core::error::AiMuxError;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::shared::AbortSignal;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::Warning;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, TokenUsage, Usage};
use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RequestTimeout, RetryConfig, send_stream_timed, send_timed,
};
use aimux_stream::SseStream;
use serde_json::json;

use super::convert::parse_stop_reason;
use super::types::{AnthropicResponse, ContentBlock, StreamEvent};

/// How the request body is sent over the wire.
#[derive(Debug, Clone, Copy)]
pub(crate) enum BodyEncoding {
    /// Send as `HttpBody::Json` — the HTTP layer re-serializes the body value.
    /// Standard Anthropic path.
    Json,
    /// Send as `HttpBody::Bytes` using the *exact* bytes the headers were built
    /// over. AWS SigV4 path — prevents re-serialization from breaking the
    /// signature (the signature is computed over `body_bytes`, and
    /// `body_bytes` is what is sent).
    Bytes,
}

/// Build an Anthropic `HttpRequest`.
///
/// The body is serialized to bytes once so the same bytes are used both for
/// header construction (the `build_headers` closure — needed by SigV4, which
/// signs the request body) and for the wire body in the [`BodyEncoding::Bytes`]
/// path. For [`BodyEncoding::Json`] the closure receives the serialized bytes
/// but the body is sent as a re-serializable `Value` (the signature is not
/// involved, so re-serialization is harmless).
fn build_anthropic_request(
    endpoint: &str,
    body: &serde_json::Value,
    build_headers: &impl Fn(&[u8], &str) -> Result<Vec<(String, String)>, AiMuxError>,
    body_encoding: BodyEncoding,
    abort_signal: Option<AbortSignal>,
) -> Result<HttpRequest, AiMuxError> {
    // Serialize once; the Bytes path sends these exact bytes and the closure
    // signs over them, guaranteeing signature/body agreement.
    let body_bytes = serde_json::to_vec(body).map_err(|e| AiMuxError::Json(e.to_string()))?;
    let headers = build_headers(&body_bytes, endpoint)?;

    let http_body = match body_encoding {
        BodyEncoding::Json => HttpBody::Json(body.clone()),
        BodyEncoding::Bytes => HttpBody::Bytes(body_bytes, "application/json".to_string()),
    };

    Ok(HttpRequest {
        method: HttpMethod::Post,
        url: endpoint.to_string(),
        headers,
        body: http_body,
        abort_signal,
    })
}

/// Map Anthropic response content blocks into `GenerateContent` items.
///
/// Text / tool_use / thinking / server_tool_use blocks are surfaced; result
/// blocks (web_search_tool_result, code_execution_tool_result, ...) and any
/// unknown block type are intentionally dropped — `GenerateContent` has no
/// provider-tool-result variant, and none of the current tests assert on their
/// content.
fn parse_anthropic_content(blocks: &[ContentBlock]) -> Vec<GenerateContent> {
    let mut content = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Text { text } => {
                content.push(GenerateContent::Text {
                    text: text.clone(),
                    provider_metadata: None,
                });
            }
            ContentBlock::ToolUse { id, name, input } => {
                content.push(GenerateContent::ToolCall {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    input: input.clone(),
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: None,
                });
            }
            ContentBlock::Thinking {
                thinking,
                signature,
            } => {
                content.push(GenerateContent::Reasoning {
                    text: thinking.clone(),
                    provider_metadata: Some(json!({
                        "anthropic": { "signature": signature }
                    })),
                });
            }
            // Provider-executed (server-side) tool calls are surfaced as tool
            // calls so they round-trip on follow-up turns.
            ContentBlock::ServerToolUse { id, name, input } => {
                content.push(GenerateContent::ToolCall {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    input: input.clone(),
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: None,
                });
            }
            _ => {}
        }
    }
    content
}

/// Shared non-streaming Anthropic core.
///
/// Builds and sends the request (using `build_headers` for auth and
/// `body_encoding` for the wire body), then parses the `AnthropicResponse` into
/// a `GenerateResult`. The usage breakdown (reasoning / text token split) is the
/// full version, so both the standard and the AWS provider report the same
/// detailed token accounting.
pub(crate) async fn anthropic_generate_core(
    endpoint: &str,
    retry_config: RetryConfig,
    body: serde_json::Value,
    warnings: Vec<Warning>,
    build_headers: impl Fn(&[u8], &str) -> Result<Vec<(String, String)>, AiMuxError>,
    body_encoding: BodyEncoding,
    abort_signal: Option<AbortSignal>,
    timeout: Option<RequestTimeout>,
) -> Result<GenerateResult, AiMuxError> {
    let request = build_anthropic_request(endpoint, &body, &build_headers, body_encoding, abort_signal)?;
    let resp = send_timed(request, retry_config, &DEFAULT_ERROR_STRUCTURE, timeout).await?;

    let data: AnthropicResponse =
        serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

    let content = parse_anthropic_content(&data.content);

    let finish_reason = data
        .stop_reason
        .as_deref()
        .map(parse_stop_reason)
        .unwrap_or(FinishReason {
            unified: FinishReasonUnified::Other,
            raw: None,
        });

    let reasoning_tokens = data
        .usage
        .output_tokens_details
        .as_ref()
        .and_then(|d| d.thinking_tokens);
    let output_total = data.usage.output_tokens;
    let text_tokens = reasoning_tokens
        .zip(output_total)
        .map(|(r, t)| t.saturating_sub(r));

    let usage = Usage {
        input_tokens: TokenUsage {
            total: data.usage.input_tokens,
            ..Default::default()
        },
        output_tokens: TokenUsage {
            total: output_total,
            text: text_tokens,
            reasoning: reasoning_tokens,
            ..Default::default()
        },
        raw: None,
    };

    Ok(GenerateResult {
        content,
        finish_reason,
        usage,
        warnings,
        provider_metadata: None,
        response: ResponseMetadata {
            id: Some(data.id),
            timestamp: None,
            model_id: Some(data.model),
        },
        request_body: Some(body),
        response_headers: None,
    })
}

/// Per-content-block state during streaming.
///
/// Text blocks track whether a `TextStart` has been emitted; tool_use blocks
/// accumulate the `input_json_delta` partial-json fragments so the final
/// `ToolCall` can carry the parsed JSON object; thinking blocks track whether a
/// `ReasoningStart` has been emitted.
enum BlockState {
    Text {
        started: bool,
    },
    ToolUse {
        id: String,
        name: String,
        accumulated_json: String,
    },
    Thinking {
        started: bool,
    },
}

/// Shared Anthropic streaming core.
///
/// Builds and sends the request (using `build_headers` for auth and
/// `body_encoding` for the wire body), then runs the Anthropic SSE event loop
/// to produce a `StreamResult`.
///
/// `build_headers` receives the serialized body bytes and the endpoint URL —
/// the standard path returns a Bearer/x-api-key header set (ignoring the body),
/// the AWS path returns a SigV4-signed header set (signing over the body).
pub(crate) async fn anthropic_stream_core(
    endpoint: &str,
    retry_config: RetryConfig,
    body: serde_json::Value,
    warnings: Vec<Warning>,
    build_headers: impl Fn(&[u8], &str) -> Result<Vec<(String, String)>, AiMuxError>,
    body_encoding: BodyEncoding,
    abort_signal: Option<AbortSignal>,
    timeout: Option<RequestTimeout>,
) -> Result<StreamResult, AiMuxError> {
    let request = build_anthropic_request(endpoint, &body, &build_headers, body_encoding, abort_signal)?;
    let resp = send_stream_timed(request, retry_config, &DEFAULT_ERROR_STRUCTURE, timeout).await?;

    let response_headers = resp.headers;
    let sse_stream = SseStream::new(resp.body);

    let stream = async_stream::stream! {
        // First part: StreamStart.
        yield Ok(StreamPart::StreamStart { warnings });

        let mut sse = sse_stream;
        let mut blocks: HashMap<usize, BlockState> = HashMap::new();
        let mut final_usage = Usage::default();
        let mut final_finish_reason: Option<FinishReason> = None;
        let mut response_meta_emitted = false;

        while let Some(event) = sse.next().await {
            match event {
                Ok(sse_event) => {
                    match serde_json::from_str::<StreamEvent>(&sse_event.data) {
                        Ok(StreamEvent::MessageStart { message }) => {
                            if let Some(usage) = &message.usage {
                                final_usage = Usage {
                                    input_tokens: TokenUsage {
                                        total: usage.input_tokens,
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                };
                            }
                            if !response_meta_emitted {
                                yield Ok(StreamPart::ResponseMetadata {
                                    id: Some(message.id.clone()),
                                    timestamp: None,
                                    model_id: Some(message.model.clone()),
                                });
                                response_meta_emitted = true;
                            }
                        }
                        Ok(StreamEvent::ContentBlockStart { index, content_block }) => {
                            match content_block {
                                ContentBlock::Text { .. } => {
                                    blocks.insert(index, BlockState::Text { started: false });
                                }
                                ContentBlock::Thinking { .. } => {
                                    blocks.insert(index, BlockState::Thinking { started: false });
                                }
                                ContentBlock::ToolUse { id, name, .. } => {
                                    yield Ok(StreamPart::ToolInputStart {
                                        id: id.clone(),
                                        tool_name: name.clone(),
                                        provider_executed: None,
                                        dynamic: None,
                                        title: None,
                                        provider_metadata: None,
                                    });
                                    blocks.insert(index, BlockState::ToolUse {
                                        id,
                                        name,
                                        accumulated_json: String::new(),
                                    });
                                }
                                // Server-side / provider-executed tool blocks
                                // (server_tool_use, mcp_tool_use, result
                                // blocks, ...) are not yet streamed as
                                // first-class parts; ignore them here so an
                                // unknown block type never aborts the stream.
                                _ => {}
                            }
                        }
                        Ok(StreamEvent::ContentBlockDelta { index, delta }) => {
                            if let Some(text) = delta.text {
                                // Start the text segment on the first delta. The
                                // text id is the stringified content-block
                                // index, matching the TS SDK.
                                let start_id: Option<String> = match blocks.get_mut(&index) {
                                    Some(BlockState::Text { started: false }) => {
                                        if let Some(BlockState::Text { started }) =
                                            blocks.get_mut(&index)
                                        {
                                            *started = true;
                                        }
                                        Some(index.to_string())
                                    }
                                    _ => None,
                                };
                                if let Some(id) = start_id {
                                    yield Ok(StreamPart::TextStart {
                                        id,
                                        provider_metadata: None,
                                    });
                                }
                                yield Ok(StreamPart::TextDelta {
                                    id: index.to_string(),
                                    delta: text,
                                    provider_metadata: None,
                                });
                            }
                            if let Some(partial) = delta.partial_json {
                                // Accumulate the partial JSON fragment and emit
                                // a ToolInputDelta. Empty fragments (the
                                // leading `input_json_delta` with
                                // `partial_json: ""`) are skipped, matching the
                                // TS SDK.
                                let delta_id: Option<String> = match blocks.get_mut(&index) {
                                    Some(BlockState::ToolUse {
                                        id,
                                        accumulated_json,
                                        ..
                                    }) if !partial.is_empty() => {
                                        accumulated_json.push_str(&partial);
                                        Some(id.clone())
                                    }
                                    _ => None,
                                };
                                if let Some(id) = delta_id {
                                    yield Ok(StreamPart::ToolInputDelta {
                                        id,
                                        delta: partial,
                                        provider_metadata: None,
                                    });
                                }
                            }
                            if let Some(thinking) = delta.thinking {
                                // Start the reasoning segment on the first
                                // thinking delta. The id is the stringified
                                // content-block index, matching the TS SDK.
                                let start_id: Option<String> = match blocks.get_mut(&index) {
                                    Some(BlockState::Thinking { started: false }) => {
                                        if let Some(BlockState::Thinking { started }) =
                                            blocks.get_mut(&index)
                                        {
                                            *started = true;
                                        }
                                        Some(index.to_string())
                                    }
                                    _ => None,
                                };
                                if let Some(id) = start_id {
                                    yield Ok(StreamPart::ReasoningStart {
                                        id,
                                        provider_metadata: None,
                                    });
                                }
                                yield Ok(StreamPart::ReasoningDelta {
                                    id: index.to_string(),
                                    delta: thinking,
                                    provider_metadata: None,
                                });
                            }
                        }
                        Ok(StreamEvent::ContentBlockStop { index }) => {
                            // Removing the block releases the borrow before any
                            // yield.
                            if let Some(state) = blocks.remove(&index) {
                                match state {
                                    BlockState::Text { started: true } => {
                                        yield Ok(StreamPart::TextEnd {
                                            id: index.to_string(),
                                            provider_metadata: None,
                                        });
                                    }
                                    BlockState::Text { started: false } => {
                                        // Text block with no deltas — nothing to emit.
                                    }
                                    BlockState::Thinking { started: true } => {
                                        yield Ok(StreamPart::ReasoningEnd {
                                            id: index.to_string(),
                                            provider_metadata: None,
                                        });
                                    }
                                    BlockState::Thinking { started: false } => {
                                        // Thinking block with no deltas — nothing to emit.
                                    }
                                    BlockState::ToolUse {
                                        id,
                                        name,
                                        accumulated_json,
                                    } => {
                                        yield Ok(StreamPart::ToolInputEnd {
                                            id: id.clone(),
                                            provider_metadata: None,
                                        });
                                        let input: serde_json::Value = if accumulated_json
                                            .is_empty()
                                        {
                                            serde_json::json!({})
                                        } else {
                                            serde_json::from_str(&accumulated_json)
                                                .unwrap_or(serde_json::json!({}))
                                        };
                                        yield Ok(StreamPart::ToolCall {
                                            tool_call_id: id,
                                            tool_name: name,
                                            input,
                                            provider_executed: None,
                                            dynamic: None,
                                            provider_metadata: None,
                                        });
                                    }
                                }
                            }
                        }
                        Ok(StreamEvent::MessageDelta { delta, usage }) => {
                            if let Some(reason) = delta.stop_reason {
                                final_finish_reason = Some(parse_stop_reason(&reason));
                            }
                            if let Some(u) = usage {
                                let reasoning_tokens = u
                                    .output_tokens_details
                                    .as_ref()
                                    .and_then(|d| d.thinking_tokens);
                                let output_total = u.output_tokens;
                                let text_tokens = reasoning_tokens
                                    .zip(output_total)
                                    .map(|(r, t)| t.saturating_sub(r));
                                final_usage.output_tokens = TokenUsage {
                                    total: output_total,
                                    text: text_tokens,
                                    reasoning: reasoning_tokens,
                                    ..Default::default()
                                };
                            }
                        }
                        Ok(StreamEvent::MessageStop) => break,
                        Ok(StreamEvent::Error { error }) => {
                            // Surface Anthropic in-stream errors (e.g.
                            // overloaded_error) as a `StreamPart::Error` and
                            // stop the stream, mirroring the TS "forward error
                            // chunks" / "forward overloaded error" behaviour.
                            yield Ok(StreamPart::Error {
                                error: AiMuxError::Provider(error.message),
                            });
                            return;
                        }
                        Ok(_) | Err(_) => {}
                    }
                }
                Err(e) => {
                    yield Ok(StreamPart::Error {
                        error: AiMuxError::Stream(e.to_string()),
                    });
                    return;
                }
            }
        }

        // Final part: Finish.
        yield Ok(StreamPart::Finish {
            finish_reason: final_finish_reason.unwrap_or(FinishReason {
                unified: FinishReasonUnified::Stop,
                raw: None,
            }),
            usage: final_usage,
            provider_metadata: None,
        });
    };

    Ok(StreamResult {
        stream: Box::pin(stream),
        request_body: Some(body),
        response_headers: Some(response_headers),
    })
}
