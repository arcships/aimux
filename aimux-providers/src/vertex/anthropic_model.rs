//! Anthropic partner models on Vertex AI — implements `LanguageModel`.
//!
//! Serves Anthropic Claude models (e.g. `claude-sonnet-4-20250514`) through the
//! Vertex AI `rawPredict` / `streamRawPredict` endpoints. These endpoints proxy
//! the standard Anthropic Messages API: the request body is an Anthropic
//! Messages request wrapped in an `anthropic_version` envelope — with the model
//! identity carried by the URL path (`publishers/anthropic/models/{model}`),
//! not the body — and the response is a verbatim Anthropic Messages response.
//!
//! This reuses the shared [`crate::anthropic::convert`] message conversion
//! logic and [`crate::anthropic::types`] response types. Only the endpoint
//! construction and authentication differ from the standard Anthropic provider,
//! and both come from the parent Vertex provider ([`super::VertexAuth`]).
//!
//! Reference: https://docs.cloud.google.com/claude-on-vertex-ai

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
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, RetryConfig, send, send_stream};
use aimux_stream::SseStream;

use crate::anthropic::convert::{build_request_body, parse_stop_reason};
use crate::anthropic::types::{AnthropicResponse, ContentBlock, StreamEvent};

use super::VertexAuth;

/// `anthropic_version` envelope value required by the Vertex AI `rawPredict` /
/// `streamRawPredict` endpoints.
const ANTHROPIC_VERTEX_VERSION: &str = "vertex-2023-10-16";

/// Google/Vertex error structure: `{ "error": { "message": "...", "status": "..." } }`.
///
/// `rawPredict` HTTP-level errors come back in the Vertex AI error shape; the
/// success body is Anthropic-shaped and is parsed separately.
const GOOGLE_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["error", "message"],
    type_path: &["error", "status"],
};

/// Configuration for an Anthropic-on-Vertex model instance.
///
/// `base_url` is the Vertex AI base URL *without* a `/publishers/{publisher}`
/// suffix (e.g. `.../projects/{project}/locations/{location}`); the publisher
/// (`anthropic`) is appended by the endpoint helpers. The parent
/// [`super::VertexProvider::anthropic_model`] strips any existing
/// `/publishers/google` suffix from its configured base URL before constructing
/// this config.
#[derive(Debug, Clone)]
pub struct VertexAnthropicConfig {
    pub base_url: String,
    pub auth: VertexAuth,
}

/// An Anthropic Claude language model served via Vertex AI `rawPredict`.
///
/// Does **not** hold an HTTP client — `http::send` / `http::send_stream` use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct VertexAnthropicModel {
    model_id: String,
    config: VertexAnthropicConfig,
}

impl VertexAnthropicModel {
    pub fn new(model_id: String, config: VertexAnthropicConfig) -> Self {
        Self { model_id, config }
    }

    /// `…/publishers/anthropic/models/{model}:rawPredict`
    fn generate_endpoint(&self) -> String {
        format!(
            "{}/publishers/anthropic/models/{}:rawPredict",
            self.config.base_url, self.model_id
        )
    }

    /// `…/publishers/anthropic/models/{model}:streamRawPredict`
    fn stream_endpoint(&self) -> String {
        format!(
            "{}/publishers/anthropic/models/{}:streamRawPredict",
            self.config.base_url, self.model_id
        )
    }

    /// Build the request headers, reusing the parent Vertex auth (Bearer token
    /// or API key). Mirrors [`super::VertexModel::build_headers`].
    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> Vec<(String, String)> {
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        match &self.config.auth {
            VertexAuth::BearerToken(token) => {
                headers.push(("Authorization".to_string(), format!("Bearer {}", token)));
            }
            VertexAuth::ApiKey(key) => {
                headers.push(("x-goog-api-key".to_string(), key.clone()));
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.push((k.clone(), v.clone()));
            }
        }
        headers
    }

    /// Wrap a standard Anthropic Messages request body in the Vertex AI
    /// `rawPredict` envelope: prepend `anthropic_version` and drop the `model`
    /// field (the model identity is carried by the URL path, not the body).
    fn wrap_raw_predict_body(&self, anthropic_body: Value) -> Value {
        let mut envelope = serde_json::Map::new();
        envelope.insert(
            "anthropic_version".to_string(),
            json!(ANTHROPIC_VERTEX_VERSION),
        );
        if let Value::Object(map) = anthropic_body {
            for (k, v) in map {
                if k == "model" {
                    continue;
                }
                envelope.insert(k, v);
            }
        }
        Value::Object(envelope)
    }
}

#[async_trait]
impl LanguageModel for VertexAnthropicModel {
    fn provider(&self) -> &str {
        "google.vertex"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let anthropic_body = build_request_body(&self.model_id, options, false)?;
        let body = self.wrap_raw_predict_body(anthropic_body);
        let url = self.generate_endpoint();
        let headers = self.build_headers(options.headers.as_ref());

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url,
                headers,
                body: HttpBody::Json(body.clone()),
            },
            RetryConfig::default(),
            &GOOGLE_ERROR_STRUCTURE,
        )
        .await?;

        let data: AnthropicResponse = serde_json::from_slice(&resp.body)
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        let mut content = Vec::new();
        for block in &data.content {
            match block {
                ContentBlock::Text { text } => {
                    content.push(GenerateContent::Text { text: text.clone(), provider_metadata: None});
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
                        provider_metadata: Some(serde_json::json!({
                            "anthropic": { "signature": signature }
                        })),
                    });
                }
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

        let usage = Usage {
            input_tokens: aimux_core::types::TokenUsage {
                total: data.usage.input_tokens,
                ..Default::default()
            },
            output_tokens: aimux_core::types::TokenUsage {
                total: data.usage.output_tokens,
                ..Default::default()
            },
            raw: None,
        };

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: Vec::new(),
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
        let anthropic_body = build_request_body(&self.model_id, options, true)?;
        let body = self.wrap_raw_predict_body(anthropic_body);
        let url = self.stream_endpoint();
        let headers = self.build_headers(options.headers.as_ref());

        let resp = send_stream(
            HttpRequest {
                method: HttpMethod::Post,
                url,
                headers,
                body: HttpBody::Json(body.clone()),
            },
            RetryConfig::default(),
            &GOOGLE_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;

        let sse_stream = SseStream::new(resp.body);

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings: vec![] });

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
                                            title: None,
                                            provider_metadata: None,
                                        });
                                        blocks.insert(index, BlockState::ToolUse {
                                            id,
                                            name,
                                            accumulated_json: String::new(),
                                        });
                                    }
                                    _ => {}
                                }
                            }
                            Ok(StreamEvent::ContentBlockDelta { index, delta }) => {
                                if let Some(text) = delta.text {
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
                                        yield Ok(StreamPart::TextStart { id, provider_metadata: None});
                                    }
                                    yield Ok(StreamPart::TextDelta {
                                        id: index.to_string(),
                                        delta: text,
                                        provider_metadata: None,
                                    });
                                }
                                if let Some(partial) = delta.partial_json {
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
                                if let Some(state) = blocks.remove(&index) {
                                    match state {
                                        BlockState::Text { started: true } => {
                                            yield Ok(StreamPart::TextEnd {
                                                id: index.to_string(),
                                                provider_metadata: None,
                                            });
                                        }
                                        BlockState::Text { started: false } => {}
                                        BlockState::Thinking { started: true } => {
                                            yield Ok(StreamPart::ReasoningEnd {
                                                id: index.to_string(),
                                                provider_metadata: None,
                                            });
                                        }
                                        BlockState::Thinking { started: false } => {}
                                        BlockState::ToolUse {
                                            id,
                                            name,
                                            accumulated_json,
                                        } => {
                                            yield Ok(StreamPart::ToolInputEnd { id: id.clone(), provider_metadata: None});
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
                                    final_usage.output_tokens = aimux_core::types::TokenUsage {
                                        total: u.output_tokens,
                                        ..Default::default()
                                    };
                                }
                            }
                            Ok(StreamEvent::MessageStop) => break,
                            Ok(StreamEvent::Error { error }) => {
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

/// Per-content-block state during streaming (mirrors the Anthropic provider).
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
