//! Google Vertex AI language model — implements `LanguageModel`.
//!
//! Reuses the shared [`crate::google::convert`] message conversion logic and
//! [`crate::google::types`] response types. Only the endpoint construction and
//! authentication differ from the public Gemini API provider.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

use aimux_provider_utils::response::{ErrorStructure, parse_provider_error};
use aimux_stream::SseStream;

use crate::google::convert::{build_request_body, convert_usage, parse_finish_reason};
use crate::google::types::{Candidate, GenerateContentResponse, GoogleUsageMetadata, StreamChunk};

use super::VertexAuth;

/// Google-specific error structure: `{ "error": { "message": "..." } }`.
const GOOGLE_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["error", "message"],
    type_path: &["error", "status"],
};

/// Configuration for a Vertex model instance (cloned from the provider).
#[derive(Debug, Clone)]
pub struct VertexConfig {
    pub base_url: String,
    pub auth: VertexAuth,
}

/// A Google Vertex AI language model.
pub struct VertexModel {
    model_id: String,
    config: VertexConfig,
    client: Client,
}

impl VertexModel {
    pub fn new(model_id: String, config: VertexConfig, client: Client) -> Self {
        Self {
            model_id,
            config,
            client,
        }
    }

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

    /// The effective base URL for this model. Tuned models addressed via
    /// `endpoints/{id}` are served from `…/locations/{region}/endpoints/{id}`
    /// (no `/publishers/google` suffix), so the suffix is stripped from the
    /// configured base URL. Mirrors the TS `loadBaseURL({ endpoint: true })`.
    fn effective_base_url(&self) -> &str {
        if self.model_id.starts_with("endpoints/") {
            // Strip the trailing `/publishers/google` (if present).
            if let Some(stripped) = self.config.base_url.strip_suffix("/publishers/google") {
                return stripped;
            }
        }
        &self.config.base_url
    }

    /// `…/models/{model}:generateContent`
    fn generate_endpoint(&self) -> String {
        let model_path = if self.model_id.contains('/') {
            self.model_id.clone()
        } else {
            format!("models/{}", self.model_id)
        };
        format!(
            "{}/{}:generateContent",
            self.effective_base_url(),
            model_path
        )
    }

    /// `…/models/{model}:streamGenerateContent?alt=sse`
    fn stream_endpoint(&self) -> String {
        let model_path = if self.model_id.contains('/') {
            self.model_id.clone()
        } else {
            format!("models/{}", self.model_id)
        };
        format!(
            "{}/{}:streamGenerateContent?alt=sse",
            self.effective_base_url(),
            model_path
        )
    }
}

#[async_trait]
impl LanguageModel for VertexModel {
    fn provider(&self) -> &str {
        "google.vertex"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let body = build_request_body(&self.model_id, options);
        let headers = self.build_headers(options.headers.as_ref());

        let mut req = self.client.post(self.generate_endpoint());
        for (k, v) in &headers {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::try_from(k),
                reqwest::header::HeaderValue::try_from(v),
            ) {
                req = req.header(name, val);
            }
        }

        let resp = req
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
                &GOOGLE_ERROR_STRUCTURE,
            ));
        }

        let response_headers: HashMap<String, String> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        let data: GenerateContentResponse = resp
            .json()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        let candidate = data
            .candidates
            .into_iter()
            .next()
            .ok_or_else(|| AiMuxError::Provider("no candidates in response".to_string()))?;

        let (content, has_tool_calls) = extract_content_from_candidate(&candidate);

        let finish_reason = candidate
            .finish_reason
            .as_deref()
            .map(|r| parse_finish_reason(r, has_tool_calls))
            .unwrap_or(FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            });

        let usage = data
            .usage_metadata
            .as_ref()
            .map(convert_usage)
            .unwrap_or_default();

        let provider_metadata = Some(serde_json::json!({
            "googleVertex": {
                "promptFeedback": data.prompt_feedback,
                "usageMetadata": data.usage_metadata,
                "finishMessage": candidate.finish_message,
            }
        }));

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: Vec::new(),
            provider_metadata,
            response: ResponseMetadata {
                id: data.response_id,
                timestamp: None,
                model_id: data.model_version,
            },
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let body = build_request_body(&self.model_id, options);
        let headers = self.build_headers(options.headers.as_ref());

        let mut req = self.client.post(self.stream_endpoint());
        for (k, v) in &headers {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::try_from(k),
                reqwest::header::HeaderValue::try_from(v),
            ) {
                req = req.header(name, val);
            }
        }

        let resp = req
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
                &GOOGLE_ERROR_STRUCTURE,
            ));
        }

        let response_headers = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect::<HashMap<_, _>>();

        let byte_stream = resp.bytes_stream();
        let sse_stream = SseStream::new(byte_stream);

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings: vec![] });

            let mut sse_stream = sse_stream;
            let mut text_id: Option<String> = None;
            let mut block_counter = 0usize;
            let mut final_usage: Usage = Usage::default();
            let mut final_finish_reason: Option<FinishReason> = None;
            let mut has_tool_calls = false;
            let mut response_metadata_emitted = false;
            let mut stream_errored = false;

            while let Some(event) = sse_stream.next().await {
                if stream_errored {
                    break;
                }
                match event {
                    Ok(sse_event) => {
                        let parsed: Value = match serde_json::from_str(&sse_event.data) {
                            Ok(v) => v,
                            Err(e) => {
                                yield Ok(StreamPart::Error {
                                    error: AiMuxError::Json(e.to_string()),
                                });
                                stream_errored = true;
                                break;
                            }
                        };

                        let chunk: StreamChunk = match serde_json::from_value(parsed) {
                            Ok(c) => c,
                            Err(e) => {
                                yield Ok(StreamPart::Error {
                                    error: AiMuxError::Json(e.to_string()),
                                });
                                stream_errored = true;
                                break;
                            }
                        };

                        if !response_metadata_emitted
                            && let Some(id) = &chunk.response_id {
                                response_metadata_emitted = true;
                                yield Ok(StreamPart::ResponseMetadata {
                                    id: Some(id.clone()),
                                    timestamp: None,
                                    model_id: chunk.model_version.clone(),
                                });
                            }

                        if let Some(usage) = &chunk.usage_metadata {
                            final_usage = convert_usage(usage);
                        }

                        let Some(candidates) = chunk.candidates else {
                            continue;
                        };
                        let Some(candidate) = candidates.into_iter().next() else {
                            continue;
                        };

                        if let Some(parts) =
                            candidate.content.as_ref().and_then(|c| c.parts.as_ref())
                        {
                            for part in parts {
                                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                                    if !text.is_empty() {
                                        if text_id.is_none() {
                                            let id = format!("{}", block_counter);
                                            block_counter += 1;
                                            text_id = Some(id.clone());
                                            yield Ok(StreamPart::TextStart { id });
                                        }
                                        if let Some(id) = &text_id {
                                            yield Ok(StreamPart::TextDelta {
                                                id: id.clone(),
                                                delta: text.to_string(),
                                            });
                                        }
                                    }
                                } else if let Some(fc) = part.get("functionCall") {
                                    let name = fc
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let id = fc
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string())
                                        .unwrap_or_else(|| format!("call-{}", block_counter));
                                    block_counter += 1;
                                    let args = fc.get("args").cloned().unwrap_or(json!({}));

                                    yield Ok(StreamPart::ToolInputStart {
                                        id: id.clone(),
                                        tool_name: name.to_string(),
                                        provider_executed: None,
                                        dynamic: None,
                                    });
                                    let args_str = args.to_string();
                                    yield Ok(StreamPart::ToolInputDelta {
                                        id: id.clone(),
                                        delta: args_str,
                                    });
                                    yield Ok(StreamPart::ToolInputEnd { id: id.clone() });
                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: id,
                                        tool_name: name.to_string(),
                                        input: args,
                                        provider_executed: None,
                                        dynamic: None,
                                    });
                                    has_tool_calls = true;
                                }
                            }
                        }

                        if let Some(reason) = candidate.finish_reason.as_deref() {
                            if let Some(id) = text_id.take() {
                                yield Ok(StreamPart::TextEnd { id });
                            }
                            final_finish_reason =
                                Some(parse_finish_reason(reason, has_tool_calls));
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

            if let Some(id) = text_id.take() {
                yield Ok(StreamPart::TextEnd { id });
            }

            let provider_metadata = Some(serde_json::json!({ "googleVertex": {} }));

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
                usage: if stream_errored { Usage::default() } else { final_usage },
                provider_metadata,
            });
        };

        Ok(StreamResult {
            stream: Box::pin(stream),
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Extract `GenerateContent` items from a non-streaming candidate.
fn extract_content_from_candidate(candidate: &Candidate) -> (Vec<GenerateContent>, bool) {
    let mut content = Vec::new();
    let mut has_tool_calls = false;

    let Some(parts) = candidate.content.as_ref().and_then(|c| c.parts.as_ref()) else {
        return (content, has_tool_calls);
    };

    for part in parts {
        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
            if !text.is_empty() {
                content.push(GenerateContent::Text {
                    text: text.to_string(),
                });
            }
        } else if let Some(fc) = part.get("functionCall") {
            let name = fc
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let id = fc
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let input = fc.get("args").cloned().unwrap_or(json!({}));
            content.push(GenerateContent::ToolCall {
                tool_call_id: id,
                tool_name: name,
                input,
                provider_executed: None,
                dynamic: None,
                provider_metadata: None,
            });
            has_tool_calls = true;
        }
    }

    (content, has_tool_calls)
}

// Suppress unused warnings for types re-exported via convert.
#[allow(unused_imports)]
use GoogleUsageMetadata as _GoogleUsageMetadata;
