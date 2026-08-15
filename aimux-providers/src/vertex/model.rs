//! Google Vertex AI language model — implements `LanguageModel`.
//!
//! Reuses the shared [`crate::google::convert`] message conversion logic and
//! [`crate::google::types`] response types. Only the endpoint construction and
//! authentication differ from the public Gemini API provider.

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

use crate::google::convert::{
    build_request_body, convert_usage, extract_sources, parse_finish_reason,
};
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
    /// 凭证来源(RFC-0023):`None` = explicit;`Some("env:VAR")` = 环境变量。
    pub api_key_source: Option<String>,
    /// 重试配置(M1b)。默认 `RetryConfig::default()`（max_retries=2）。
    pub retry_config: RetryConfig,
}

/// A Google Vertex AI language model.
///
/// Does **not** hold an HTTP client — `http::send` / `http::send_stream` use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct VertexModel {
    model_id: String,
    config: VertexConfig,
}

impl VertexModel {
    #[must_use]
    pub fn new(model_id: String, config: VertexConfig) -> Self {
        Self { model_id, config }
    }

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
            })),
        }
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let body = build_request_body(&self.model_id, options);
        let headers = self.build_headers(options.headers.as_ref());
        let retry_config = crate::openai::model::resolve_retry_config(
            &self.config.retry_config,
            options.max_retries,
        );

        let resp = send_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.generate_endpoint(),
                headers,
                body: HttpBody::Json(body.clone()),

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
                recording_context: options.recording_context.clone(),
            },
            retry_config,
            &GOOGLE_ERROR_STRUCTURE,
            options.timeout.map(Into::into),
        )
        .await?;

        let response_headers = resp.headers;

        let data: GenerateContentResponse =
            serde_json::from_slice(&resp.body).map_err(AiMuxError::from)?;

        let candidate = data.candidates.into_iter().next().ok_or_else(|| {
            AiMuxError::InvalidResponseData("no candidates in response".to_string())
        })?;

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
                // Gemini responses carry no timestamp field; use the response
                // `Date` header (RFC1123) like Bedrock does.
                timestamp: response_headers.get("date").cloned(),
                model_id: data.model_version,
            },
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let body = build_request_body(&self.model_id, options);
        let headers = self.build_headers(options.headers.as_ref());
        let retry_config = crate::openai::model::resolve_retry_config(
            &self.config.retry_config,
            options.max_retries,
        );

        let resp = send_stream_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.stream_endpoint(),
                headers,
                body: HttpBody::Json(body.clone()),

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
                recording_context: options.recording_context.clone(),
            },
            retry_config,
            &GOOGLE_ERROR_STRUCTURE,
            options.timeout.map(Into::into),
        )
        .await?;

        let response_headers = resp.headers;
        // Same source as the non-stream path: the response `Date` header.
        let response_timestamp = response_headers.get("date").cloned();

        let sse_stream = SseStream::new(resp.body);

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

            // Provider-metadata accumulators (mirrors TS `lastGroundingMetadata` /
            // `lastUrlContextMetadata` + the finishReason-chunk snapshot). Same
            // behaviour as the public Gemini API provider.
            let mut last_grounding_metadata: Option<Value> = None;
            let mut last_url_context_metadata: Option<Value> = None;
            let mut last_prompt_feedback: Option<Value> = None;
            let mut last_safety_ratings: Option<Value> = None;
            let mut last_finish_message: Option<Value> = None;
            let mut last_usage_metadata_value: Option<Value> = None;

            // Source dedup across chunks (url sources only).
            let mut emitted_source_urls: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut source_id = 0usize;

            // Associates code-execution results / server tool responses with
            // their preceding call.
            let mut last_code_execution_tool_call_id: Option<String> = None;
            let mut last_server_tool_call_id: Option<String> = None;

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
                                    error: AiMuxError::from(e),
                                });
                                stream_errored = true;
                                break;
                            }
                        };

                        let chunk: StreamChunk = match serde_json::from_value(parsed) {
                            Ok(c) => c,
                            Err(e) => {
                                yield Ok(StreamPart::Error {
                                    error: AiMuxError::from(e),
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
                                    timestamp: response_timestamp.clone(),
                                    model_id: chunk.model_version.clone(),
                                });
                            }

                        if let Some(usage) = &chunk.usage_metadata {
                            final_usage = convert_usage(usage);
                            last_usage_metadata_value = serde_json::to_value(usage).ok();
                        }
                        if let Some(pf) = &chunk.prompt_feedback {
                            last_prompt_feedback = Some(pf.clone());
                        }

                        let Some(candidates) = chunk.candidates else {
                            continue;
                        };
                        let Some(candidate) = candidates.into_iter().next() else {
                            continue;
                        };

                        if let Some(gm) = &candidate.grounding_metadata {
                            last_grounding_metadata = Some(gm.clone());
                        }
                        if let Some(ucm) = &candidate.url_context_metadata {
                            last_url_context_metadata = Some(ucm.clone());
                        }

                        // Extract url sources from this chunk's grounding metadata
                        // (deduplicated across chunks; document sources are not
                        // emitted in the stream, matching TS).
                        let chunk_sources =
                            extract_sources(candidate.grounding_metadata.as_ref(), &mut source_id);
                        for src in chunk_sources {
                            if let GenerateContent::Source {
                                url: Some(url),
                                source_type,
                                id,
                                title,
                                provider_metadata: None,
                            } = src
                                && emitted_source_urls.insert(url.clone()) {
                                    yield Ok(StreamPart::Source {
                                        id,
                                        source_type,
                                        url: Some(url),
                                        title,
                                        provider_metadata: None,
                                    });
                                }
                        }

                        if let Some(parts) =
                            candidate.content.as_ref().and_then(|c| c.parts.as_ref())
                        {
                            for part in parts {
                                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                                    if !text.is_empty() {
                                        if text_id.is_none() {
                                            let id = format!("{block_counter}");
                                            block_counter += 1;
                                            text_id = Some(id.clone());
                                            yield Ok(StreamPart::TextStart { id, provider_metadata: None});
                                        }
                                        if let Some(id) = &text_id {
                                            yield Ok(StreamPart::TextDelta {
                                                id: id.clone(),
                                                delta: text.to_string(),
                                                provider_metadata: None,
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
                                        .map(std::string::ToString::to_string)
                                        .unwrap_or_else(|| format!("call-{block_counter}"));
                                    block_counter += 1;
                                    let args = fc.get("args").cloned().unwrap_or(json!({}));
                                    let thought_signature = part
                                        .get("thoughtSignature")
                                        .and_then(|v| v.as_str())
                                        .map(std::string::ToString::to_string);

                                    yield Ok(StreamPart::ToolInputStart {
                                        id: id.clone(),
                                        tool_name: name.to_string(),
                                        provider_executed: None,
                                        dynamic: None,
                                        title: None,
                                        provider_metadata: None,
                                    });
                                    let args_str = args.to_string();
                                    yield Ok(StreamPart::ToolInputDelta {
                                        id: id.clone(),
                                        delta: args_str,
                                        provider_metadata: None,
                                    });
                                    yield Ok(StreamPart::ToolInputEnd { id: id.clone(), provider_metadata: None});
                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: id,
                                        tool_name: name.to_string(),
                                        input: args,
                                        provider_executed: None,
                                        dynamic: None,
                                        thought_signature,
                                        provider_metadata: None,
                                    });
                                    has_tool_calls = true;
                                } else if let Some(ec) = part.get("executableCode") {
                                    // Provider-executed code execution.
                                    let has_code = ec
                                        .get("code")
                                        .and_then(|v| v.as_str())
                                        .map(|s| !s.is_empty())
                                        .unwrap_or(false);
                                    if has_code {
                                        let id = format!("call-{block_counter}");
                                        block_counter += 1;
                                        last_code_execution_tool_call_id = Some(id.clone());
                                        yield Ok(StreamPart::ToolCall {
                                            tool_call_id: id,
                                            tool_name: "code_execution".to_string(),
                                            input: ec.clone(),
                                            provider_executed: None,
                                            dynamic: None,
                                            thought_signature: None,
                                            provider_metadata: None,
                                        });
                                        // provider-executed → does NOT set has_tool_calls
                                    }
                                } else if let Some(cer) = part.get("codeExecutionResult") {
                                    // Result corresponds to the most recent
                                    // executableCode part.
                                    if let Some(call_id) =
                                        last_code_execution_tool_call_id.take()
                                    {
                                        let outcome =
                                            cer.get("outcome").cloned().unwrap_or(json!(null));
                                        let output = cer
                                            .get("output")
                                            .and_then(|v| v.as_str())
                                            .map(std::string::ToString::to_string)
                                            .unwrap_or_default();
                                        yield Ok(StreamPart::ToolResult {
                                            tool_call_id: call_id,
                                            tool_name: String::new(),
                                            result: json!({ "outcome": outcome, "output": output }),
                                            is_error: None,
                                            preliminary: None,
                                            dynamic: None,
                                            provider_metadata: None,
                                        });
                                    }
                                } else if let Some(tc) = part.get("toolCall") {
                                    // Server-side tool call (provider-executed).
                                    let tool_type = tc
                                        .get("toolType")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let id = tc
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .map(std::string::ToString::to_string)
                                        .unwrap_or_else(|| format!("call-{block_counter}"));
                                    block_counter += 1;
                                    last_server_tool_call_id = Some(id.clone());
                                    let args = tc.get("args").cloned().unwrap_or(json!({}));
                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: id,
                                        tool_name: format!("server:{tool_type}"),
                                        input: args,
                                        provider_executed: None,
                                        dynamic: None,
                                        thought_signature: None,
                                        provider_metadata: None,
                                    });
                                    // provider-executed → does NOT set has_tool_calls
                                } else if let Some(tr) = part.get("toolResponse") {
                                    // Server-side tool response.
                                    let id = last_server_tool_call_id
                                        .take()
                                        .or_else(|| {
                                            tr.get("id")
                                                .and_then(|v| v.as_str())
                                                .map(std::string::ToString::to_string)
                                        })
                                        .unwrap_or_else(|| format!("call-{block_counter}"));
                                    block_counter += 1;
                                    let response =
                                        tr.get("response").cloned().unwrap_or(json!({}));
                                    yield Ok(StreamPart::ToolResult {
                                        tool_call_id: id,
                                        tool_name: String::new(),
                                        result: response,
                                        is_error: None,
                                        preliminary: None,
                                        dynamic: None,
                                        provider_metadata: None,
                                    });
                                }
                            }
                        }

                        if let Some(reason) = candidate.finish_reason.as_deref() {
                            if let Some(id) = text_id.take() {
                                yield Ok(StreamPart::TextEnd { id, provider_metadata: None});
                            }
                            // Snapshot the finishReason-chunk metadata.
                            if let Some(sr) = &candidate.safety_ratings {
                                last_safety_ratings =
                                    Some(serde_json::to_value(sr).unwrap_or(Value::Null));
                            }
                            if let Some(fm) = &candidate.finish_message {
                                last_finish_message = Some(json!(fm));
                            }
                            final_finish_reason =
                                Some(parse_finish_reason(reason, has_tool_calls));
                        }
                    }
                    Err(e) => {
                        yield Ok(StreamPart::Error {
                            error: AiMuxError::InvalidResponseData(e.to_string()),
                        });
                        stream_errored = true;
                        break;
                    }
                }
            }

            if let Some(id) = text_id.take() {
                yield Ok(StreamPart::TextEnd { id, provider_metadata: None});
            }

            let provider_metadata = Some(serde_json::json!({
                "googleVertex": {
                    "promptFeedback": last_prompt_feedback,
                    "groundingMetadata": last_grounding_metadata,
                    "urlContextMetadata": last_url_context_metadata,
                    "safetyRatings": last_safety_ratings,
                    "usageMetadata": last_usage_metadata_value,
                    "finishMessage": last_finish_message,
                }
            }));

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
                    provider_metadata: None,
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
            let thought_signature = part
                .get("thoughtSignature")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string);
            content.push(GenerateContent::ToolCall {
                tool_call_id: id,
                tool_name: name,
                input,
                provider_executed: None,
                dynamic: None,
                thought_signature,
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
