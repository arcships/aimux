//! Google Gemini language model — implements `LanguageModel`.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, ProviderMetadata, ResponseMetadata, Usage,
};

use aimux_provider_utils::response::{ErrorStructure, error_for_status};
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, send_stream_timed, send_timed};
use aimux_stream::SseStream;

use super::GoogleConfig;
use super::convert::{
    build_request_body_with_warnings, convert_usage, extract_sources, parse_finish_reason,
};
use super::types::{
    Candidate, GenerateContentResponse, GoogleErrorEnvelope, GoogleUsageMetadata, StreamChunk,
};

/// Google-specific error structure: `{ "error": { "message": "..." } }`.
///
/// Google uses `code` (HTTP status as a number) and `status` (a string like
/// `INVALID_ARGUMENT`) instead of OpenAI's `type` field.
const GOOGLE_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["error", "message"],
    type_path: &["error", "status"],
};

/// A Google Gemini language model.
///
/// Does **not** hold an HTTP client — `http::send` / `http::send_stream` use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct GoogleModel {
    model_id: String,
    config: GoogleConfig,
}

impl GoogleModel {
    pub fn new(model_id: String, config: GoogleConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        // The TS SDK sends the API key via `x-goog-api-key`. The query-param
        // form (`?key=…`) is also supported but the header form is preferred.
        headers.insert("x-goog-api-key".to_string(), self.config.api_key.clone());
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    /// `…/models/{model}:generateContent` (or `{model}:generateContent` when
    /// the model id already contains a `/`, e.g. a fine-tuned path).
    fn generate_endpoint(&self) -> String {
        let model_path = if self.model_id.contains('/') {
            self.model_id.clone()
        } else {
            format!("models/{}", self.model_id)
        };
        format!("{}/{}:generateContent", self.config.base_url, model_path)
    }

    fn stream_endpoint(&self) -> String {
        let model_path = if self.model_id.contains('/') {
            self.model_id.clone()
        } else {
            format!("models/{}", self.model_id)
        };
        format!(
            "{}/{}:streamGenerateContent?alt=sse",
            self.config.base_url, model_path
        )
    }
}

/// Build the header list for a JSON POST: auth/extra headers + `Content-Type`.
///
/// Returns a `Vec<(String, String)>` for `HttpRequest` — no reqwest types.
fn build_header_list(headers: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    list.push(("Content-Type".to_string(), "application/json".to_string()));
    list
}

#[async_trait]
impl LanguageModel for GoogleModel {
    fn provider(&self) -> &str {
        "google.generative-ai"
    }

    fn model_id(&self) -> &str {
        &self.model_id
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
        let (body, tool_warnings) = build_request_body_with_warnings(&self.model_id, options);
        let headers = self.build_headers(options.headers.as_ref());
        let retry_config = crate::openai::model::resolve_retry_config(
            &self.config.retry_config,
            options.max_retries,
        );

        let resp = send_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.generate_endpoint(),
                headers: build_header_list(&headers),
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

        // Provider metadata: wrap the raw Google metadata under a `google`
        // key (matching the TS `wrapProviderMetadata`).
        let provider_metadata = Some(serde_json::json!({
            "google": {
                "promptFeedback": data.prompt_feedback,
                "groundingMetadata": candidate.grounding_metadata,
                "urlContextMetadata": candidate.url_context_metadata,
                "safetyRatings": candidate.safety_ratings,
                "usageMetadata": data.usage_metadata,
                "finishMessage": candidate.finish_message,
            }
        }));

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: tool_warnings,
            provider_metadata,
            response: ResponseMetadata {
                id: data.response_id,
                timestamp: None,
                model_id: None,
            },
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let (body, tool_warnings) = build_request_body_with_warnings(&self.model_id, options);
        let headers = self.build_headers(options.headers.as_ref());
        let retry_config = crate::openai::model::resolve_retry_config(
            &self.config.retry_config,
            options.max_retries,
        );

        let resp = send_stream_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.stream_endpoint(),
                headers: build_header_list(&headers),
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

        let sse_stream = SseStream::new(resp.body);

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings: tool_warnings });

            let mut sse_stream = sse_stream;
            let mut text_id: Option<String> = None;
            let mut reasoning_id: Option<String> = None;
            let mut block_counter = 0usize;
            let mut final_usage: Usage = Usage::default();
            let mut final_finish_reason: Option<FinishReason> = None;
            let mut has_tool_calls = false;
            let mut response_metadata_emitted = false;
            let mut stream_errored = false;

            // Provider-metadata accumulators (mirrors TS `lastGroundingMetadata` /
            // `lastUrlContextMetadata` + the finishReason-chunk snapshot).
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
                        // Parse as generic Value first to surface errors.
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

                        // Try to parse as a StreamChunk. The Google stream
                        // is a sequence of independent JSON objects (one per
                        // SSE event), not OpenAI-style delta fragments.
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

                        // Emit ResponseMetadata from the first chunk that has
                        // a responseId (matches TS behaviour).
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
                            last_usage_metadata_value =
                                serde_json::to_value(usage).ok();
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
                                // thoughtSignature → provider_metadata (upstream :778-782)
                                let thought_sig_meta: Option<Value> = part
                                    .get("thoughtSignature")
                                    .and_then(|v| v.as_str())
                                    .map(|s| json!({ "google": { "thoughtSignature": s } }));

                                // text part
                                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                                    if text.is_empty() {
                                        // Empty-text part may carry a thoughtSignature
                                        // (upstream :784-794): emit as a zero-length
                                        // delta with the signature metadata on
                                        // whichever block is open — text or
                                        // reasoning (mirrors the non-streaming
                                        // path, which attaches to the last
                                        // content item regardless of type).
                                        if let Some(meta) = &thought_sig_meta {
                                            if let Some(id) = &text_id {
                                                yield Ok(StreamPart::TextDelta {
                                                    id: id.clone(),
                                                    delta: String::new(),
                                                    provider_metadata: Some(meta.clone()),
                                                });
                                            } else if let Some(id) = &reasoning_id {
                                                yield Ok(StreamPart::ReasoningDelta {
                                                    id: id.clone(),
                                                    delta: String::new(),
                                                    provider_metadata: Some(meta.clone()),
                                                });
                                            }
                                        }
                                    } else {
                                        let is_thought = part.get("thought").and_then(serde_json::Value::as_bool).unwrap_or(false);
                                        if is_thought {
                                            // Close any open text block before starting reasoning
                                            if let Some(id) = text_id.take() {
                                                yield Ok(StreamPart::TextEnd { id, provider_metadata: None });
                                            }
                                            // Start a reasoning block if not already active
                                            if reasoning_id.is_none() {
                                                let id = format!("{block_counter}");
                                                block_counter += 1;
                                                reasoning_id = Some(id.clone());
                                                yield Ok(StreamPart::ReasoningStart {
                                                    id,
                                                    provider_metadata: thought_sig_meta.clone(),
                                                });
                                            }
                                            if let Some(id) = &reasoning_id {
                                                yield Ok(StreamPart::ReasoningDelta {
                                                    id: id.clone(),
                                                    delta: text.to_string(),
                                                    provider_metadata: thought_sig_meta.clone(),
                                                });
                                            }
                                        } else {
                                            // Close any open reasoning block before starting text
                                            if let Some(id) = reasoning_id.take() {
                                                yield Ok(StreamPart::ReasoningEnd { id, provider_metadata: None });
                                            }
                                            if text_id.is_none() {
                                                let id = format!("{block_counter}");
                                                block_counter += 1;
                                                text_id = Some(id.clone());
                                                yield Ok(StreamPart::TextStart { id, provider_metadata: thought_sig_meta.clone() });
                                            }
                                            if let Some(id) = &text_id {
                                                yield Ok(StreamPart::TextDelta {
                                                    id: id.clone(),
                                                    delta: text.to_string(),
                                                    provider_metadata: thought_sig_meta.clone(),
                                                });
                                            }
                                        }
                                    }
                                } else if let Some(fc) = part.get("functionCall") {
                                    // Single-chunk complete function call (the
                                    // common case for the public Gemini API).
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
                                        provider_metadata: thought_sig_meta.clone(),
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
                                            tool_name: "code_execution".to_string(),
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
                                    let mut server_meta = json!({
                                        "google": { "serverToolCallId": id, "serverToolType": tool_type }
                                    });
                                    if let Some(s) = part.get("thoughtSignature").and_then(|v| v.as_str()) {
                                        server_meta["google"]["thoughtSignature"] = json!(s);
                                    }
                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: id,
                                        tool_name: format!("server:{tool_type}"),
                                        input: args,
                                        provider_executed: None,
                                        dynamic: None,
                                        thought_signature: None,
                                        provider_metadata: Some(server_meta),
                                    });
                                    // provider-executed → does NOT set has_tool_calls
                                } else if let Some(tr) = part.get("toolResponse") {
                                    // Server-side tool response.
                                    let tool_type = tr.get("toolType").and_then(|v| v.as_str()).unwrap_or("");
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
                                    let mut server_meta = json!({
                                        "google": { "serverToolCallId": id, "serverToolType": tool_type }
                                    });
                                    if let Some(s) = part.get("thoughtSignature").and_then(|v| v.as_str()) {
                                        server_meta["google"]["thoughtSignature"] = json!(s);
                                    }
                                    yield Ok(StreamPart::ToolResult {
                                        tool_call_id: id,
                                        tool_name: format!("server:{tool_type}"),
                                        result: response,
                                        is_error: None,
                                        preliminary: None,
                                        dynamic: None,
                                        provider_metadata: Some(server_meta),
                                    });
                                } else if let Some(inline) = part.get("inlineData") {
                                    // File output — upstream :847-877.
                                    // Close any open text/reasoning block before file.
                                    if let Some(id) = text_id.take() {
                                        yield Ok(StreamPart::TextEnd { id, provider_metadata: None });
                                    }
                                    if let Some(id) = reasoning_id.take() {
                                        yield Ok(StreamPart::ReasoningEnd { id, provider_metadata: None });
                                    }
                                    if let (Some(data), Some(mime)) = (
                                        inline.get("data").and_then(|v| v.as_str()),
                                        inline.get("mimeType").and_then(|v| v.as_str()),
                                    ) {
                                        // part.thought === true → upstream emits 'reasoning-file';
                                        // emit plain File (reasoning-file is a separate PR).
                                        yield Ok(StreamPart::File {
                                            data: FileData::Data { data: FileBytes::Base64(data.to_string()) },
                                            media_type: mime.to_string(),
                                            provider_metadata: thought_sig_meta.clone(),
                                        });
                                    }
                                }
                            }
                        }

                        if let Some(reason) = candidate.finish_reason.as_deref() {
                            // Close any open text/reasoning segment.
                            if let Some(id) = text_id.take() {
                                yield Ok(StreamPart::TextEnd { id, provider_metadata: None});
                            }
                            if let Some(id) = reasoning_id.take() {
                                yield Ok(StreamPart::ReasoningEnd { id, provider_metadata: None});
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

            // Close any remaining open text/reasoning segment.
            if let Some(id) = text_id.take() {
                yield Ok(StreamPart::TextEnd { id, provider_metadata: None});
            }
            if let Some(id) = reasoning_id.take() {
                yield Ok(StreamPart::ReasoningEnd { id, provider_metadata: None});
            }

            let provider_metadata = Some(serde_json::json!({
                "google": {
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
///
/// Returns `(content, has_tool_calls)` so the caller can disambiguate the
/// `STOP` finish reason. `has_tool_calls` is only set for **client-executed**
/// `functionCall` parts — provider-executed `executableCode` and server-side
/// `toolCall` parts do not flip the finish reason to `tool-calls` (mirroring
/// the TS `hasToolCalls: content.some(part => part.type === 'tool-call' && !part.providerExecuted)`).
///
/// Sources extracted from `groundingMetadata.groundingChunks` are appended
/// after the parts, matching the TS `extractSources` + `content.push(source)`.
fn extract_content_from_candidate(candidate: &Candidate) -> (Vec<GenerateContent>, bool) {
    let mut content = Vec::new();
    let mut has_tool_calls = false;
    let mut source_id = 0usize;

    // Associates code-execution results / server tool responses with
    // their preceding call.
    let mut last_code_execution_tool_call_id: Option<String> = None;
    let mut last_server_tool_call_id: Option<String> = None;

    let parts = candidate.content.as_ref().and_then(|c| c.parts.as_ref());

    if let Some(parts) = parts {
        for part in parts {
            // thoughtSignature → provider_metadata (upstream :448-451)
            let thought_sig_meta: Option<Value> = part
                .get("thoughtSignature")
                .and_then(|v| v.as_str())
                .map(|s| json!({ "google": { "thoughtSignature": s } }));

            // Branch order matches upstream (google-language-model.ts:420-534):
            // executableCode → codeExecutionResult → text → functionCall
            // → inlineData → toolCall → toolResponse
            if let Some(ec) = part.get("executableCode") {
                let has_code = ec
                    .get("code")
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                if has_code {
                    let id = format!("call-{}", content.len());
                    last_code_execution_tool_call_id = Some(id.clone());
                    content.push(GenerateContent::ToolCall {
                        tool_call_id: id,
                        tool_name: "code_execution".to_string(),
                        input: ec.clone(),
                        provider_executed: None,
                        dynamic: None,
                        thought_signature: None,
                        provider_metadata: None,
                    });
                }
            } else if let Some(cer) = part.get("codeExecutionResult") {
                if let Some(call_id) = last_code_execution_tool_call_id.take() {
                    let outcome = cer.get("outcome").cloned().unwrap_or(json!(null));
                    let output = cer
                        .get("output")
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string)
                        .unwrap_or_default();
                    content.push(GenerateContent::ToolResult {
                        tool_call_id: call_id,
                        tool_name: "code_execution".to_string(),
                        result: json!({ "outcome": outcome, "output": output }),
                        is_error: None,
                        preliminary: None,
                        dynamic: None,
                        provider_metadata: None,
                    });
                }
            } else if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                if text.is_empty() {
                    // Empty-text part may carry a thoughtSignature to attach
                    // to the preceding content item (upstream :454-458).
                    if let (Some(meta), Some(last)) = (&thought_sig_meta, content.last_mut()) {
                        set_provider_metadata(last, meta.clone());
                    }
                } else {
                    let is_thought = part
                        .get("thought")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    if is_thought {
                        content.push(GenerateContent::Reasoning {
                            text: text.to_string(),
                            provider_metadata: thought_sig_meta.clone(),
                        });
                    } else {
                        content.push(GenerateContent::Text {
                            text: text.to_string(),
                            provider_metadata: thought_sig_meta.clone(),
                        });
                    }
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
                    provider_metadata: thought_sig_meta.clone(),
                });
                has_tool_calls = true;
            } else if let Some(inline) = part.get("inlineData") {
                // File output (e.g. gemini-2.5-flash-image) — upstream :478-490.
                if let (Some(data), Some(mime)) = (
                    inline.get("data").and_then(|v| v.as_str()),
                    inline.get("mimeType").and_then(|v| v.as_str()),
                ) {
                    // part.thought === true → upstream emits 'reasoning-file';
                    // emit plain File (reasoning-file is a separate PR).
                    content.push(GenerateContent::File {
                        data: FileData::Data {
                            data: FileBytes::Base64(data.to_string()),
                        },
                        media_type: mime.to_string(),
                        provider_metadata: thought_sig_meta.clone(),
                    });
                }
            } else if let Some(tc) = part.get("toolCall") {
                // Server-side tool call (provider-executed, Gemini 3).
                let tool_type = tc.get("toolType").and_then(|v| v.as_str()).unwrap_or("");
                let id = tc
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                last_server_tool_call_id = Some(id.clone());
                let input = tc.get("args").cloned().unwrap_or(json!({}));
                let thought_signature = part
                    .get("thoughtSignature")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string);
                let mut server_meta = json!({
                    "google": { "serverToolCallId": id, "serverToolType": tool_type }
                });
                if let Some(s) = part.get("thoughtSignature").and_then(|v| v.as_str()) {
                    server_meta["google"]["thoughtSignature"] = json!(s);
                }
                content.push(GenerateContent::ToolCall {
                    tool_call_id: id,
                    tool_name: format!("server:{tool_type}"),
                    input,
                    provider_executed: None,
                    dynamic: None,
                    thought_signature,
                    provider_metadata: Some(server_meta),
                });
                // provider-executed → does NOT set has_tool_calls
            } else if let Some(tr) = part.get("toolResponse") {
                // Server-side tool response (upstream :512-533).
                let tool_type = tr.get("toolType").and_then(|v| v.as_str()).unwrap_or("");
                let id = last_server_tool_call_id
                    .take()
                    .or_else(|| {
                        tr.get("id")
                            .and_then(|v| v.as_str())
                            .map(std::string::ToString::to_string)
                    })
                    .unwrap_or_default();
                let response = tr.get("response").cloned().unwrap_or(json!({}));
                let mut server_meta = json!({
                    "google": { "serverToolCallId": id, "serverToolType": tool_type }
                });
                if let Some(s) = part.get("thoughtSignature").and_then(|v| v.as_str()) {
                    server_meta["google"]["thoughtSignature"] = json!(s);
                }
                content.push(GenerateContent::ToolResult {
                    tool_call_id: id,
                    tool_name: format!("server:{tool_type}"),
                    result: response,
                    is_error: None,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: Some(server_meta),
                });
                last_server_tool_call_id = None;
            }
        }
    }

    // Sources are appended after the parts (mirrors TS).
    let mut sources = extract_sources(candidate.grounding_metadata.as_ref(), &mut source_id);
    content.append(&mut sources);

    (content, has_tool_calls)
}

/// Set `provider_metadata` on any `GenerateContent` variant (used to attach
/// a thoughtSignature from an empty-text part to the preceding item).
fn set_provider_metadata(item: &mut GenerateContent, meta: ProviderMetadata) {
    match item {
        GenerateContent::Text {
            provider_metadata, ..
        }
        | GenerateContent::Reasoning {
            provider_metadata, ..
        }
        | GenerateContent::ToolCall {
            provider_metadata, ..
        }
        | GenerateContent::File {
            provider_metadata, ..
        }
        | GenerateContent::Source {
            provider_metadata, ..
        }
        | GenerateContent::ToolResult {
            provider_metadata, ..
        } => {
            *provider_metadata = Some(meta);
        }
    }
}

/// Parse a Google error JSON envelope into an `AiMuxError`. Used for
/// mid-stream error objects (the Gemini SSE stream can carry `{ "error": … }`
/// events). Kept for parity with the OpenAI provider even though the common
/// error path goes through `parse_provider_error`.
#[allow(dead_code)]
fn parse_google_error_body(body: &str) -> AiMuxError {
    if let Ok(env) = serde_json::from_str::<GoogleErrorEnvelope>(body) {
        let msg = env.error.message;
        let provider_code = env.error.status;
        return match env.error.code {
            Some(code) => error_for_status(
                code as u16,
                provider_code,
                msg,
                None,
                Some(body.to_string()),
            ),
            None => AiMuxError::ApiCall(ApiCallError {
                provider_code,
                message: msg,
                response_body: Some(body.to_string()),
                ..Default::default()
            }),
        };
    }
    AiMuxError::ApiCall(ApiCallError {
        message: body.to_string(),
        response_body: Some(body.to_string()),
        ..Default::default()
    })
}

// Suppress unused warning for GoogleUsageMetadata (re-exported via convert).
#[allow(unused_imports)]
use GoogleUsageMetadata as _GoogleUsageMetadata;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::GoogleConfig;

    #[test]
    fn config_snapshot_records_provider_identity() {
        let config = GoogleConfig::new("sk-test");
        let model = GoogleModel::new("gemini-2.0-flash".to_string(), config);

        let snap = model.config_snapshot();
        assert_eq!(snap.provider, "google.generative-ai");
        assert_eq!(snap.model_id, "gemini-2.0-flash");
        assert_eq!(
            snap.base_url.as_deref(),
            Some("https://generativelanguage.googleapis.com/v1beta")
        );
        assert_eq!(snap.api_key_source, "explicit");
        assert_eq!(snap.profile, None);
        assert_eq!(snap.provider_options, None);
    }
}
