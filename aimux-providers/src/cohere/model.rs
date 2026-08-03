//! Cohere language model — implements `LanguageModel` trait.
//!
//! Mirrors the TS `cohere-chat-language-model.ts`. Cohere uses its own message
//! format (not OpenAI-compatible) and streams named SSE events
//! (`event: type\ndata: json`).

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

use aimux_provider_utils::response::ErrorStructure;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, send_stream_timed, send_timed,
};
use aimux_stream::SseStream;

use super::CohereConfig;
use super::convert::{build_request_body, parse_finish_reason};
use super::types::{ChatResponse, StreamEvent, TokenPair};

/// Cohere error structure: `{ "message": "..." }` (flat, no `error` wrapper).
const COHERE_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["message"],
    type_path: &[],
};

/// A Cohere language model.
pub struct CohereModel {
    model_id: String,
    config: CohereConfig,
}

impl CohereModel {
    pub fn new(model_id: String, config: CohereConfig) -> Self {
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
        format!("{}/chat", self.config.base_url)
    }
}

// ── Usage conversion ─────────────────────────────────────────────────────────

/// Convert Cohere `tokens` usage into the core `Usage` type.
///
/// Mirrors the TS `convertCohereUsage`:
/// - `input.total = input_tokens`, `input.noCache = input_tokens`
/// - `output.total = output_tokens`
fn convert_usage(tokens: &TokenPair) -> Usage {
    Usage {
        input_tokens: aimux_core::types::TokenUsage {
            total: Some(tokens.input_tokens),
            no_cache: Some(tokens.input_tokens),
            cache_read: None,
            cache_write: None,
            ..Default::default()
        },
        output_tokens: aimux_core::types::TokenUsage {
            total: Some(tokens.output_tokens),
            ..Default::default()
        },
        raw: None,
    }
}

// ── Pending tool call accumulator (streaming) ────────────────────────────────

struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

// ── LanguageModel impl ───────────────────────────────────────────────────────

#[async_trait]
impl LanguageModel for CohereModel {
    fn provider(&self) -> &str {
        "cohere"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let body_result = build_request_body(&self.model_id, options, false);
        let body = body_result.body.clone();
        let headers = self.build_headers(options.headers.as_ref());

        let resp = send_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                body: HttpBody::Json(body.clone()),

                abort_signal: options.abort_signal.clone(),
            },
            RetryConfig::default(),
            &COHERE_ERROR_STRUCTURE,
            options.timeout.map(Into::into),
        )
        .await?;

        let response_headers = resp.headers;
        let data: ChatResponse =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

        // Build content array.
        let mut content = Vec::new();

        // Content items (text and thinking).
        if let Some(items) = &data.message.content {
            for item in items {
                match item {
                    super::types::ContentItem::Text { text } => {
                        if !text.is_empty() {
                            content.push(GenerateContent::Text {
                                text: text.clone(),
                                provider_metadata: None,
                            });
                        }
                    }
                    super::types::ContentItem::Thinking { thinking } => {
                        // Mirrors TS: a `thinking` content item becomes a
                        // `reasoning` content item. Empty thinking is dropped.
                        if !thinking.is_empty() {
                            content.push(GenerateContent::Reasoning {
                                text: thinking.clone(),
                                provider_metadata: None,
                            });
                        }
                    }
                }
            }
        }

        // Citations (RAG) → Source content items.
        //
        // Mirrors TS `cohere-chat-language-model.ts`: each citation becomes a
        // `{ type: 'source', sourceType: 'document', title, providerMetadata:
        // { cohere: { start, end, text, sources, citationType } } }` content
        // item. The Rust `GenerateContent::Source` variant carries no
        // `mediaType` / `providerMetadata`, so only `source_type` and `title`
        // are preserved here; the rich per-citation metadata (start/end/text/
        // sources/citationType) is dropped — a documented data-model gap.
        if let Some(citations) = &data.message.citations {
            for (i, citation) in citations.iter().enumerate() {
                let title = citation
                    .get("sources")
                    .and_then(|s| s.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|src| src.get("document"))
                    .and_then(|d| d.get("title"))
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "Document".to_string());
                content.push(GenerateContent::Source {
                    id: format!("citation-{}", i),
                    source_type: "document".to_string(),
                    url: None,
                    title: Some(title),
                    provider_metadata: None,
                });
            }
        }

        // Tool calls.
        if let Some(tool_calls) = &data.message.tool_calls {
            for tc in tool_calls {
                // Cohere sometimes returns "null" for empty arguments.
                let args_str = tc.function.arguments.replacen("null", "{}", 1);
                let input: Value = serde_json::from_str(&args_str)
                    .unwrap_or_else(|_| Value::String(tc.function.arguments.clone()));
                content.push(GenerateContent::ToolCall {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.function.name.clone(),
                    input,
                    provider_executed: None,
                    dynamic: None,
                    thought_signature: None,
                    provider_metadata: None,
                });
            }
        }

        let finish_reason = parse_finish_reason(&data.finish_reason);
        let usage = convert_usage(&data.usage.tokens);

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: body_result.warnings,
            provider_metadata: None,
            response: ResponseMetadata {
                id: data.generation_id,
                timestamp: None,
                model_id: None,
            },
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let body_result = build_request_body(&self.model_id, options, true);
        let body = body_result.body.clone();
        let headers = self.build_headers(options.headers.as_ref());

        let resp = send_stream_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                body: HttpBody::Json(body.clone()),

                abort_signal: options.abort_signal.clone(),
            },
            RetryConfig::default(),
            &COHERE_ERROR_STRUCTURE,
            options.timeout.map(Into::into),
        )
        .await?;

        let response_headers = resp.headers;
        let sse_stream = SseStream::new(resp.body);

        let stream_warnings = body_result.warnings;
        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings: stream_warnings });

            let mut final_usage = Usage::default();
            let mut final_finish_reason: Option<FinishReason> = None;
            let mut pending_tool_call: Option<PendingToolCall> = None;
            let mut is_reasoning = false;
            let mut stream_errored = false;

            let mut sse_iter = Box::pin(sse_stream);

            while let Some(event) = sse_iter.next().await {
                if stream_errored {
                    break;
                }

                match event {
                    Ok(sse_event) => {
                        // Parse the JSON payload.
                        let parsed: StreamEvent = match serde_json::from_str(&sse_event.data) {
                            Ok(e) => e,
                            Err(e) => {
                                // Unparsable chunk — emit Error.
                                yield Ok(StreamPart::Error {
                                    error: AiMuxError::Json(e.to_string()),
                                });
                                stream_errored = true;
                                break;
                            }
                        };

                        match parsed.event_type.as_str() {
                            "message-start" => {
                                yield Ok(StreamPart::ResponseMetadata {
                                    id: parsed.id.clone(),
                                    timestamp: None,
                                    model_id: None,
                                });
                            }

                            "content-start" => {
                                let idx = parsed.index.unwrap_or(0);
                                let content_type = parsed
                                    .delta
                                    .as_ref()
                                    .and_then(|d| d.message.as_ref())
                                    .and_then(|m| m.content.as_ref())
                                    .and_then(|c| c.get("type"))
                                    .and_then(|t| t.as_str());

                                if content_type == Some("thinking") {
                                    is_reasoning = true;
                                    yield Ok(StreamPart::ReasoningStart {
                                        id: format!("reasoning-{}", idx),
                                        provider_metadata: None,
                                    });
                                } else {
                                    yield Ok(StreamPart::TextStart {
                                        id: format!("{}", idx),
                                        provider_metadata: None,
                                    });
                                }
                            }

                            "content-delta" => {
                                let idx = parsed.index.unwrap_or(0);
                                let content = parsed
                                    .delta
                                    .as_ref()
                                    .and_then(|d| d.message.as_ref())
                                    .and_then(|m| m.content.as_ref());

                                if let Some(content) = content {
                                    // Thinking delta.
                                    if let Some(thinking) =
                                        content.get("thinking").and_then(|t| t.as_str())
                                    {
                                        yield Ok(StreamPart::ReasoningDelta {
                                            id: format!("reasoning-{}", idx),
                                            delta: thinking.to_string(),
                                            provider_metadata: None,
                                        });
                                    }
                                    // Text delta.
                                    else if let Some(text) =
                                        content.get("text").and_then(|t| t.as_str())
                                    {
                                        yield Ok(StreamPart::TextDelta {
                                            id: format!("{}", idx),
                                            delta: text.to_string(),
                                            provider_metadata: None,
                                        });
                                    }
                                }
                            }

                            "content-end" => {
                                let idx = parsed.index.unwrap_or(0);
                                if is_reasoning {
                                    yield Ok(StreamPart::ReasoningEnd {
                                        id: format!("reasoning-{}", idx),
                                        provider_metadata: None,
                                    });
                                    is_reasoning = false;
                                } else {
                                    yield Ok(StreamPart::TextEnd {
                                        id: format!("{}", idx),
                                        provider_metadata: None,
                                    });
                                }
                            }

                            "tool-plan-delta" => {
                                // Tool plan deltas are not emitted as stream parts
                                // (no corresponding variant). Silently consume.
                            }

                            "tool-call-start" => {
                                let tc = parsed
                                    .delta
                                    .as_ref()
                                    .and_then(|d| d.message.as_ref())
                                    .and_then(|m| m.tool_calls.as_ref());

                                if let Some(tc) = tc {
                                    let id = tc
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let name = tc
                                        .get("function")
                                        .and_then(|f| f.get("name"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let initial_args = tc
                                        .get("function")
                                        .and_then(|f| f.get("arguments"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();

                                    pending_tool_call = Some(PendingToolCall {
                                        id: id.clone(),
                                        name: name.clone(),
                                        arguments: initial_args.clone(),
                                    });

                                    yield Ok(StreamPart::ToolInputStart {
                                        id: id.clone(),
                                        tool_name: name,
                                        provider_executed: None,
                                        dynamic: None,
                                        title: None,
                                        provider_metadata: None,
                                    });

                                    if !initial_args.is_empty() {
                                        yield Ok(StreamPart::ToolInputDelta {
                                            id,
                                            delta: initial_args,
                                            provider_metadata: None,
                                        });
                                    }
                                }
                            }

                            "tool-call-delta" => {
                                if let Some(ptc) = &mut pending_tool_call {
                                    let args_delta = parsed
                                        .delta
                                        .as_ref()
                                        .and_then(|d| d.message.as_ref())
                                        .and_then(|m| m.tool_calls.as_ref())
                                        .and_then(|tc| tc.get("function"))
                                        .and_then(|f| f.get("arguments"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");

                                    if !args_delta.is_empty() {
                                        ptc.arguments.push_str(args_delta);
                                        yield Ok(StreamPart::ToolInputDelta {
                                            id: ptc.id.clone(),
                                            delta: args_delta.to_string(),
                                            provider_metadata: None,
                                        });
                                    }
                                }
                            }

                            "tool-call-end" => {
                                if let Some(ptc) = pending_tool_call.take() {
                                    yield Ok(StreamPart::ToolInputEnd {
                                        id: ptc.id.clone(),
                                        provider_metadata: None,
                                    });

                                    let trimmed = ptc.arguments.trim();
                                    let input: Value = if trimmed.is_empty() {
                                        json!({})
                                    } else {
                                        serde_json::from_str(trimmed)
                                            .unwrap_or_else(|_| {
                                                Value::String(ptc.arguments.clone())
                                            })
                                    };

                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: ptc.id,
                                        tool_name: ptc.name,
                                        input,
                                        provider_executed: None,
                                        dynamic: None,
                                        thought_signature: None,
                                        provider_metadata: None,
                                    });
                                }
                            }

                            "message-end" => {
                                if let Some(delta) = &parsed.delta {
                                    if let Some(reason) = &delta.finish_reason {
                                        final_finish_reason = Some(parse_finish_reason(reason));
                                    }
                                    if let Some(usage) = &delta.usage {
                                        final_usage = convert_usage(&usage.tokens);
                                    }
                                }
                            }

                            // citation-start, citation-end, and any unknown
                            // event types are silently consumed.
                            _ => {}
                        }
                    }
                    Err(e) => {
                        yield Ok(StreamPart::Error {
                            error: AiMuxError::Stream(e.to_string()),
                        });
                        stream_errored = true;
                        break;
                    }
                }
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
                provider_metadata: Some(serde_json::json!({ "cohere": {} })),
            });
        };

        Ok(StreamResult {
            stream: Box::pin(stream),
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }
}
