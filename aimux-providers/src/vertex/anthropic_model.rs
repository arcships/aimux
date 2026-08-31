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
//! Reference: <https://docs.cloud.google.com/claude-on-vertex-ai>

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

use aimux_provider_utils::{HttpRequest, RetryConfig};

use crate::anthropic::convert::{build_request_body_with_warnings, parse_stop_reason};
use crate::anthropic::stream::stream_parts_for_result_block;
use crate::anthropic::tool_name_mapping::ToolNameMapping;
use crate::anthropic::types::{AnthropicResponse, ContentBlock, StreamEvent};

use super::VertexAuth;

/// `anthropic_version` envelope value required by the Vertex AI `rawPredict` /
/// `streamRawPredict` endpoints.
const ANTHROPIC_VERTEX_VERSION: &str = "vertex-2023-10-16";

/// Google/Vertex error structure: `{ "error": { "message": "...", "status": "..." } }`.
///
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
    /// 凭证来源(RFC-0023):`None` = explicit;`Some("env:VAR")` = 环境变量。
    pub api_key_source: Option<String>,
    /// Retry settings used by Core model operations.
    pub retry_config: RetryConfig,
}

/// An Anthropic Claude language model served via Vertex AI `rawPredict`.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct VertexAnthropicModel {
    model_id: String,
    config: VertexAnthropicConfig,
}

impl VertexAnthropicModel {
    #[must_use]
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
                headers.push(("Authorization".to_string(), format!("Bearer {token}")));
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

    fn retry_config(&self) -> aimux_core::retry::RetryConfig {
        self.config.retry_config
    }

    fn config_snapshot(&self) -> aimux_core::recording::ProviderRecord {
        use aimux_core::recording::ProviderRecord;
        // M2b: record identity + credential source + auth kind. Never serialize
        // the bearer token or API key plaintext.
        let auth_kind = match &self.config.auth {
            VertexAuth::BearerToken(_) => "bearer_token",
            VertexAuth::ApiKey(_) => "api_key",
        };
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
            provider_options: Some(serde_json::json!({
                "auth_kind": auth_kind,
                "anthropic_version": ANTHROPIC_VERTEX_VERSION,
            })),
        }
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let req = build_request_body_with_warnings(&self.model_id, options, false)?;
        let body = self.wrap_raw_predict_body(req.body);
        let url = self.generate_endpoint();
        let headers = self.build_headers(options.headers.as_ref());
        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: url.clone(),
                headers,

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
                recording_context: options.recording_context.clone(),
                ..Default::default()
            },
            body.clone(),
            aimux_provider_utils::create_json_response_handler(),
            crate::google::google_failed_response_handler(),
        )
        .await?;

        let data: AnthropicResponse = resp.value;

        let content = crate::anthropic::stream::parse_anthropic_content(
            &data.content,
            &ToolNameMapping::new(options.tools.as_deref()),
        );

        let finish_reason = data
            .stop_reason
            .as_deref()
            .map(parse_stop_reason)
            .unwrap_or(FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            });

        // RFC-0015 P0-2: fill cache fields + raw (same shape as Anthropic).
        let usage = crate::anthropic::usage::usage_from_anthropic(&data.usage);

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: req.warnings,
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
        let body = self.wrap_raw_predict_body(req.body);
        let warnings = req.warnings;
        let tool_names = ToolNameMapping::new(options.tools.as_deref());
        let url = self.stream_endpoint();
        let headers = self.build_headers(options.headers.as_ref());
        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: url.clone(),
                headers,

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
                recording_context: options.recording_context.clone(),
                ..Default::default()
            },
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<StreamEvent>(),
            crate::google::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let mut sse_stream = resp.value;
        let first_event = match sse_stream.next().await {
            Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
            first_event => first_event,
        };
        if let Some(Ok(StreamEvent::Error { error })) = first_event.as_ref() {
            return Err(crate::anthropic::stream::anthropic_stream_error(
                error,
                &url,
                body.clone(),
                response_headers,
            ));
        }
        let stream_error_url = url;
        let stream_request_body = body.clone();
        let stream_response_headers = response_headers.clone();

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings });

            let mut sse = futures::stream::iter(first_event.into_iter()).chain(sse_stream);
            let mut blocks: HashMap<usize, BlockState> = HashMap::new();
            let mut final_usage = Usage::default();
            let mut final_finish_reason: Option<FinishReason> = None;
            let mut response_meta_emitted = false;
            let mut mcp_tool_calls: HashMap<String, (String, String)> = HashMap::new();

            while let Some(event) = sse.next().await {
                match event {
                    Ok(stream_event) => {
                        match stream_event {
                            StreamEvent::MessageStart { message } => {
                                if let Some(usage) = &message.usage {
                                    // RFC-0015 P0-2: full input side incl.
                                    // cache fields + raw (same as Anthropic).
                                    final_usage =
                                        crate::anthropic::usage::usage_from_anthropic(usage);
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
                            StreamEvent::ContentBlockStart { index, content_block } => {
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
                                    ContentBlock::ServerToolUse { id, name, input } => {
                                        yield Ok(StreamPart::ToolCall {
                                            tool_call_id: id.clone(),
                                            tool_name: tool_names
                                                .to_custom_tool_name(&name)
                                                .to_string(),
                                            input: input.clone(),
                                            provider_executed: Some(true),
                                            dynamic: None,
                                            thought_signature: None,
                                            provider_metadata: None,
                                        });
                                    }
                                    ContentBlock::McpToolUse { id, name, input, server_name } => {
                                        mcp_tool_calls
                                            .insert(id.clone(), (name.clone(), server_name.clone()));
                                        yield Ok(StreamPart::ToolCall {
                                            tool_call_id: id.clone(),
                                            tool_name: name.clone(),
                                            input: input.clone(),
                                            provider_executed: Some(true),
                                            dynamic: Some(true),
                                            thought_signature: None,
                                            provider_metadata: Some(json!({
                                                "anthropic": {
                                                    "type": "mcp-tool-use",
                                                    "serverName": server_name,
                                                }
                                            })),
                                        });
                                    }
                                    ContentBlock::RedactedThinking { data } => {
                                        yield Ok(StreamPart::ReasoningStart {
                                            id: index.to_string(),
                                            provider_metadata: Some(json!({
                                                "anthropic": { "redactedData": data }
                                            })),
                                        });
                                        blocks.insert(index, BlockState::Thinking { started: true });
                                    }
                                    other => {
                                        for part in stream_parts_for_result_block(
                                            &other,
                                            &tool_names,
                                            &mcp_tool_calls,
                                        ) {
                                            yield Ok(part);
                                        }
                                    }
                                }
                            }
                            StreamEvent::ContentBlockDelta { index, delta } => {
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
                            StreamEvent::ContentBlockStop { index } => {
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
                                                thought_signature: None,
                                                provider_metadata: None,
                                            });
                                        }
                                    }
                                }
                            }
                            StreamEvent::MessageDelta { delta, usage } => {
                                if let Some(reason) = delta.stop_reason {
                                    final_finish_reason = Some(parse_stop_reason(&reason));
                                }
                                if let Some(u) = usage {
                                    // Reuse the shared conversion so the output
                                    // reasoning/text split (from
                                    // output_tokens_details) is preserved, matching
                                    // anthropic/stream.rs. Only the output side is
                                    // applied — MessageDelta carries output tokens
                                    // only, so input-side usage from MessageStart is
                                    // kept intact.
                                    final_usage.output_tokens =
                                        crate::anthropic::usage::usage_from_anthropic(&u)
                                            .output_tokens;
                                }
                            }
                            StreamEvent::MessageStop => break,
                            StreamEvent::Error { error } => {
                                yield Ok(StreamPart::Error {
                                    error: crate::anthropic::stream::anthropic_stream_error(
                                        &error,
                                        &stream_error_url,
                                        stream_request_body.clone(),
                                        stream_response_headers.clone(),
                                    ),
                                });
                                return;
                            }
                            _ => {}
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
