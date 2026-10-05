//! Google Gemini language model — implements `LanguageModel`.

use aimux_core::tool::RawToolCall;
use aimux_core::tool::ToolResult;
use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{
    GenerateContent, GenerateResult, GeneratedFile, ReasoningOutput, Source, StreamResult,
};
use aimux_core::shared::{FileBytes, GeneratedFileData, provider_namespace};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, ProviderMetadata, ResponseMetadata, Usage,
};

use aimux_provider_utils::HttpRequest;

use super::GoogleConfig;
use super::convert::{
    build_request_body_with_warnings, code_execution_tool_name, convert_usage, extract_sources,
    parse_finish_reason,
};
use super::types::{Candidate, GenerateContentResponse, GoogleStreamEvent};

/// A Google Gemini language model.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the
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

    fn retry_config(&self) -> aimux_core::retry::RetryConfig {
        self.config.retry_config
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
        if options
            .prompt
            .iter()
            .filter_map(|message| match message {
                aimux_core::language_model_message::LanguageModelMessage::Assistant {
                    content,
                    ..
                } => Some(content),
                _ => None,
            })
            .flatten()
            .any(|part| {
                matches!(
                    part,
                    aimux_core::language_model_message::AssistantPart::ReasoningFile(
                        aimux_core::language_model_message::ReasoningFilePart {
                            data: aimux_core::shared::GeneratedFileData::Url { .. },
                            ..
                        }
                    ) | aimux_core::language_model_message::AssistantPart::File(
                        aimux_core::language_model_message::FilePart {
                            data: aimux_core::shared::FileData::Url { .. },
                            ..
                        }
                    )
                )
            })
        {
            return Err(AiMuxError::UnsupportedFunctionality(
                "File data URLs in assistant messages are not supported".to_string(),
            ));
        }
        crate::google::convert::validate_tool_result_files(&options.prompt)?;
        let code_execution_tool_name = code_execution_tool_name(options.tools.as_deref());
        let (body, tool_warnings) = build_request_body_with_warnings(&self.model_id, options);
        let headers = self.build_headers(options.headers.as_ref());
        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest::new(
                self.generate_endpoint(),
                build_header_list(&headers),
                options,
            ),
            body.clone(),
            aimux_provider_utils::create_json_response_handler(),
            super::google_failed_response_handler(),
        )
        .await?;

        let response_body = resp.raw_value;
        let response_headers = resp.response_headers;

        let data: GenerateContentResponse = resp.value;

        let candidate = data.candidates.into_iter().next().ok_or_else(|| {
            AiMuxError::InvalidResponseData("no candidates in response".to_string())
        })?;

        let (content, has_tool_calls) =
            extract_content_from_candidate(&candidate, &code_execution_tool_name);

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
        let provider_metadata = Some(
            provider_namespace(
                "google",
                json!({
                    "promptFeedback": data.prompt_feedback,
                    "groundingMetadata": candidate.grounding_metadata,
                    "urlContextMetadata": candidate.url_context_metadata,
                    "safetyRatings": candidate.safety_ratings,
                    "usageMetadata": data.usage_metadata,
                    "finishMessage": candidate.finish_message,
                }),
            )
            .expect("provider metadata must be an object"),
        );

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: tool_warnings,
            provider_metadata,
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
            response: Some(aimux_core::result::GenerateResponse {
                body: response_body,
                headers: Some(response_headers),
                ..aimux_core::result::GenerateResponse::from(ResponseMetadata {
                    id: data.response_id,
                    timestamp: None,
                    model_id: None,
                })
            }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        if options
            .prompt
            .iter()
            .filter_map(|message| match message {
                aimux_core::language_model_message::LanguageModelMessage::Assistant {
                    content,
                    ..
                } => Some(content),
                _ => None,
            })
            .flatten()
            .any(|part| {
                matches!(
                    part,
                    aimux_core::language_model_message::AssistantPart::ReasoningFile(
                        aimux_core::language_model_message::ReasoningFilePart {
                            data: aimux_core::shared::GeneratedFileData::Url { .. },
                            ..
                        }
                    ) | aimux_core::language_model_message::AssistantPart::File(
                        aimux_core::language_model_message::FilePart {
                            data: aimux_core::shared::FileData::Url { .. },
                            ..
                        }
                    )
                )
            })
        {
            return Err(AiMuxError::UnsupportedFunctionality(
                "File data URLs in assistant messages are not supported".to_string(),
            ));
        }
        crate::google::convert::validate_tool_result_files(&options.prompt)?;
        let code_execution_tool_name = code_execution_tool_name(options.tools.as_deref());
        let (body, tool_warnings) = build_request_body_with_warnings(&self.model_id, options);
        let headers = self.build_headers(options.headers.as_ref());
        let endpoint = self.stream_endpoint();
        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest::new(endpoint.clone(), build_header_list(&headers), options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<GoogleStreamEvent>(),
            super::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let mut sse_stream = resp.value;
        let first_event = match sse_stream.next().await {
            Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
            first_event => first_event,
        };
        if let Some(Ok(GoogleStreamEvent::Error(error))) = first_event.as_ref() {
            return Err(super::google_stream_error(
                &error.error,
                &endpoint,
                body.clone(),
                response_headers,
            ));
        }
        let stream_error_url = endpoint.clone();
        let stream_request_body = body.clone();
        let stream_error_headers = response_headers.clone();

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings: tool_warnings });

            let mut sse_stream = futures::stream::iter(first_event.into_iter()).chain(sse_stream);
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
                    Ok(GoogleStreamEvent::Chunk(chunk)) => {

                        // Emit ResponseMetadata from the first chunk that has
                        // a responseId (matches TS behaviour).
                        if !response_metadata_emitted
                            && let Some(id) = &chunk.response_id {
                                response_metadata_emitted = true;
                                yield Ok(StreamPart::ResponseMetadata(ResponseMetadata {
                                    id: Some(id.clone()),
                                    timestamp: None,
                                    model_id: chunk.model_version.clone(),
                                }));
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
                            if let GenerateContent::Source(Source {
                                url: Some(url),
                                source_type,
                                id,
                                title,
                                provider_metadata: None,
                            }) = src
                                && emitted_source_urls.insert(url.clone()) {
                                    yield Ok(StreamPart::Source(Source {
                                        id,
                                        source_type,
                                        url: Some(url),
                                        title,
                                        provider_metadata: None,
                                    }));
                                }
                        }

                        if let Some(parts) =
                            candidate.content.as_ref().and_then(|c| c.parts.as_ref())
                        {
                            for part in parts {
                                // thoughtSignature → provider_metadata (upstream :778-782)
                                let thought_sig_meta: Option<ProviderMetadata> = part
                                    .get("thoughtSignature")
                                    .and_then(|v| v.as_str())
                                    .map(|s| provider_namespace("google", json!({ "thoughtSignature": s })).expect("provider metadata must be an object"));

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
                                    yield Ok(StreamPart::ToolCall(RawToolCall {
                                        tool_call_id: id,
                                        tool_name: name.to_string(),
                                        input: args.to_string(),
                                        provider_executed: None,
                                        dynamic: None,
                                        thought_signature,
                                        provider_metadata: thought_sig_meta.clone(),
                                    }));
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
                                        yield Ok(StreamPart::ToolCall(RawToolCall {
                                            tool_call_id: id.clone(),
                                            tool_name: code_execution_tool_name.clone(),
                                            input: ec.to_string(),
                                            provider_executed: Some(true),
                                            dynamic: None,
                                            thought_signature: None,
                                            provider_metadata: Some(server_tool_metadata(
                                                &id,
                                                "code_execution",
                                                None,
                                            )),
                                        }));
                                        // provider-executed → does NOT set has_tool_calls
                                    }
                                } else if let Some(cer) = part.get("codeExecutionResult") {
                                    // Result corresponds to the most recent
                                    // executableCode part. Gemini may emit
                                    // several results for that one call, so
                                    // retain the association until a new call.
                                    if let Some(call_id) =
                                        last_code_execution_tool_call_id.as_ref()
                                    {
                                        let outcome =
                                            cer.get("outcome").cloned().unwrap_or(json!(null));
                                        let output = cer
                                            .get("output")
                                            .and_then(|v| v.as_str())
                                            .map(std::string::ToString::to_string)
                                            .unwrap_or_default();
                                        yield Ok(StreamPart::ToolResult(ToolResult {
                                            tool_call_id: call_id.clone(),
                                            tool_name: code_execution_tool_name.clone(),
                                            result: json!({ "outcome": outcome, "output": output }),
                                            is_error: None,
                                            preliminary: None,
                                            dynamic: None,
                                            provider_metadata: Some(server_tool_metadata(
                                                call_id,
                                                "code_execution",
                                                None,
                                            )),
                                        }));
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
                                    let server_meta = server_tool_metadata(
                                        &id,
                                        tool_type,
                                        part.get("thoughtSignature").and_then(|v| v.as_str()),
                                    );
                                    yield Ok(StreamPart::ToolCall(RawToolCall {
                                        tool_call_id: id,
                                        tool_name: format!("server:{tool_type}"),
                                        input: args.to_string(),
                                        provider_executed: Some(true),
                                        dynamic: Some(true),
                                        thought_signature: None,
                                        provider_metadata: Some(server_meta),
                                    }));
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
                                    let server_meta = server_tool_metadata(
                                        &id,
                                        tool_type,
                                        part.get("thoughtSignature").and_then(|v| v.as_str()),
                                    );
                                    yield Ok(StreamPart::ToolResult(ToolResult {
                                        tool_call_id: id,
                                        tool_name: format!("server:{tool_type}"),
                                        result: response,
                                        is_error: None,
                                        preliminary: None,
                                        dynamic: None,
                                        provider_metadata: Some(server_meta),
                                    }));
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
                                        let file = GeneratedFile {
                                            data: GeneratedFileData::Data { data: FileBytes::Base64(data.to_string()) },
                                            media_type: mime.to_string(),
                                            provider_metadata: thought_sig_meta.clone(),
                                        };
                                        yield Ok(if part.get("thought").and_then(serde_json::Value::as_bool).unwrap_or(false) { StreamPart::ReasoningFile(file) } else { StreamPart::File(file) });
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
                    Ok(GoogleStreamEvent::Error(error)) => {
                        yield Ok(StreamPart::Error {
                            error: super::google_stream_error(
                                &error.error,
                                &stream_error_url,
                                stream_request_body.clone(),
                                stream_error_headers.clone(),
                            ),
                        });
                        stream_errored = true;
                        break;
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

            // Close any remaining open text/reasoning segment.
            if let Some(id) = text_id.take() {
                yield Ok(StreamPart::TextEnd { id, provider_metadata: None});
            }
            if let Some(id) = reasoning_id.take() {
                yield Ok(StreamPart::ReasoningEnd { id, provider_metadata: None});
            }

            let provider_metadata = Some(provider_namespace("google", json!({
                "promptFeedback": last_prompt_feedback,
                "groundingMetadata": last_grounding_metadata,
                "urlContextMetadata": last_url_context_metadata,
                "safetyRatings": last_safety_ratings,
                "usageMetadata": last_usage_metadata_value,
                "finishMessage": last_finish_message,
            })).expect("provider metadata must be an object"));

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
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
            response: Some(aimux_core::result::StreamResponse {
                headers: Some(response_headers),
            }),
        })
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn server_tool_metadata(
    tool_call_id: &str,
    server_tool_type: &str,
    thought_signature: Option<&str>,
) -> ProviderMetadata {
    let mut payload = json!({
        "serverToolCallId": tool_call_id,
        "serverToolType": server_tool_type,
    });
    if let Some(signature) = thought_signature {
        payload["thoughtSignature"] = json!(signature);
    }
    provider_namespace("google", payload).expect("provider metadata must be an object")
}

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
fn extract_content_from_candidate(
    candidate: &Candidate,
    code_execution_tool_name: &str,
) -> (Vec<GenerateContent>, bool) {
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
            let thought_sig_meta: Option<ProviderMetadata> = part
                .get("thoughtSignature")
                .and_then(|v| v.as_str())
                .map(|s| {
                    provider_namespace("google", json!({ "thoughtSignature": s }))
                        .expect("provider metadata must be an object")
                });

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
                    content.push(GenerateContent::ToolCall(RawToolCall {
                        tool_call_id: id.clone(),
                        tool_name: code_execution_tool_name.to_string(),
                        input: ec.to_string(),
                        provider_executed: Some(true),
                        dynamic: None,
                        thought_signature: None,
                        provider_metadata: Some(server_tool_metadata(&id, "code_execution", None)),
                    }));
                }
            } else if let Some(cer) = part.get("codeExecutionResult") {
                // One executableCode may be followed by multiple results.
                if let Some(call_id) = last_code_execution_tool_call_id.as_ref() {
                    let outcome = cer.get("outcome").cloned().unwrap_or(json!(null));
                    let output = cer
                        .get("output")
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string)
                        .unwrap_or_default();
                    content.push(GenerateContent::ToolResult(ToolResult {
                        tool_call_id: call_id.clone(),
                        tool_name: code_execution_tool_name.to_string(),
                        result: json!({ "outcome": outcome, "output": output }),
                        is_error: None,
                        preliminary: None,
                        dynamic: None,
                        provider_metadata: Some(server_tool_metadata(
                            call_id,
                            "code_execution",
                            None,
                        )),
                    }));
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
                        content.push(GenerateContent::Reasoning(ReasoningOutput {
                            text: text.to_string(),
                            provider_metadata: thought_sig_meta.clone(),
                        }));
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
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: id,
                    tool_name: name,
                    input: input.to_string(),
                    provider_executed: None,
                    dynamic: None,
                    thought_signature,
                    provider_metadata: thought_sig_meta.clone(),
                }));
                has_tool_calls = true;
            } else if let Some(inline) = part.get("inlineData") {
                // File output (e.g. gemini-2.5-flash-image) — upstream :478-490.
                if let (Some(data), Some(mime)) = (
                    inline.get("data").and_then(|v| v.as_str()),
                    inline.get("mimeType").and_then(|v| v.as_str()),
                ) {
                    let file = GeneratedFile {
                        data: GeneratedFileData::Data {
                            data: FileBytes::Base64(data.to_string()),
                        },
                        media_type: mime.to_string(),
                        provider_metadata: thought_sig_meta.clone(),
                    };
                    content.push(
                        if part
                            .get("thought")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false)
                        {
                            GenerateContent::ReasoningFile(file)
                        } else {
                            GenerateContent::File(file)
                        },
                    );
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
                let server_meta = server_tool_metadata(
                    &id,
                    tool_type,
                    part.get("thoughtSignature").and_then(|v| v.as_str()),
                );
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: id,
                    tool_name: format!("server:{tool_type}"),
                    input: input.to_string(),
                    provider_executed: Some(true),
                    dynamic: Some(true),
                    thought_signature,
                    provider_metadata: Some(server_meta),
                }));
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
                let server_meta = server_tool_metadata(
                    &id,
                    tool_type,
                    part.get("thoughtSignature").and_then(|v| v.as_str()),
                );
                content.push(GenerateContent::ToolResult(ToolResult {
                    tool_call_id: id,
                    tool_name: format!("server:{tool_type}"),
                    result: response,
                    is_error: None,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: Some(server_meta),
                }));
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
        | GenerateContent::Reasoning(ReasoningOutput {
            provider_metadata, ..
        })
        | GenerateContent::ToolCall(RawToolCall {
            provider_metadata, ..
        })
        | GenerateContent::File(GeneratedFile {
            provider_metadata, ..
        })
        | GenerateContent::Source(Source {
            provider_metadata, ..
        })
        | GenerateContent::ReasoningFile(GeneratedFile {
            provider_metadata, ..
        })
        | GenerateContent::Custom {
            provider_metadata, ..
        }
        | GenerateContent::ToolApprovalRequest(aimux_core::result::RawToolApprovalRequest {
            provider_metadata,
            ..
        })
        | GenerateContent::ToolResult(ToolResult {
            provider_metadata, ..
        }) => {
            *provider_metadata = Some(meta);
        }
    }
}

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
