//! Anthropic language model — implements `LanguageModel` trait.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::TokenUsage;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

use aimux_provider_utils::response::{DEFAULT_ERROR_STRUCTURE, parse_provider_error};
use aimux_stream::SseStream;
use serde_json::json;

use super::AnthropicConfig;
use super::convert::{build_request_body_with_warnings, parse_stop_reason};
use super::types::{AnthropicResponse, ContentBlock, StreamEvent};

/// An Anthropic language model (e.g. `claude-sonnet-4-20250514`).
pub struct AnthropicModel {
    model_id: String,
    config: AnthropicConfig,
    client: Client,
}

impl AnthropicModel {
    pub fn new(model_id: String, config: AnthropicConfig, client: Client) -> Self {
        Self {
            model_id,
            config,
            client,
        }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "anthropic-version".to_string(),
            self.config.api_version.clone(),
        );
        // Auth: prefer bearer token, fall back to x-api-key.
        if let Some(token) = &self.config.auth_token {
            headers.insert("authorization".to_string(), format!("Bearer {token}"));
        } else {
            headers.insert("x-api-key".to_string(), self.config.api_key.clone());
        }
        // Extra config-level headers (lower precedence than per-call headers).
        if let Some(cfg_headers) = &self.config.headers {
            for (k, v) in cfg_headers {
                headers.insert(k.clone(), v.clone());
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/messages", self.config.base_url)
    }
}

#[async_trait]
impl LanguageModel for AnthropicModel {
    fn provider(&self) -> &str {
        &self.config.name
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let req = build_request_body_with_warnings(&self.model_id, options, false)?;
        let body = req.body;
        let warnings = req.warnings;
        let mut headers = self.build_headers(options.headers.as_ref());
        if !req.betas.is_empty() {
            headers.insert(
                "anthropic-beta".to_string(),
                req.betas.iter().cloned().collect::<Vec<_>>().join(","),
            );
        }

        let resp = self
            .client
            .post(self.endpoint())
            .header("Content-Type", "application/json")
            .headers(reqwest::header::HeaderMap::from_iter(
                headers.iter().filter_map(|(k, v)| {
                    reqwest::header::HeaderName::try_from(k)
                        .ok()
                        .zip(reqwest::header::HeaderValue::try_from(v).ok())
                }),
            ))
            .json(&body)
            .send()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(parse_provider_error(
                status.as_u16(),
                &text,
                &DEFAULT_ERROR_STRUCTURE,
            ));
        }

        let data: AnthropicResponse = resp
            .json()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        // Build content array.
        let mut content = Vec::new();
        for block in &data.content {
            match block {
                ContentBlock::Text { text } => {
                    content.push(GenerateContent::Text { text: text.clone() });
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
                // Provider-executed (server-side) tool calls are surfaced as
                // tool calls so they round-trip on follow-up turns. The result
                // blocks (web_search_tool_result, code_execution_tool_result,
                // ...) and any unknown block type are intentionally dropped:
                // `GenerateContent` has no provider-tool-result variant, and
                // none of the current tests assert on their content.
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

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let req = build_request_body_with_warnings(&self.model_id, options, true)?;
        let body = req.body;
        let warnings = req.warnings;
        let mut headers = self.build_headers(options.headers.as_ref());
        if !req.betas.is_empty() {
            headers.insert(
                "anthropic-beta".to_string(),
                req.betas.iter().cloned().collect::<Vec<_>>().join(","),
            );
        }

        let resp = self
            .client
            .post(self.endpoint())
            .header("Content-Type", "application/json")
            .headers(reqwest::header::HeaderMap::from_iter(
                headers.iter().filter_map(|(k, v)| {
                    reqwest::header::HeaderName::try_from(k)
                        .ok()
                        .zip(reqwest::header::HeaderValue::try_from(v).ok())
                }),
            ))
            .json(&body)
            .send()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(parse_provider_error(
                status.as_u16(),
                &text,
                &DEFAULT_ERROR_STRUCTURE,
            ));
        }

        let response_headers = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect::<HashMap<_, _>>();

        let byte_stream = resp.bytes_stream();
        let sse_stream = SseStream::new(byte_stream);

        // Per-content-block state, keyed by the Anthropic `index` field. Text
        // blocks track whether a `TextStart` has been emitted; tool_use blocks
        // accumulate the `input_json_delta` partial-json fragments so the final
        // `ToolCall` can carry the parsed JSON object; thinking blocks track
        // whether a `ReasoningStart` has been emitted.
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
                                        input_tokens: aimux_core::types::TokenUsage {
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
                                        });
                                        blocks.insert(index, BlockState::ToolUse {
                                            id,
                                            name,
                                            accumulated_json: String::new(),
                                        });
                                    }
                                    // Server-side / provider-executed tool
                                    // blocks (server_tool_use, mcp_tool_use,
                                    // result blocks, ...) are not yet streamed
                                    // as first-class parts; ignore them here so
                                    // an unknown block type never aborts the
                                    // stream.
                                    _ => {}
                                }
                            }
                            Ok(StreamEvent::ContentBlockDelta { index, delta }) => {
                                if let Some(text) = delta.text {
                                    // Start the text segment on the first delta. The text
                                    // id is the stringified content-block index, matching
                                    // the TS SDK.
                                    let start_id: Option<String> = match blocks.get_mut(&index)
                                    {
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
                                        yield Ok(StreamPart::TextStart { id });
                                    }
                                    yield Ok(StreamPart::TextDelta {
                                        id: index.to_string(),
                                        delta: text,
                                    });
                                }
                                if let Some(partial) = delta.partial_json {
                                    // Accumulate the partial JSON fragment and emit a
                                    // ToolInputDelta. Empty fragments (the leading
                                    // `input_json_delta` with `partial_json: ""`) are
                                    // skipped, matching the TS SDK.
                                    let delta_id: Option<String> = match blocks.get_mut(&index)
                                    {
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
                                        });
                                    }
                                }
                                if let Some(thinking) = delta.thinking {
                                    // Start the reasoning segment on the first
                                    // thinking delta. The id is the stringified
                                    // content-block index, matching the TS SDK.
                                    let start_id: Option<String> = match blocks.get_mut(&index)
                                    {
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
                                        yield Ok(StreamPart::ReasoningStart { id,
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
                                // Removing the block releases the borrow before any yield.
                                if let Some(state) = blocks.remove(&index) {
                                    match state {
                                        BlockState::Text { started: true } => {
                                            yield Ok(StreamPart::TextEnd {
                                                id: index.to_string(),
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
                                            yield Ok(StreamPart::ToolInputEnd { id: id.clone() });
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
                                // stop the stream, mirroring the TS "forward
                                // error chunks" / "forward overloaded error"
                                // behaviour.
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
}
