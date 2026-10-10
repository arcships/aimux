//! Google Vertex AI language model — implements `LanguageModel`.
//!
//! Reuses the shared [`crate::google::convert`] message conversion logic and
//! [`crate::google::types`] response types. Only the endpoint construction and
//! authentication differ from the public Gemini API provider.

use std::collections::{HashMap, HashSet};

use aimux_core::tool::RawToolCall;
use aimux_core::tool::ToolResult;
use async_trait::async_trait;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{
    GenerateContent, GenerateResult, GeneratedFile, ReasoningOutput, Source, StreamResult,
};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, ProviderMetadata, ResponseMetadata, Usage, Warning,
};
use aimux_provider_utils::{TransformStreamController, Transformer, pipe_through};

use crate::google::convert::{
    build_vertex_request_body, code_execution_tool_name, extract_sources, parse_finish_reason,
    prepare_all_tools, validate_call_options_for_namespace,
};
use crate::google::types::{Candidate, GenerateContentResponse, GoogleStreamEvent, StreamChunk};
use crate::google::utils::{GoogleJsonAccumulator, PartialArg};

use crate::google::options::{GOOGLE, Namespace};
use crate::shared::EndpointConfig;
use aimux_core::language_model::SupportedUrls;
use aimux_core::shared::{FileBytes, GeneratedFileData};

/// A Google Vertex AI language model.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct VertexModel {
    model_id: String,
    config: EndpointConfig,
}

impl VertexModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }

    fn model_path(&self) -> String {
        if self.model_id.contains('/') {
            self.model_id.clone()
        } else {
            format!("models/{}", self.model_id)
        }
    }

    /// `…/models/{model}:generateContent`
    fn generate_endpoint(&self, base_url: &str) -> String {
        format!("{}/{}:generateContent", base_url, self.model_path())
    }

    /// `…/models/{model}:streamGenerateContent?alt=sse`
    fn stream_endpoint(&self, base_url: &str) -> String {
        format!(
            "{}/{}:streamGenerateContent?alt=sse",
            base_url,
            self.model_path()
        )
    }
}

#[async_trait]
impl LanguageModel for VertexModel {
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
        let options = vertex_call_options(options)?;
        let options = &options;
        let code_execution_tool_name = code_execution_tool_name(options.tools.as_deref());
        let (body, warnings) = vertex_request_body(&self.model_id, options, false)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(self.generate_endpoint(exchange.base_url()), options),
            body.clone(),
            aimux_provider_utils::create_json_response_handler(),
            crate::google::google_failed_response_handler(),
        )
        .await?;

        let response_body = resp.raw_value;
        let response_headers = resp.response_headers;

        let raw: Value = resp.value;
        let mut parsed_response = raw.clone();
        if parsed_response
            .get("candidates")
            .is_some_and(Value::is_null)
        {
            parsed_response["candidates"] = json!([]);
        }
        let data: GenerateContentResponse = serde_json::from_value(parsed_response)
            .map_err(|error| AiMuxError::InvalidResponseData(error.to_string()))?;

        let candidate = data.candidates.into_iter().next().unwrap_or(Candidate {
            content: None,
            finish_reason: None,
            finish_message: None,
            safety_ratings: None,
            grounding_metadata: None,
            url_context_metadata: None,
            index: None,
        });
        let (content, has_tool_calls) =
            extract_content_from_candidate(&candidate, &code_execution_tool_name);
        let block_reason = confirmed_block_reason(data.prompt_feedback.as_ref());
        let finish_reason = candidate
            .finish_reason
            .as_deref()
            .map(|r| parse_finish_reason(r, has_tool_calls))
            .unwrap_or(FinishReason {
                unified: if block_reason.is_some() {
                    FinishReasonUnified::ContentFilter
                } else {
                    FinishReasonUnified::Other
                },
                raw: block_reason,
            });
        let usage = vertex_usage(raw.get("usageMetadata"));

        let provider_metadata = Some(vertex_provider_metadata(json!({
            "promptFeedback": data.prompt_feedback,
            "groundingMetadata": candidate.grounding_metadata,
            "urlContextMetadata": candidate.url_context_metadata,
            "safetyRatings": candidate.safety_ratings,
            "usageMetadata": raw.get("usageMetadata"),
            "serviceTier": raw.pointer("/usageMetadata/serviceTier"),
            "finishMessage": candidate.finish_message,
        })));

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings,
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
        let options = vertex_call_options(options)?;
        let options = &options;
        let code_execution_tool_name = code_execution_tool_name(options.tools.as_deref());
        let (body, warnings) = vertex_request_body(&self.model_id, options, true)?;
        let include_raw_chunks = options.include_raw_chunks == Some(true);
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let endpoint = self.stream_endpoint(exchange.base_url());
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(endpoint.clone(), options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            crate::google::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let sse_stream = resp.value;
        let stream_error_url = endpoint;
        let stream_request_body = body.clone();
        let stream_error_headers = response_headers.clone();

        let stream = pipe_through(
            sse_stream,
            VertexModelStream {
                warnings,
                include_raw_chunks,
                code_execution_tool_name,
                stream_error_url,
                stream_request_body,
                stream_error_headers,
                text_id: None,
                reasoning_id: None,
                block_counter: 0,
                final_usage: Usage::default(),
                final_finish_reason: None,
                has_tool_calls: false,
                active_tool_calls: Vec::new(),
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

/// The `TransformStream` of `GoogleLanguageModel.doStream` as configured by
/// `@ai-sdk/google-vertex`.
struct VertexModelStream {
    warnings: Vec<Warning>,
    include_raw_chunks: bool,
    code_execution_tool_name: String,
    stream_error_url: String,
    stream_request_body: Value,
    stream_error_headers: HashMap<String, String>,
    text_id: Option<String>,
    reasoning_id: Option<String>,
    block_counter: usize,
    final_usage: Usage,
    final_finish_reason: Option<FinishReason>,
    has_tool_calls: bool,
    active_tool_calls: Vec<(RawToolCall, GoogleJsonAccumulator)>,
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

impl VertexModelStream {
    fn transform_chunk(
        &mut self,
        chunk: Box<StreamChunk>,
        controller: &mut TransformStreamController<StreamPart>,
    ) {
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

        if !self.prompt_blocked
            && let Some(pf) = &chunk.prompt_feedback
        {
            self.last_prompt_feedback = Some(pf.clone());
            if let Some(reason) = confirmed_block_reason(Some(pf)) {
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
                id,
                title,
                provider_metadata: None,
            }) = src
                && self.emitted_source_urls.insert(url.clone())
            {
                controller.enqueue(StreamPart::Source(Source::Url {
                    id,
                    url,
                    title,
                    provider_metadata: None,
                }));
            }
        }

        if let Some(parts) = candidate.content.as_ref().and_then(|c| c.parts.as_ref()) {
            for part in parts
                .iter()
                .filter(|part| part.get("functionCall").is_none())
                .chain(
                    parts
                        .iter()
                        .filter(|part| part.get("functionCall").is_some()),
                )
            {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    let metadata = thought_metadata(part);
                    if text.is_empty() {
                        if metadata.is_some()
                            && let Some(id) = &self.text_id
                        {
                            controller.enqueue(StreamPart::TextDelta {
                                id: id.clone(),
                                delta: String::new(),
                                provider_metadata: metadata,
                            });
                        }
                    } else if part.get("thought") == Some(&Value::Bool(true)) {
                        if let Some(id) = self.text_id.take() {
                            controller.enqueue(StreamPart::TextEnd {
                                id,
                                provider_metadata: None,
                            });
                        }
                        if self.reasoning_id.is_none() {
                            let id = format!("{}", self.block_counter);
                            self.block_counter += 1;
                            self.reasoning_id = Some(id.clone());
                            controller.enqueue(StreamPart::ReasoningStart {
                                id,
                                provider_metadata: metadata.clone(),
                            });
                        }
                        controller.enqueue(StreamPart::ReasoningDelta {
                            id: self.reasoning_id.clone().unwrap(),
                            delta: text.to_string(),
                            provider_metadata: metadata,
                        });
                    } else {
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
                                provider_metadata: metadata.clone(),
                            });
                        }
                        controller.enqueue(StreamPart::TextDelta {
                            id: self.text_id.clone().unwrap(),
                            delta: text.to_string(),
                            provider_metadata: metadata,
                        });
                    }
                } else if let Some(file) = inline_file(part) {
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
                    controller.enqueue(if part.get("thought") == Some(&Value::Bool(true)) {
                        StreamPart::ReasoningFile(file)
                    } else {
                        StreamPart::File(file)
                    });
                } else if let Some(fc) = part.get("functionCall") {
                    let name = fc.get("name").and_then(Value::as_str);
                    let args = fc.get("args").filter(|value| !value.is_null());
                    let partial_args = fc.get("partialArgs").filter(|value| !value.is_null());
                    let will_continue =
                        fc.get("willContinue").and_then(Value::as_bool) == Some(true);
                    let thought_signature = part
                        .get("thoughtSignature")
                        .and_then(Value::as_str)
                        .filter(|signature| !signature.is_empty())
                        .map(str::to_owned);
                    let tool_metadata = thought_signature.as_deref().map(|signature| {
                        vertex_provider_metadata(json!({ "thoughtSignature": signature }))
                    });
                    if partial_args.is_some() || (name.is_some() && will_continue) {
                        if let Some(name) = name {
                            let id = fc
                                .get("id")
                                .and_then(Value::as_str)
                                .filter(|id| !id.is_empty())
                                .map(str::to_owned)
                                .unwrap_or_else(aimux_provider_utils::generate_id);
                            controller.enqueue(StreamPart::ToolInputStart {
                                id: id.clone(),
                                tool_name: name.to_owned(),
                                provider_executed: None,
                                dynamic: None,
                                title: None,
                                provider_metadata: tool_metadata.clone(),
                            });
                            self.active_tool_calls.push((
                                RawToolCall {
                                    tool_call_id: id,
                                    tool_name: name.to_owned(),
                                    input: String::new(),
                                    provider_executed: None,
                                    dynamic: None,
                                    provider_metadata: tool_metadata.clone(),
                                },
                                GoogleJsonAccumulator::new(),
                            ));
                        }
                        if let Some(partial_args) = partial_args.and_then(Value::as_array)
                            && let Some((call, accumulator)) = self.active_tool_calls.last_mut()
                        {
                            let partial_args: Vec<PartialArg> = partial_args
                                .iter()
                                .map(|arg| PartialArg {
                                    json_path: arg
                                        .get("jsonPath")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_owned(),
                                    string_value: arg
                                        .get("stringValue")
                                        .and_then(Value::as_str)
                                        .map(str::to_owned),
                                    number_value: arg.get("numberValue").and_then(Value::as_f64),
                                    bool_value: arg.get("boolValue").and_then(Value::as_bool),
                                    null_value: arg.get("nullValue").map(|_| ()),
                                    will_continue: arg.get("willContinue").and_then(Value::as_bool),
                                })
                                .collect();
                            match accumulator.process_partial_args(&partial_args) {
                                Ok(result) => {
                                    if !result.text_delta.is_empty() {
                                        controller.enqueue(StreamPart::ToolInputDelta {
                                            id: call.tool_call_id.clone(),
                                            delta: result.text_delta,
                                            provider_metadata: tool_metadata,
                                        });
                                    }
                                }
                                Err(error) => {
                                    controller.enqueue(StreamPart::Error { error });
                                    controller.terminate();
                                    return;
                                }
                            }
                            if !will_continue
                                && partial_args
                                    .iter()
                                    .all(|arg| arg.will_continue != Some(true))
                            {
                                for event in finish_streaming_tool_call(&mut self.active_tool_calls)
                                {
                                    controller.enqueue(event);
                                }
                                self.has_tool_calls = true;
                            }
                        }
                        continue;
                    }
                    if name.is_none()
                        && args.is_none()
                        && partial_args.is_none()
                        && fc.get("willContinue").is_none_or(Value::is_null)
                    {
                        if !self.active_tool_calls.is_empty() {
                            for event in finish_streaming_tool_call(&mut self.active_tool_calls) {
                                controller.enqueue(event);
                            }
                            self.has_tool_calls = true;
                        }
                        continue;
                    }
                    let Some(name) = name else {
                        continue;
                    };
                    let id = fc
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(aimux_provider_utils::generate_id);
                    let args = args.cloned().unwrap_or(json!({}));
                    controller.enqueue(StreamPart::ToolInputStart {
                        id: id.clone(),
                        tool_name: name.to_string(),
                        provider_executed: None,
                        dynamic: None,
                        title: None,
                        provider_metadata: tool_metadata.clone(),
                    });
                    let args_str = args
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| args.to_string());
                    if fc.get("args").is_some_and(|value| !value.is_null()) {
                        controller.enqueue(StreamPart::ToolInputDelta {
                            id: id.clone(),
                            delta: args_str.clone(),
                            provider_metadata: tool_metadata.clone(),
                        });
                    }
                    controller.enqueue(StreamPart::ToolInputEnd {
                        id: id.clone(),
                        provider_metadata: tool_metadata.clone(),
                    });
                    controller.enqueue(StreamPart::ToolCall(RawToolCall {
                        tool_call_id: id,
                        tool_name: name.to_string(),
                        input: args_str,
                        provider_executed: None,
                        dynamic: None,
                        provider_metadata: tool_metadata,
                    }));
                    self.has_tool_calls = true;
                } else if let Some(inline) = part.get("inlineData") {
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
                    if let (Some(data), Some(media_type)) = (
                        inline.get("data").and_then(Value::as_str),
                        inline.get("mimeType").and_then(Value::as_str),
                    ) {
                        let file = GeneratedFile {
                            data: GeneratedFileData::Data {
                                data: FileBytes::Base64(data.to_string()),
                            },
                            media_type: media_type.to_string(),
                            provider_metadata: thought_metadata(part),
                        };
                        controller.enqueue(
                            if part.get("thought").and_then(Value::as_bool) == Some(true) {
                                StreamPart::ReasoningFile(file)
                            } else {
                                StreamPart::File(file)
                            },
                        );
                    }
                } else if let Some(ec) = part.get("executableCode") {
                    // Provider-executed code execution.
                    let has_code = ec
                        .get("code")
                        .and_then(|v| v.as_str())
                        .map(|s| !s.is_empty())
                        .unwrap_or(false);
                    if has_code {
                        let id = format!("call-{}", self.block_counter);
                        self.block_counter += 1;
                        self.last_code_execution_tool_call_id = Some(id.clone());
                        controller.enqueue(StreamPart::ToolCall(RawToolCall {
                            tool_call_id: id.clone(),
                            tool_name: self.code_execution_tool_name.clone(),
                            input: ec.to_string(),
                            provider_executed: Some(true),
                            dynamic: None,
                            provider_metadata: Some(vertex_server_tool_metadata(
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
                            provider_metadata: Some(vertex_server_tool_metadata(
                                call_id,
                                "code_execution",
                                None,
                            )),
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
                        .unwrap_or_else(aimux_provider_utils::generate_id);
                    self.block_counter += 1;
                    self.last_server_tool_call_id = Some(id.clone());
                    let args = tc.get("args").cloned().unwrap_or(json!({}));
                    let thought_signature = part
                        .get("thoughtSignature")
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string);
                    let server_meta =
                        vertex_server_tool_metadata(&id, tool_type, thought_signature.as_deref());
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
                        .unwrap_or_else(|| format!("call-{}", self.block_counter));
                    self.block_counter += 1;
                    let response = tr.get("response").cloned().unwrap_or(json!({}));
                    let server_meta = vertex_server_tool_metadata(
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
                }
            }
        }

        if let Some(reason) = candidate.finish_reason.as_deref() {
            self.final_finish_reason = Some(parse_finish_reason(reason, self.has_tool_calls));
        }
    }
}

impl Transformer for VertexModelStream {
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
            Ok(raw) => {
                if self.include_raw_chunks {
                    controller.enqueue(StreamPart::Raw {
                        raw_value: raw.clone(),
                    });
                }
                if let Some(usage) = raw.get("usageMetadata") {
                    self.final_usage = vertex_usage(Some(usage));
                    self.last_usage_metadata_value = Some(usage.clone());
                }
                serde_json::from_value::<GoogleStreamEvent>(raw)
                    .map_err(|error| AiMuxError::InvalidResponseData(error.to_string()))
            }
            Err(error) => Err(error),
        };

        match event {
            Ok(GoogleStreamEvent::Chunk(chunk)) => self.transform_chunk(chunk, controller),
            Ok(GoogleStreamEvent::Error(error)) => {
                controller.enqueue(StreamPart::Error {
                    error: crate::google::google_stream_error(
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
        let provider_metadata = Some(vertex_provider_metadata(json!({
            "promptFeedback": self.last_prompt_feedback,
            "groundingMetadata": self.last_grounding_metadata,
            "urlContextMetadata": self.last_url_context_metadata,
            "safetyRatings": self.last_safety_ratings,
            "serviceTier": self.last_usage_metadata_value.as_ref().and_then(|usage| usage.get("serviceTier")),
            "usageMetadata": self.last_usage_metadata_value,
            "finishMessage": self.last_finish_message,
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

fn finish_streaming_tool_call(
    active_tool_calls: &mut Vec<(RawToolCall, GoogleJsonAccumulator)>,
) -> Vec<StreamPart> {
    let Some((mut call, accumulator)) = active_tool_calls.pop() else {
        return Vec::new();
    };
    let result = accumulator.finalize();
    let mut events = Vec::new();
    if !result.closing_delta.is_empty() {
        events.push(StreamPart::ToolInputDelta {
            id: call.tool_call_id.clone(),
            delta: result.closing_delta,
            provider_metadata: call.provider_metadata.clone(),
        });
    }
    events.push(StreamPart::ToolInputEnd {
        id: call.tool_call_id.clone(),
        provider_metadata: call.provider_metadata.clone(),
    });
    call.input = result.final_json;
    events.push(StreamPart::ToolCall(call));
    events
}

fn vertex_provider_metadata(mut payload: Value) -> ProviderMetadata {
    if let Some(object) = payload.as_object_mut()
        && object.get("serviceTier").is_some_and(Value::is_null)
    {
        object.remove("serviceTier");
    }
    Namespace::Vertex.metadata(payload)
}

fn vertex_server_tool_metadata(
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
    vertex_provider_metadata(payload)
}

/// Extract `GenerateContent` items from a non-streaming candidate.
fn extract_content_from_candidate(
    candidate: &Candidate,
    code_execution_tool_name: &str,
) -> (Vec<GenerateContent>, bool) {
    let mut content: Vec<GenerateContent> = Vec::new();
    let mut has_tool_calls = false;
    let mut source_id = 0usize;
    let mut last_code_execution_tool_call_id: Option<String> = None;
    let mut last_server_tool_call_id: Option<String> = None;

    let parts = candidate.content.as_ref().and_then(|c| c.parts.as_ref());

    if let Some(parts) = parts {
        for part in parts {
            if let Some(ec) = part.get("executableCode") {
                let has_code = ec
                    .get("code")
                    .and_then(|v| v.as_str())
                    .is_some_and(|code| !code.is_empty());
                if has_code {
                    let id = format!("call-{}", content.len());
                    last_code_execution_tool_call_id = Some(id.clone());
                    content.push(GenerateContent::ToolCall(RawToolCall {
                        tool_call_id: id.clone(),
                        tool_name: code_execution_tool_name.to_string(),
                        input: ec.to_string(),
                        provider_executed: Some(true),
                        dynamic: None,
                        provider_metadata: Some(vertex_server_tool_metadata(
                            &id,
                            "code_execution",
                            None,
                        )),
                    }));
                }
            } else if let Some(cer) = part.get("codeExecutionResult") {
                // One executableCode may be followed by multiple results.
                if let Some(call_id) = last_code_execution_tool_call_id.as_ref() {
                    let outcome = cer.get("outcome").cloned().unwrap_or(json!(null));
                    let output = cer
                        .get("output")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    content.push(GenerateContent::ToolResult(ToolResult {
                        tool_call_id: call_id.clone(),
                        tool_name: code_execution_tool_name.to_string(),
                        result: json!({ "outcome": outcome, "output": output }),
                        is_error: None,
                        preliminary: None,
                        dynamic: None,
                        provider_metadata: Some(vertex_server_tool_metadata(
                            call_id,
                            "code_execution",
                            None,
                        )),
                    }));
                }
            } else if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                let provider_metadata = thought_metadata(part);
                if !text.is_empty() {
                    content.push(if part.get("thought") == Some(&Value::Bool(true)) {
                        GenerateContent::Reasoning(ReasoningOutput {
                            text: text.to_string(),
                            provider_metadata,
                        })
                    } else {
                        GenerateContent::Text {
                            text: text.to_string(),
                            provider_metadata,
                        }
                    });
                } else if provider_metadata.is_some()
                    && let Some(last) = content.last_mut()
                {
                    match last {
                        GenerateContent::Text {
                            provider_metadata: metadata,
                            ..
                        } => *metadata = provider_metadata,
                        GenerateContent::Reasoning(reasoning) => {
                            reasoning.provider_metadata = provider_metadata
                        }
                        GenerateContent::File(file) | GenerateContent::ReasoningFile(file) => {
                            file.provider_metadata = provider_metadata
                        }
                        GenerateContent::Custom {
                            provider_metadata: metadata,
                            ..
                        } => *metadata = provider_metadata,
                        GenerateContent::ToolApprovalRequest(request) => {
                            request.provider_metadata = provider_metadata
                        }
                        GenerateContent::ToolCall(call) => {
                            call.provider_metadata = provider_metadata
                        }
                        GenerateContent::ToolResult(result) => {
                            result.provider_metadata = provider_metadata
                        }
                        GenerateContent::Source(
                            Source::Url {
                                provider_metadata: metadata,
                                ..
                            }
                            | Source::Document {
                                provider_metadata: metadata,
                                ..
                            },
                        ) => *metadata = provider_metadata,
                    }
                }
            } else if let Some(file) = inline_file(part) {
                content.push(if part.get("thought") == Some(&Value::Bool(true)) {
                    GenerateContent::ReasoningFile(file)
                } else {
                    GenerateContent::File(file)
                });
            } else if let Some(fc) = part.get("functionCall")
                && fc.get("name").is_some_and(|name| !name.is_null())
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
                    .map(str::to_string)
                    .unwrap_or_else(aimux_provider_utils::generate_id);
                let input = fc
                    .get("args")
                    .filter(|value| !value.is_null())
                    .cloned()
                    .unwrap_or(json!({}));
                let thought_signature = part
                    .get("thoughtSignature")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string);
                let provider_metadata = thought_signature.as_deref().map(|signature| {
                    vertex_provider_metadata(json!({ "thoughtSignature": signature }))
                });
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: id,
                    tool_name: name,
                    input: input.to_string(),
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata,
                }));
                has_tool_calls = true;
            } else if let Some(tc) = part.get("toolCall") {
                let tool_type = tc.get("toolType").and_then(|v| v.as_str()).unwrap_or("");
                let id = tc
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string)
                    .unwrap_or_else(|| format!("call-{}", content.len()));
                last_server_tool_call_id = Some(id.clone());
                let input = tc.get("args").cloned().unwrap_or(json!({}));
                let thought_signature = part
                    .get("thoughtSignature")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string);
                let server_meta =
                    vertex_server_tool_metadata(&id, tool_type, thought_signature.as_deref());
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: id,
                    tool_name: format!("server:{tool_type}"),
                    input: input.to_string(),
                    provider_executed: Some(true),
                    dynamic: Some(true),
                    provider_metadata: Some(server_meta),
                }));
            } else if let Some(tr) = part.get("toolResponse") {
                let tool_type = tr.get("toolType").and_then(|v| v.as_str()).unwrap_or("");
                let id = last_server_tool_call_id
                    .take()
                    .or_else(|| {
                        tr.get("id")
                            .and_then(|v| v.as_str())
                            .map(std::string::ToString::to_string)
                    })
                    .unwrap_or_else(|| format!("call-{}", content.len()));
                let response = tr.get("response").cloned().unwrap_or(json!({}));
                let server_meta = vertex_server_tool_metadata(
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
            }
        }
    }

    // Sources are appended after the parts (mirrors TS / google provider).
    let mut sources = extract_sources(candidate.grounding_metadata.as_ref(), &mut source_id);
    content.append(&mut sources);

    (content, has_tool_calls)
}

fn thought_metadata(part: &Value) -> Option<ProviderMetadata> {
    part.get("thoughtSignature")
        .and_then(Value::as_str)
        .filter(|signature| !signature.is_empty())
        .map(|signature| vertex_provider_metadata(json!({ "thoughtSignature": signature })))
}

fn inline_file(part: &Value) -> Option<GeneratedFile> {
    let inline = part.get("inlineData")?;
    Some(GeneratedFile {
        media_type: inline.get("mimeType")?.as_str()?.to_string(),
        data: GeneratedFileData::Data {
            data: FileBytes::Base64(inline.get("data")?.as_str()?.to_string()),
        },
        provider_metadata: thought_metadata(part),
    })
}

fn confirmed_block_reason(feedback: Option<&Value>) -> Option<String> {
    feedback?
        .get("blockReason")?
        .as_str()
        .filter(|reason| {
            !matches!(
                *reason,
                "" | "BLOCK_REASON_UNSPECIFIED" | "BLOCKED_REASON_UNSPECIFIED"
            )
        })
        .map(str::to_string)
}

fn vertex_usage(raw: Option<&Value>) -> Usage {
    use aimux_core::types::{InputTokenUsage, OutputTokenUsage};
    let Some(raw) = raw.filter(|value| !value.is_null()) else {
        return Usage::default();
    };
    let count = |key: &str| raw.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
    let input = count("promptTokenCount") + count("toolUsePromptTokenCount");
    let cached = count("cachedContentTokenCount");
    let text = count("candidatesTokenCount");
    let reasoning = count("thoughtsTokenCount");
    Usage {
        input_tokens: InputTokenUsage {
            total: Some(input),
            no_cache: Some(input.saturating_sub(cached)),
            cache_read: Some(cached),
            ..Default::default()
        },
        output_tokens: OutputTokenUsage {
            total: Some(text + reasoning),
            text: Some(text),
            reasoning: Some(reasoning),
        },
        raw: raw.as_object().cloned(),
    }
}

fn vertex_call_options(options: &CallOptions) -> Result<CallOptions, AiMuxError> {
    let mut options = options.clone();
    // The shared validator reads the Google key; validate the effective Vertex options.
    if let Some(effective) = Namespace::Vertex
        .read(options.provider_options.as_ref())
        .cloned()
    {
        options
            .provider_options
            .as_mut()
            .unwrap()
            .insert(GOOGLE.into(), effective);
    }
    validate_call_options_for_namespace(&options, Namespace::Vertex)?;
    if let Some(provider_options) = Namespace::Vertex.read(options.provider_options.as_ref()) {
        let mut headers = options.headers.clone().unwrap_or_default();
        for (option, header) in [
            ("sharedRequestType", "X-Vertex-AI-LLM-Shared-Request-Type"),
            ("requestType", "X-Vertex-AI-LLM-Request-Type"),
        ] {
            if let Some(value) = provider_options.get(option).and_then(Value::as_str) {
                headers.retain(|name, _| !name.eq_ignore_ascii_case(header));
                headers.insert(header.to_string(), value.to_string());
            }
        }
        options.headers = Some(headers);
    }
    Ok(options)
}

fn vertex_request_body(
    model_id: &str,
    options: &CallOptions,
    streaming: bool,
) -> Result<(Value, Vec<Warning>), AiMuxError> {
    let mut body = build_vertex_request_body(model_id, options)?;
    let mut warnings =
        prepare_all_tools(&options.tools, options.tool_choice.as_ref(), model_id).warnings;
    body.as_object_mut().unwrap().remove("serviceTier");
    if Namespace::Vertex
        .read(options.provider_options.as_ref())
        .and_then(|options| options.get("serviceTier"))
        .is_some_and(|value| !value.is_null())
    {
        warnings.push(Warning::Other { message: "'serviceTier' is a Gemini API option and is not supported on Vertex AI. Use 'sharedRequestType' (and optionally 'requestType') instead. See https://docs.cloud.google.com/vertex-ai/generative-ai/docs/priority-paygo".to_string() });
    }
    if let Some(contents) = body.get_mut("contents").and_then(Value::as_array_mut) {
        for content in contents {
            if let Some(parts) = content.get_mut("parts").and_then(Value::as_array_mut) {
                for part in parts {
                    for key in ["functionCall", "functionResponse"] {
                        if let Some(call) = part.get_mut(key).and_then(Value::as_object_mut) {
                            call.remove("id");
                        }
                    }
                }
            }
        }
    }
    if streaming
        && Namespace::Vertex
            .read(options.provider_options.as_ref())
            .and_then(|options| options.get("streamFunctionCallArguments"))
            == Some(&Value::Bool(true))
    {
        let config = body
            .as_object_mut()
            .unwrap()
            .entry("toolConfig")
            .or_insert_with(|| json!({}));
        let function_config = config
            .as_object_mut()
            .unwrap()
            .entry("functionCallingConfig")
            .or_insert_with(|| json!({}));
        function_config["streamFunctionCallArguments"] = json!(true);
    }
    Ok((body, warnings))
}
