//! Google Gemini language model — implements `LanguageModel`.

use std::collections::{HashMap, HashSet};

use aimux_core::tool::RawToolCall;
use aimux_core::tool::ToolResult;
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{
    GenerateContent, GenerateResult, GeneratedFile, ReasoningOutput, Source, StreamResult,
};
use aimux_core::shared::{FileBytes, GeneratedFileData};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, ProviderMetadata, ResponseMetadata, Usage, Warning,
};
use aimux_provider_utils::{TransformStreamController, Transformer, pipe_through};

use aimux_core::language_model::SupportedUrls;

use super::convert::{
    build_request_body_with_warnings, code_execution_tool_name, convert_usage, extract_sources,
    parse_finish_reason, validate_call_options,
};
use super::options::google_metadata;
use super::types::{Candidate, GenerateContentResponse, GoogleStreamEvent, StreamChunk};
use crate::shared::EndpointConfig;

/// A Google Gemini language model.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct GoogleModel {
    model_id: String,
    config: EndpointConfig,
    generate_id: aimux_provider_utils::IdGenerator,
}

impl GoogleModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self {
            model_id,
            config,
            generate_id: aimux_provider_utils::generate_id,
        }
    }

    pub(crate) fn with_generate_id(
        mut self,
        generate_id: Option<aimux_provider_utils::IdGenerator>,
    ) -> Self {
        if let Some(generate_id) = generate_id {
            self.generate_id = generate_id;
        }
        self
    }

    /// `models/{model}`, or the id itself when it already contains a `/`
    /// (e.g. a fine-tuned path).
    fn model_path(&self) -> String {
        if self.model_id.contains('/') {
            self.model_id.clone()
        } else {
            format!("models/{}", self.model_id)
        }
    }
}

#[async_trait]
impl LanguageModel for GoogleModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn supported_urls(&self) -> SupportedUrls {
        self.config.supported_urls(&self.model_id)
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        validate_call_options(options)?;
        let code_execution_tool_name = code_execution_tool_name(options.tools.as_deref());
        let (body, mut tool_warnings) = build_request_body_with_warnings(&self.model_id, options)?;
        for warning in &mut tool_warnings {
            if let aimux_core::types::Warning::Other { message } = warning {
                *message = message.replace("google.generative-ai", &self.config.provider);
            }
        }
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let url = exchange.url(&format!("/{}:generateContent", self.model_path()));
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(url, options),
            body.clone(),
            aimux_provider_utils::create_json_response_handler(),
            super::google_failed_response_handler(),
        )
        .await?;

        let response_body = resp.raw_value;
        let response_headers = resp.response_headers;

        let data: GenerateContentResponse = resp.value;

        let candidate = data.candidates.into_iter().next().unwrap_or_default();
        let block_reason = confirmed_prompt_block_reason(data.prompt_feedback.as_ref());

        let (content, has_tool_calls) =
            extract_content_from_candidate(&candidate, &code_execution_tool_name, self.generate_id);

        let finish_reason = if candidate.finish_reason.is_none() && block_reason.is_some() {
            FinishReason {
                unified: FinishReasonUnified::ContentFilter,
                raw: block_reason,
            }
        } else {
            candidate
                .finish_reason
                .as_deref()
                .map(|r| parse_finish_reason(r, has_tool_calls))
                .unwrap_or(FinishReason {
                    unified: FinishReasonUnified::Other,
                    raw: None,
                })
        };

        let usage = data
            .usage_metadata
            .as_ref()
            .map(convert_usage)
            .unwrap_or_default();

        // Provider metadata: wrap the raw Google metadata under a `google`
        // key (matching the TS `wrapProviderMetadata`).
        let provider_metadata = Some(google_metadata(serde_json::json!({
            "promptFeedback": data.prompt_feedback,
            "groundingMetadata": candidate.grounding_metadata,
            "urlContextMetadata": candidate.url_context_metadata,
            "safetyRatings": candidate.safety_ratings,
            "usageMetadata": data.usage_metadata,
            "finishMessage": candidate.finish_message,
            "serviceTier": data.usage_metadata.as_ref().and_then(|usage| usage.service_tier.as_ref()),
        })));

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: tool_warnings,
            provider_metadata,
            response: Some(aimux_core::shared::ResponseInfo {
                id: data.response_id,
                timestamp: None,
                model_id: None,
                headers: Some(response_headers),
                body: response_body,
            }),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        validate_call_options(options)?;
        let code_execution_tool_name = code_execution_tool_name(options.tools.as_deref());
        let (body, mut tool_warnings) = build_request_body_with_warnings(&self.model_id, options)?;
        for warning in &mut tool_warnings {
            if let aimux_core::types::Warning::Other { message } = warning {
                *message = message.replace("google.generative-ai", &self.config.provider);
            }
        }
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let endpoint = exchange.url(&format!(
            "/{}:streamGenerateContent?alt=sse",
            self.model_path()
        ));
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(endpoint.clone(), options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            super::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let mut sse_stream = resp.value;
        let first_event = match sse_stream.next().await {
            Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
            first_event => first_event,
        };
        let stream_error_url = endpoint.clone();
        let stream_request_body = body.clone();
        let stream_error_headers = response_headers.clone();

        let generate_id = self.generate_id;
        let include_raw_chunks = options.include_raw_chunks.unwrap_or(false);
        let stream = pipe_through(
            futures::stream::iter(first_event).chain(sse_stream),
            GoogleLanguageModelStream {
                warnings: tool_warnings,
                include_raw_chunks,
                code_execution_tool_name,
                stream_error_url,
                stream_request_body,
                stream_error_headers,
                generate_id,
                text_id: None,
                reasoning_id: None,
                block_counter: 0,
                final_usage: Usage::default(),
                final_finish_reason: None,
                has_tool_calls: false,
                response_metadata_emitted: false,
                stream_errored: false,
                prompt_blocked: false,
                last_grounding_metadata: None,
                last_url_context_metadata: None,
                last_prompt_feedback: None,
                last_safety_ratings: None,
                last_finish_message: None,
                last_usage_metadata_value: None,
                emitted_source_urls: HashSet::new(),
                source_id: 0,
                last_code_execution_tool_call_id: None,
                last_server_tool_call_id: None,
            },
        );

        Ok(StreamResult {
            stream: Box::pin(stream),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
            response: Some(aimux_core::shared::StreamResponseInfo {
                headers: Some(response_headers),
            }),
        })
    }
}

/// The `TransformStream` of `GoogleLanguageModel.doStream`.
struct GoogleLanguageModelStream {
    warnings: Vec<Warning>,
    include_raw_chunks: bool,
    code_execution_tool_name: String,
    stream_error_url: String,
    stream_request_body: Value,
    stream_error_headers: HashMap<String, String>,
    generate_id: aimux_provider_utils::IdGenerator,
    text_id: Option<String>,
    reasoning_id: Option<String>,
    block_counter: usize,
    final_usage: Usage,
    final_finish_reason: Option<FinishReason>,
    has_tool_calls: bool,
    response_metadata_emitted: bool,
    stream_errored: bool,
    prompt_blocked: bool,
    // Provider-metadata accumulators (mirrors TS `lastGroundingMetadata` /
    // `lastUrlContextMetadata` + the finishReason-chunk snapshot).
    last_grounding_metadata: Option<Value>,
    last_url_context_metadata: Option<Value>,
    last_prompt_feedback: Option<Value>,
    last_safety_ratings: Option<Value>,
    last_finish_message: Option<Value>,
    last_usage_metadata_value: Option<Value>,
    // Source dedup across chunks (url sources only).
    emitted_source_urls: HashSet<String>,
    source_id: usize,
    // Associates code-execution results / server tool responses with their
    // preceding call.
    last_code_execution_tool_call_id: Option<String>,
    last_server_tool_call_id: Option<String>,
}

impl GoogleLanguageModelStream {
    fn transform_chunk(
        &mut self,
        chunk: Box<StreamChunk>,
        controller: &mut TransformStreamController<StreamPart>,
    ) {
        // Emit ResponseMetadata from the first chunk that has
        // a responseId (matches TS behaviour).
        if !self.response_metadata_emitted
            && let Some(id) = &chunk.response_id
        {
            self.response_metadata_emitted = true;
            controller.enqueue(StreamPart::ResponseMetadata(ResponseMetadata {
                id: Some(id.clone()),
                timestamp: None,
                model_id: None,
            }));
        }

        if let Some(usage) = &chunk.usage_metadata {
            self.final_usage = convert_usage(usage);
            self.last_usage_metadata_value = serde_json::to_value(usage).ok();
        }
        if let Some(pf) = &chunk.prompt_feedback
            && !self.prompt_blocked
        {
            self.last_prompt_feedback = Some(pf.clone());
            if let Some(reason) = confirmed_prompt_block_reason(Some(pf)) {
                self.prompt_blocked = true;
                self.final_finish_reason = Some(FinishReason {
                    unified: FinishReasonUnified::ContentFilter,
                    raw: Some(reason),
                });
            }
        }

        let Some(candidates) = chunk.candidates else {
            return;
        };
        let Some(candidate) = candidates.into_iter().next() else {
            return;
        };

        if let Some(gm) = &candidate.grounding_metadata {
            self.last_grounding_metadata = Some(gm.clone());
        }
        if let Some(ucm) = &candidate.url_context_metadata {
            self.last_url_context_metadata = Some(ucm.clone());
        }

        if let Some(sr) = &candidate.safety_ratings {
            self.last_safety_ratings = serde_json::to_value(sr).ok();
        }
        if let Some(fm) = &candidate.finish_message {
            self.last_finish_message = Some(json!(fm));
        }
        if self.prompt_blocked {
            return;
        }

        // Extract url sources from this chunk's grounding metadata
        // (deduplicated across chunks; document sources are not
        // emitted in the stream, matching TS).
        let chunk_sources =
            extract_sources(candidate.grounding_metadata.as_ref(), &mut self.source_id);
        for src in chunk_sources {
            if let GenerateContent::Source(Source::Url {
                url,
                id: _,
                title,
                provider_metadata: None,
            }) = src
                && self.emitted_source_urls.insert(url.clone())
            {
                controller.enqueue(StreamPart::Source(Source::Url {
                    id: (self.generate_id)(),
                    url,
                    title,
                    provider_metadata: None,
                }));
            }
        }

        if let Some(parts) = candidate.content.as_ref().and_then(|c| c.parts.as_ref()) {
            for part in parts {
                // thoughtSignature → provider_metadata (upstream :778-782)
                let thought_sig_meta: Option<ProviderMetadata> = part
                    .get("thoughtSignature")
                    .and_then(|v| v.as_str())
                    .filter(|signature| !signature.is_empty())
                    .map(|s| google_metadata(json!({ "thoughtSignature": s })));

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
                        if let Some(meta) = &thought_sig_meta
                            && let Some(id) = &self.text_id
                        {
                            controller.enqueue(StreamPart::TextDelta {
                                id: id.clone(),
                                delta: String::new(),
                                provider_metadata: Some(meta.clone()),
                            });
                        }
                    } else {
                        let is_thought = part
                            .get("thought")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        if is_thought {
                            // Close any open text block before starting reasoning
                            if let Some(id) = self.text_id.take() {
                                controller.enqueue(StreamPart::TextEnd {
                                    id,
                                    provider_metadata: None,
                                });
                            }
                            // Start a reasoning block if not already active
                            if self.reasoning_id.is_none() {
                                let id = format!("{}", self.block_counter);
                                self.block_counter += 1;
                                self.reasoning_id = Some(id.clone());
                                controller.enqueue(StreamPart::ReasoningStart {
                                    id,
                                    provider_metadata: thought_sig_meta.clone(),
                                });
                            }
                            if let Some(id) = &self.reasoning_id {
                                controller.enqueue(StreamPart::ReasoningDelta {
                                    id: id.clone(),
                                    delta: text.to_string(),
                                    provider_metadata: thought_sig_meta.clone(),
                                });
                            }
                        } else {
                            // Close any open reasoning block before starting text
                            if let Some(id) = self.reasoning_id.take() {
                                controller.enqueue(StreamPart::ReasoningEnd {
                                    id,
                                    provider_metadata: None,
                                });
                            }
                            if self.text_id.is_none() {
                                let id = format!("{}", self.block_counter);
                                self.block_counter += 1;
                                self.text_id = Some(id.clone());
                                controller.enqueue(StreamPart::TextStart {
                                    id,
                                    provider_metadata: thought_sig_meta.clone(),
                                });
                            }
                            if let Some(id) = &self.text_id {
                                controller.enqueue(StreamPart::TextDelta {
                                    id: id.clone(),
                                    delta: text.to_string(),
                                    provider_metadata: thought_sig_meta.clone(),
                                });
                            }
                        }
                    }
                } else if let Some(ec) = part.get("executableCode") {
                    // Provider-executed code execution.
                    let has_code = ec
                        .get("code")
                        .and_then(|v| v.as_str())
                        .map(|s| !s.is_empty())
                        .unwrap_or(false);
                    if has_code {
                        let id = (self.generate_id)();
                        self.last_code_execution_tool_call_id = Some(id.clone());
                        controller.enqueue(StreamPart::ToolCall(RawToolCall {
                            tool_call_id: id.clone(),
                            tool_name: self.code_execution_tool_name.clone(),
                            input: ec.to_string(),
                            provider_executed: Some(true),
                            dynamic: None,
                            provider_metadata: None,
                        }));
                        // provider-executed → does NOT set has_tool_calls
                    }
                } else if let Some(cer) = part.get("codeExecutionResult") {
                    // Result corresponds to the most recent
                    // executableCode part. Gemini may emit
                    // several results for that one call, so
                    // retain the association until a new call.
                    if let Some(call_id) = self.last_code_execution_tool_call_id.as_ref() {
                        let outcome = cer.get("outcome").cloned().unwrap_or(json!(null));
                        let output = cer
                            .get("output")
                            .and_then(|v| v.as_str())
                            .map(std::string::ToString::to_string)
                            .unwrap_or_default();
                        controller.enqueue(StreamPart::ToolResult(ToolResult {
                            tool_call_id: call_id.clone(),
                            tool_name: self.code_execution_tool_name.clone(),
                            result: json!({ "outcome": outcome, "output": output }),
                            is_error: None,
                            preliminary: None,
                            dynamic: None,
                            provider_metadata: None,
                        }));
                    }
                } else if let Some(tc) = part.get("toolCall") {
                    // Server-side tool call (provider-executed).
                    let tool_type = tc.get("toolType").and_then(|v| v.as_str()).unwrap_or("");
                    let id = tc
                        .get("id")
                        .and_then(|v| v.as_str())
                        .filter(|id| !id.is_empty())
                        .map(std::string::ToString::to_string)
                        .unwrap_or_else(self.generate_id);
                    self.last_server_tool_call_id = Some(id.clone());
                    let args = tc.get("args").cloned().unwrap_or(json!({}));
                    let server_meta = server_tool_metadata(
                        &id,
                        tool_type,
                        part.get("thoughtSignature").and_then(|v| v.as_str()),
                    );
                    controller.enqueue(StreamPart::ToolCall(RawToolCall {
                        tool_call_id: id,
                        tool_name: format!("server:{tool_type}"),
                        input: args.to_string(),
                        provider_executed: Some(true),
                        dynamic: Some(true),
                        provider_metadata: Some(server_meta),
                    }));
                    // provider-executed → does NOT set has_tool_calls
                } else if let Some(tr) = part.get("toolResponse") {
                    // Server-side tool response.
                    let tool_type = tr.get("toolType").and_then(|v| v.as_str()).unwrap_or("");
                    let id = self
                        .last_server_tool_call_id
                        .take()
                        .or_else(|| {
                            tr.get("id")
                                .and_then(|v| v.as_str())
                                .map(std::string::ToString::to_string)
                        })
                        .filter(|id| !id.is_empty())
                        .unwrap_or_else(self.generate_id);
                    let response = tr.get("response").cloned().unwrap_or(json!({}));
                    let server_meta = server_tool_metadata(
                        &id,
                        tool_type,
                        part.get("thoughtSignature").and_then(|v| v.as_str()),
                    );
                    controller.enqueue(StreamPart::ToolResult(ToolResult {
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
                    if let Some(id) = self.text_id.take() {
                        controller.enqueue(StreamPart::TextEnd {
                            id,
                            provider_metadata: None,
                        });
                    }
                    if let Some(id) = self.reasoning_id.take() {
                        controller.enqueue(StreamPart::ReasoningEnd {
                            id,
                            provider_metadata: None,
                        });
                    }
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
                        controller.enqueue(
                            if part
                                .get("thought")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false)
                            {
                                StreamPart::ReasoningFile(file)
                            } else {
                                StreamPart::File(file)
                            },
                        );
                    }
                }
            }
            for part in parts {
                let thought_sig_meta = part
                    .get("thoughtSignature")
                    .and_then(Value::as_str)
                    .filter(|signature| !signature.is_empty())
                    .map(|signature| google_metadata(json!({"thoughtSignature": signature})));
                if let Some(fc) = part.get("functionCall") {
                    // Single-chunk complete function call (the
                    // common case for the public Gemini API).
                    if fc.get("name").and_then(Value::as_str).is_none()
                        || fc.get("partialArgs").is_some_and(|value| !value.is_null())
                        || fc.get("willContinue").and_then(Value::as_bool) == Some(true)
                    {
                        continue;
                    }
                    let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let id = fc
                        .get("id")
                        .and_then(|v| v.as_str())
                        .filter(|id| !id.is_empty())
                        .filter(|id| !id.is_empty())
                        .map(std::string::ToString::to_string)
                        .unwrap_or_else(self.generate_id);
                    let args = fc
                        .get("args")
                        .filter(|value| !value.is_null())
                        .cloned()
                        .unwrap_or(json!({}));

                    controller.enqueue(StreamPart::ToolInputStart {
                        id: id.clone(),
                        tool_name: name.to_string(),
                        provider_executed: None,
                        dynamic: None,
                        title: None,
                        provider_metadata: thought_sig_meta.clone(),
                    });
                    let args_str = args
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| args.to_string());
                    if fc.get("args").is_some_and(|value| !value.is_null()) {
                        controller.enqueue(StreamPart::ToolInputDelta {
                            id: id.clone(),
                            delta: args_str.clone(),
                            provider_metadata: thought_sig_meta.clone(),
                        });
                    }
                    controller.enqueue(StreamPart::ToolInputEnd {
                        id: id.clone(),
                        provider_metadata: thought_sig_meta.clone(),
                    });
                    controller.enqueue(StreamPart::ToolCall(RawToolCall {
                        tool_call_id: id,
                        tool_name: name.to_string(),
                        input: args_str,
                        provider_executed: None,
                        dynamic: None,
                        provider_metadata: thought_sig_meta.clone(),
                    }));
                    self.has_tool_calls = true;
                }
            }
        }

        if let Some(reason) = candidate.finish_reason.as_deref() {
            self.final_finish_reason = Some(parse_finish_reason(reason, self.has_tool_calls));
        }
    }
}

impl Transformer for GoogleLanguageModelStream {
    type Input = Result<Value, AiMuxError>;
    type Output = StreamPart;

    fn start(&mut self, controller: &mut TransformStreamController<StreamPart>) {
        controller.enqueue(StreamPart::StreamStart {
            warnings: std::mem::take(&mut self.warnings),
        });
    }

    fn transform(
        &mut self,
        event: Result<Value, AiMuxError>,
        controller: &mut TransformStreamController<StreamPart>,
    ) {
        if self.stream_errored {
            return;
        }
        let event = match event {
            Ok(raw_value) => {
                if self.include_raw_chunks {
                    controller.enqueue(StreamPart::Raw {
                        raw_value: raw_value.clone(),
                    });
                }
                serde_json::from_value::<GoogleStreamEvent>(raw_value).map_err(AiMuxError::from)
            }
            Err(error) => Err(error),
        };

        match event {
            Ok(GoogleStreamEvent::Chunk(chunk)) => self.transform_chunk(chunk, controller),
            Ok(GoogleStreamEvent::Error(error)) => {
                controller.enqueue(StreamPart::Error {
                    error: super::google_stream_error(
                        &error.error,
                        &self.stream_error_url,
                        self.stream_request_body.clone(),
                        self.stream_error_headers.clone(),
                    ),
                });
                self.stream_errored = true;
            }
            Err(error) => {
                let recoverable = error.is_recoverable_stream_error();
                controller.enqueue(StreamPart::Error { error });
                if !recoverable {
                    controller.terminate();
                }
            }
        }
    }

    fn flush(mut self, controller: &mut TransformStreamController<StreamPart>) {
        // Close any remaining open text/reasoning segment.
        if let Some(id) = self.text_id.take() {
            controller.enqueue(StreamPart::TextEnd {
                id,
                provider_metadata: None,
            });
        }
        if let Some(id) = self.reasoning_id.take() {
            controller.enqueue(StreamPart::ReasoningEnd {
                id,
                provider_metadata: None,
            });
        }

        let provider_metadata = Some(google_metadata(serde_json::json!({
            "promptFeedback": self.last_prompt_feedback,
            "groundingMetadata": self.last_grounding_metadata,
            "urlContextMetadata": self.last_url_context_metadata,
            "safetyRatings": self.last_safety_ratings,
            "usageMetadata": self.last_usage_metadata_value,
            "finishMessage": self.last_finish_message,
            "serviceTier": self.last_usage_metadata_value.as_ref().and_then(|usage| usage.get("serviceTier")),
        })));

        controller.enqueue(StreamPart::Finish {
            finish_reason: if self.stream_errored {
                FinishReason {
                    unified: FinishReasonUnified::Error,
                    raw: None,
                }
            } else {
                self.final_finish_reason.unwrap_or(FinishReason {
                    unified: FinishReasonUnified::Other,
                    raw: None,
                })
            },
            usage: if self.stream_errored {
                Usage::default()
            } else {
                self.final_usage
            },
            provider_metadata,
        });
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
    google_metadata(payload)
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
    generate_id: aimux_provider_utils::IdGenerator,
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
                .filter(|signature| !signature.is_empty())
                .map(|s| google_metadata(json!({ "thoughtSignature": s })));

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
                    let id = generate_id();
                    last_code_execution_tool_call_id = Some(id.clone());
                    content.push(GenerateContent::ToolCall(RawToolCall {
                        tool_call_id: id.clone(),
                        tool_name: code_execution_tool_name.to_string(),
                        input: ec.to_string(),
                        provider_executed: Some(true),
                        dynamic: None,
                        provider_metadata: None,
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
                        provider_metadata: None,
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
            } else if let Some(fc) = part
                .get("functionCall")
                .filter(|fc| fc.get("name").and_then(Value::as_str).is_some())
            {
                let name = fc
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let id = fc
                    .get("id")
                    .and_then(|v| v.as_str())
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(generate_id);
                let input = fc
                    .get("args")
                    .filter(|value| !value.is_null())
                    .cloned()
                    .unwrap_or(json!({}));
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: id,
                    tool_name: name,
                    input: input.to_string(),
                    provider_executed: None,
                    dynamic: None,
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
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(generate_id);
                last_server_tool_call_id = Some(id.clone());
                let input = tc.get("args").cloned().unwrap_or(json!({}));
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
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(generate_id);
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
    for source in &mut sources {
        if let GenerateContent::Source(Source::Url { id, .. } | Source::Document { id, .. }) =
            source
        {
            *id = generate_id();
        }
    }
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
        | GenerateContent::Source(Source::Url {
            provider_metadata, ..
        })
        | GenerateContent::Source(Source::Document {
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

fn confirmed_prompt_block_reason(feedback: Option<&Value>) -> Option<String> {
    feedback
        .and_then(|feedback| feedback.get("blockReason"))
        .and_then(Value::as_str)
        .filter(|reason| {
            !reason.is_empty()
                && *reason != "BLOCK_REASON_UNSPECIFIED"
                && *reason != "BLOCKED_REASON_UNSPECIFIED"
        })
        .map(str::to_owned)
}
