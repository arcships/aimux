//! Anthropic-AWS language model — implements `LanguageModel`.
//!
//! Reuses the shared [`crate::anthropic::convert`] message conversion logic and
//! [`crate::anthropic::types`] response types. Only the endpoint and
//! authentication differ from the standard Anthropic provider.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, RetryConfig, send, send_stream};
use aimux_stream::SseStream;

use crate::anthropic::convert::{build_request_body, parse_stop_reason};
use crate::anthropic::types::{AnthropicResponse, ContentBlock, StreamEvent};
use crate::bedrock::sigv4::sign_request;

use super::AnthropicAwsAuth;

/// Configuration for an Anthropic-AWS model instance.
#[derive(Debug, Clone)]
pub struct AnthropicAwsConfig {
    pub base_url: String,
    pub auth: AnthropicAwsAuth,
    pub api_version: String,
    pub workspace_id: Option<String>,
}

/// An Anthropic-AWS language model (e.g. `claude-sonnet-4-20250514`).
pub struct AnthropicAwsModel {
    model_id: String,
    config: AnthropicAwsConfig,
}

impl AnthropicAwsModel {
    pub fn new(model_id: String, config: AnthropicAwsConfig) -> Self {
        Self { model_id, config }
    }

    fn endpoint(&self) -> String {
        format!("{}/messages", self.config.base_url)
    }

    fn build_headers(
        &self,
        body: &str,
        url: &str,
        extra: Option<&HashMap<String, String>>,
    ) -> Result<Vec<(String, String)>, AiMuxError> {
        let mut base_headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            (
                "anthropic-version".to_string(),
                self.config.api_version.clone(),
            ),
        ];

        if let Some(ref ws) = self.config.workspace_id {
            base_headers.push(("anthropic-workspace-id".to_string(), ws.clone()));
        }

        if let Some(extra) = extra {
            for (k, v) in extra {
                base_headers.push((k.clone(), v.clone()));
            }
        }

        match &self.config.auth {
            AnthropicAwsAuth::ApiKey(key) => {
                base_headers.push(("x-api-key".to_string(), key.clone()));
                Ok(base_headers)
            }
            AnthropicAwsAuth::SigV4(creds) => {
                // Sign the request with AWS SigV4 (service: aws-external-anthropic).
                let extra_for_signing: Vec<(String, String)> = base_headers
                    .iter()
                    .filter(|(k, _)| k != "Content-Type")
                    .cloned()
                    .collect();

                let signed = sign_request(
                    creds,
                    "aws-external-anthropic",
                    "POST",
                    url,
                    body,
                    &extra_for_signing,
                );

                let mut headers =
                    vec![("Content-Type".to_string(), "application/json".to_string())];
                for (k, v) in &signed.headers {
                    headers.push((k.clone(), v.clone()));
                }
                Ok(headers)
            }
        }
    }
}

#[async_trait]
impl LanguageModel for AnthropicAwsModel {
    fn provider(&self) -> &str {
        "anthropic-aws"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let body = build_request_body(&self.model_id, options, false)?;
        let body_str = serde_json::to_string(&body).unwrap_or_default();
        let url = self.endpoint();
        let headers = self.build_headers(&body_str, &url, options.headers.as_ref())?;

        // Body is sent as the exact serialized bytes — the SigV4 signature in
        // `headers` was computed over `body_str`, so `HttpBody::Json` (which
        // would re-serialize) would invalidate it.
        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url,
                headers,
                body: HttpBody::Bytes(body_str.into_bytes(), "application/json".to_string()),
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let data: AnthropicResponse =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

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
        let body = build_request_body(&self.model_id, options, true)?;
        let body_str = serde_json::to_string(&body).unwrap_or_default();
        let url = self.endpoint();
        let headers = self.build_headers(&body_str, &url, options.headers.as_ref())?;

        let resp = send_stream(
            HttpRequest {
                method: HttpMethod::Post,
                url,
                headers,
                body: HttpBody::Bytes(body_str.into_bytes(), "application/json".to_string()),
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
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
                                        yield Ok(StreamPart::TextStart { id });
                                    }
                                    yield Ok(StreamPart::TextDelta {
                                        id: index.to_string(),
                                        delta: text,
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
                                if let Some(state) = blocks.remove(&index) {
                                    match state {
                                        BlockState::Text { started: true } => {
                                            yield Ok(StreamPart::TextEnd {
                                                id: index.to_string(),
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
