//! `google-language-model.ts`: the Gemini language model, its request body
//! (`getArgs`) and the response parsing, used by both the Gemini API and
//! Vertex AI providers.

use std::borrow::Cow;
use std::collections::HashSet;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, ToolPart, ToolResultContent, ToolResultOutput,
    UserPart,
};
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::{
    GenerateContent, GenerateResult, LanguageModelFile, ReasoningOutput, Source, StreamResult,
};
use aimux_core::shared::{FileBytes, FileData, GeneratedFileData};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{RawToolCall, Tool, ToolResult};
use aimux_core::types::{
    FinishReason, FinishReasonUnified, ProviderMetadata, ResponseMetadata, Usage, Warning,
};
use aimux_provider_utils::{Transformer, pipe_through};

use super::api_types::{Candidate, GenerateContentResponse, GoogleStreamEvent, StreamChunk};
use super::convert_to_messages::convert_to_google_messages_for_namespace;
use super::convert_usage::convert_usage;
use super::json_accumulator::{GoogleJsonAccumulator, PartialArg};
use super::map_finish_reason::parse_finish_reason;
use super::model_capabilities::{get_google_model_capabilities, matches_prefix_boundary};
use super::options::{GOOGLE, Namespace};
use super::prepare_tools::prepare_all_tools;
use super::prompt::GooglePrompt;
use super::sanitize_response_json_schema::sanitize_response_json_schema;
use crate::shared::EndpointConfig;
use crate::stream::{Controller, Emit, FINISH_ERROR, FINISH_OTHER, StreamContext};

/// A Google Gemini language model (the AI SDK's `GoogleLanguageModel`), used
/// by both the Gemini API and Vertex AI providers.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct GoogleModel {
    model_id: String,
    config: EndpointConfig,
    namespace: Namespace,
    generate_id: std::sync::Arc<dyn Fn() -> String + Send + Sync>,
}

impl GoogleModel {
    /// A model reading and writing the `namespace` provider options and
    /// metadata; [`Namespace::Vertex`] also turns on the Vertex AI request
    /// differences (`isVertexProvider` upstream).
    pub(crate) fn from_config(
        model_id: String,
        config: EndpointConfig,
        namespace: Namespace,
    ) -> Self {
        Self {
            model_id,
            config,
            namespace,
            generate_id: std::sync::Arc::new(aimux_provider_utils::generate_id),
        }
    }

    pub(crate) fn with_generate_id(
        mut self,
        generate_id: Option<std::sync::Arc<dyn Fn() -> String + Send + Sync>>,
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

    /// The validated call options. Vertex AI sends `sharedRequestType` and
    /// `requestType` as headers, which win over the call's own.
    fn call_options<'a>(
        &self,
        options: &'a CallOptions,
    ) -> Result<Cow<'a, CallOptions>, AiMuxError> {
        validate_call_options_for_namespace(options, self.namespace)?;
        if self.namespace != Namespace::Vertex {
            return Ok(Cow::Borrowed(options));
        }
        let Some(provider_options) = self.namespace.read(options.provider_options.as_ref()) else {
            return Ok(Cow::Borrowed(options));
        };
        let mut options = options.clone();
        let mut headers = options.headers.take().unwrap_or_default();
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
        Ok(Cow::Owned(options))
    }

    /// The request body and its warnings. A Vertex AI stream also asks for
    /// streamed function-call arguments when `streamFunctionCallArguments` is set.
    fn request_body(
        &self,
        options: &CallOptions,
        streaming: bool,
    ) -> Result<(Value, Vec<Warning>), AiMuxError> {
        let (mut body, mut warnings) = build_request_body_with_warnings_for_namespace(
            &self.model_id,
            options,
            self.namespace,
        )?;
        for warning in &mut warnings {
            if let Warning::Other { message } = warning {
                *message = message.replace("google.generative-ai", &self.config.provider);
            }
        }
        if streaming
            && self.namespace == Namespace::Vertex
            && self
                .namespace
                .read(options.provider_options.as_ref())
                .and_then(|options| options.get("streamFunctionCallArguments"))
                == Some(&Value::Bool(true))
        {
            body.as_object_mut()
                .unwrap()
                .entry("toolConfig")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .unwrap()
                .entry("functionCallingConfig")
                .or_insert_with(|| json!({}))["streamFunctionCallArguments"] = json!(true);
        }
        Ok((body, warnings))
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

    async fn supported_urls(&self) -> SupportedUrls {
        self.config.supported_urls(&self.model_id)
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let options = &*self.call_options(options)?;
        let code_execution_tool_name = code_execution_tool_name(options.tools.as_deref());
        let (body, warnings) = self.request_body(options, false)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(body);
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

        let (content, has_tool_calls) = extract_content_from_candidate(
            &candidate,
            &code_execution_tool_name,
            self.namespace,
            &*self.generate_id,
        );

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
                .unwrap_or(FINISH_OTHER)
        };

        let usage = data
            .usage_metadata
            .as_ref()
            .map(convert_usage)
            .unwrap_or_default();

        // Provider metadata under the namespace keys (the TS
        // `wrapProviderMetadata`).
        let provider_metadata = Some(self.namespace.metadata(serde_json::json!({
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
        let options = &*self.call_options(options)?;
        let code_execution_tool_name = code_execution_tool_name(options.tools.as_deref());
        let (body, warnings) = self.request_body(options, true)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(body);
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
        let stream = pipe_through(
            futures::stream::iter(first_event).chain(sse_stream),
            GoogleLanguageModelStream {
                ctx: StreamContext {
                    generate_id: self.generate_id.clone(),
                    ..StreamContext::new(options, warnings, endpoint, &body, &response_headers)
                },
                namespace: self.namespace,
                code_execution_tool_name,
                ..GoogleLanguageModelStream::default()
            },
        );

        Ok(crate::stream::strict_guard(
            StreamResult {
                stream: Box::pin(stream),
                request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
                response: Some(aimux_core::shared::StreamResponseInfo {
                    headers: Some(response_headers),
                }),
            },
            options,
        ))
    }
}

/// The `TransformStream` of `GoogleLanguageModel.doStream`.
#[derive(Default)]
struct GoogleLanguageModelStream {
    ctx: StreamContext,
    namespace: Namespace,
    code_execution_tool_name: String,
    text_id: Option<String>,
    reasoning_id: Option<String>,
    block_counter: usize,
    final_usage: Usage,
    final_finish_reason: Option<FinishReason>,
    has_tool_calls: bool,
    /// Function calls whose arguments are still streaming (Vertex AI
    /// `partialArgs`), innermost last.
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

impl GoogleLanguageModelStream {
    fn transform_chunk(&mut self, chunk: Box<StreamChunk>, controller: &mut Controller) {
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
                    id: (self.ctx.generate_id)(),
                    url,
                    title,
                    provider_metadata: None,
                }));
            }
        }

        if let Some(parts) = candidate.content.as_ref().and_then(|c| c.parts.as_ref()) {
            for part in parts {
                // thoughtSignature → provider_metadata (upstream :778-782)
                let thought_sig_meta = thought_signature_metadata(self.namespace, part);

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
                                controller.text_end(id);
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
                                controller.reasoning_end(id);
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
                        let id = (self.ctx.generate_id)();
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
                        .unwrap_or_else(|| (self.ctx.generate_id)());
                    self.last_server_tool_call_id = Some(id.clone());
                    let args = tc.get("args").cloned().unwrap_or(json!({}));
                    let server_meta = server_tool_metadata(
                        self.namespace,
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
                        .unwrap_or_else(|| (self.ctx.generate_id)());
                    let response = tr.get("response").cloned().unwrap_or(json!({}));
                    let server_meta = server_tool_metadata(
                        self.namespace,
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
                        controller.text_end(id);
                    }
                    if let Some(id) = self.reasoning_id.take() {
                        controller.reasoning_end(id);
                    }
                    if let (Some(data), Some(mime)) = (
                        inline.get("data").and_then(|v| v.as_str()),
                        inline.get("mimeType").and_then(|v| v.as_str()),
                    ) {
                        let file = LanguageModelFile {
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
            // Function calls come after the other parts, as upstream.
            for part in parts {
                if let Some(call) = part.get("functionCall")
                    && !self.function_call(call, part, controller)
                {
                    return;
                }
            }
        }

        if let Some(reason) = candidate.finish_reason.as_deref() {
            self.final_finish_reason = Some(parse_finish_reason(reason, self.has_tool_calls));
        }
    }
}

impl GoogleLanguageModelStream {
    /// One `functionCall` part: a chunk of streamed arguments (`partialArgs`,
    /// or a name with `willContinue`), the empty chunk that ends a streamed
    /// call, or a complete call. `false` when the arguments cannot be
    /// accumulated, which ends the stream.
    fn function_call(&mut self, call: &Value, part: &Value, controller: &mut Controller) -> bool {
        let field = |name: &str| call.get(name).filter(|value| !value.is_null());
        let name = field("name").and_then(Value::as_str);
        let args = field("args");
        let partial_args = field("partialArgs");
        let will_continue = field("willContinue").and_then(Value::as_bool);
        let metadata = thought_signature_metadata(self.namespace, part);
        let id = || {
            field("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map_or_else(|| (self.ctx.generate_id)(), str::to_owned)
        };

        if partial_args.is_some() || (name.is_some() && will_continue == Some(true)) {
            if let Some(name) = name {
                let id = id();
                controller.enqueue(StreamPart::ToolInputStart {
                    id: id.clone(),
                    tool_name: name.to_owned(),
                    provider_executed: None,
                    dynamic: None,
                    title: None,
                    provider_metadata: metadata.clone(),
                });
                self.active_tool_calls.push((
                    RawToolCall {
                        tool_call_id: id,
                        tool_name: name.to_owned(),
                        input: String::new(),
                        provider_executed: None,
                        dynamic: None,
                        provider_metadata: metadata.clone(),
                    },
                    GoogleJsonAccumulator::new(),
                ));
            }
            let (Some(partial_args), Some((active, accumulator))) = (
                partial_args.and_then(Value::as_array),
                self.active_tool_calls.last_mut(),
            ) else {
                return true;
            };
            let partial_args: Vec<PartialArg> = partial_args.iter().map(partial_arg).collect();
            match accumulator.process_partial_args(&partial_args) {
                Ok(result) if !result.text_delta.is_empty() => {
                    controller.enqueue(StreamPart::ToolInputDelta {
                        id: active.tool_call_id.clone(),
                        delta: result.text_delta,
                        provider_metadata: metadata,
                    });
                }
                Ok(_) => {}
                Err(error) => {
                    controller.enqueue(StreamPart::Error { error });
                    controller.terminate();
                    return false;
                }
            }
            if will_continue != Some(true)
                && partial_args
                    .iter()
                    .all(|arg| arg.will_continue != Some(true))
            {
                self.finish_streaming_tool_call(controller);
            }
        } else if name.is_none() && args.is_none() && will_continue.is_none() {
            self.finish_streaming_tool_call(controller);
        } else if let Some(name) = name {
            let id = id();
            let input = match args {
                Some(Value::String(args)) => args.clone(),
                Some(args) => args.to_string(),
                None => "{}".to_owned(),
            };
            controller.enqueue(StreamPart::ToolInputStart {
                id: id.clone(),
                tool_name: name.to_owned(),
                provider_executed: None,
                dynamic: None,
                title: None,
                provider_metadata: metadata.clone(),
            });
            if args.is_some() {
                controller.enqueue(StreamPart::ToolInputDelta {
                    id: id.clone(),
                    delta: input.clone(),
                    provider_metadata: metadata.clone(),
                });
            }
            controller.enqueue(StreamPart::ToolInputEnd {
                id: id.clone(),
                provider_metadata: metadata.clone(),
            });
            controller.enqueue(StreamPart::ToolCall(RawToolCall {
                tool_call_id: id,
                tool_name: name.to_owned(),
                input,
                provider_executed: None,
                dynamic: None,
                provider_metadata: metadata,
            }));
            self.has_tool_calls = true;
        }
        true
    }

    /// Close the innermost streamed function call with its final arguments.
    fn finish_streaming_tool_call(&mut self, controller: &mut Controller) {
        let Some((mut call, accumulator)) = self.active_tool_calls.pop() else {
            return;
        };
        let result = accumulator.finalize();
        if !result.closing_delta.is_empty() {
            controller.enqueue(StreamPart::ToolInputDelta {
                id: call.tool_call_id.clone(),
                delta: result.closing_delta,
                provider_metadata: call.provider_metadata.clone(),
            });
        }
        controller.enqueue(StreamPart::ToolInputEnd {
            id: call.tool_call_id.clone(),
            provider_metadata: call.provider_metadata.clone(),
        });
        call.input = result.final_json;
        controller.enqueue(StreamPart::ToolCall(call));
        self.has_tool_calls = true;
    }
}

impl Transformer for GoogleLanguageModelStream {
    type Input = Result<Value, AiMuxError>;
    type Output = StreamPart;

    fn start(&mut self, controller: &mut Controller) {
        self.ctx.start(controller);
    }

    fn transform(&mut self, event: Result<Value, AiMuxError>, controller: &mut Controller) {
        let event = match event {
            Ok(raw_value) => {
                self.ctx.raw(&raw_value, controller);
                serde_json::from_value::<GoogleStreamEvent>(raw_value).map_err(AiMuxError::from)
            }
            Err(error) => Err(error),
        };

        match event {
            Ok(GoogleStreamEvent::Chunk(chunk)) => self.transform_chunk(chunk, controller),
            Ok(GoogleStreamEvent::Error(error)) => {
                controller.enqueue(StreamPart::Error {
                    error: super::error::google_stream_error(
                        &error.error,
                        &self.ctx.url,
                        self.ctx.request_body.clone(),
                        self.ctx.response_headers.clone(),
                    ),
                });
                // The rest of the body is not read: the stream finishes now.
                self.stream_errored = true;
                self.finish(controller);
                controller.terminate();
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

    fn flush(mut self, controller: &mut Controller) {
        if self.ctx.fail_without_terminal(
            self.final_finish_reason.is_some() || self.stream_errored,
            "a candidate finishReason",
            controller,
        ) {
            return;
        }
        self.finish(controller);
    }
}

impl GoogleLanguageModelStream {
    /// Close the open segments and emit `Finish`: at the end of the body, or
    /// right after an error event.
    fn finish(&mut self, controller: &mut Controller) {
        // Close any remaining open text/reasoning segment.
        if let Some(id) = self.text_id.take() {
            controller.text_end(id);
        }
        if let Some(id) = self.reasoning_id.take() {
            controller.reasoning_end(id);
        }

        let provider_metadata = Some(self.namespace.metadata(serde_json::json!({
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
                FINISH_ERROR
            } else {
                self.final_finish_reason.take().unwrap_or(FINISH_OTHER)
            },
            usage: if self.stream_errored {
                Usage::default()
            } else {
                std::mem::take(&mut self.final_usage)
            },
            provider_metadata,
        });
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// `{ thoughtSignature }` under the namespace keys, when the part carries a
/// non-empty signature.
fn thought_signature_metadata(namespace: Namespace, part: &Value) -> Option<ProviderMetadata> {
    part.get("thoughtSignature")
        .and_then(Value::as_str)
        .filter(|signature| !signature.is_empty())
        .map(|signature| namespace.metadata(json!({ "thoughtSignature": signature })))
}

/// One Vertex AI `partialArgs` entry.
fn partial_arg(arg: &Value) -> PartialArg {
    PartialArg {
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
    }
}

fn server_tool_metadata(
    namespace: Namespace,
    tool_call_id: &str,
    server_tool_type: &str,
    thought_signature: Option<&str>,
) -> ProviderMetadata {
    let mut payload = json!({
        "serverToolCallId": tool_call_id,
        "serverToolType": server_tool_type,
    });
    if let Some(signature) = thought_signature.filter(|signature| !signature.is_empty()) {
        payload["thoughtSignature"] = json!(signature);
    }
    namespace.metadata(payload)
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
    namespace: Namespace,
    generate_id: &dyn Fn() -> String,
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
            let thought_sig_meta = thought_signature_metadata(namespace, part);

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
                    let file = LanguageModelFile {
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
                    namespace,
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
                    namespace,
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
        | GenerateContent::File(LanguageModelFile {
            provider_metadata, ..
        })
        | GenerateContent::Source(Source::Url {
            provider_metadata, ..
        })
        | GenerateContent::Source(Source::Document {
            provider_metadata, ..
        })
        | GenerateContent::ReasoningFile(LanguageModelFile {
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

// ── getArgs (request body) ───────────────────────────────────────────────────

/// Build a Vertex Gemini request body using Vertex's provider-metadata
/// namespaces when replaying response parts.
///
/// # Errors
/// Returns an error when `structuredOutputs` is not a boolean.
pub(crate) fn build_vertex_request_body(
    model_id: &str,
    options: &CallOptions,
) -> Result<Value, AiMuxError> {
    build_request_body_with_warnings_for_namespace(model_id, options, Namespace::Vertex)
        .map(|(body, _)| body)
}

/// Build the Gemini `generateContent` request body from `CallOptions` **and**
/// collect the tool warnings (e.g. unsupported provider-defined tools, mixed
/// function+provider tools on pre-Gemini-3 models).
///
/// Mirrors `getArgs` in `google-language-model.ts`: the sampling settings, the
/// response format, and the provider options the SDK maps (`thinkingConfig`,
/// `responseModalities`, `audioTimestamp`, `mediaResolution`, `imageConfig`
/// into `generationConfig`; `safetySettings`, `cachedContent`, `labels`,
/// `serviceTier` and `retrievalConfig` next to it).
///
/// # Errors
/// Returns an error when `structuredOutputs` is not a boolean.
pub fn build_request_body_with_warnings(
    model_id: &str,
    options: &CallOptions,
) -> Result<(Value, Vec<Warning>), AiMuxError> {
    build_request_body_with_warnings_for_namespace(model_id, options, Namespace::Google)
}

pub(crate) fn build_request_body_with_warnings_for_namespace(
    model_id: &str,
    options: &CallOptions,
    namespace: Namespace,
) -> Result<(Value, Vec<Warning>), AiMuxError> {
    let provider_options = options.provider_options.as_ref().and_then(|options| {
        namespace
            .read_keys()
            .iter()
            .find_map(|name| options.get(*name).map(|value| (*name, value)))
    });
    let structured_outputs =
        match provider_options.and_then(|(_, options)| options.get("structuredOutputs")) {
            None => true,
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                return Err(AiMuxError::invalid_argument(format!(
                    "invalid {} provider options",
                    provider_options.unwrap().0
                )));
            }
        };
    let GooglePrompt {
        mut system_instruction,
        mut contents,
    } = convert_to_google_messages_for_namespace(
        &options.prompt,
        namespace,
        get_google_model_capabilities(model_id).uses_gemini_3_features,
    );
    let mut warnings = Vec::new();
    let provider_options = namespace.read_in(options.provider_options.as_ref());
    let option = |name: &str| {
        provider_options
            .and_then(|o| o.get(name))
            .filter(|v| !v.is_null())
    };
    let is_vertex = namespace == Namespace::Vertex;
    if !is_vertex
        && options.tools.as_ref().is_some_and(|tools| {
            tools.iter().any(
                |tool| matches!(tool, Tool::Provider(tool) if tool.id == "google.vertex_rag_store"),
            )
        })
    {
        warnings.push(Warning::Other { message: "The 'vertex_rag_store' tool is only supported with the Google Vertex provider and might not be supported or could behave unexpectedly with the current Google provider (google.generative-ai).".into() });
    }

    if !is_vertex && option("streamFunctionCallArguments") == Some(&Value::Bool(true)) {
        warnings.push(Warning::Other { message: "'streamFunctionCallArguments' is only supported on the Vertex AI API and will be ignored with the current Google provider (google.generative-ai). See https://docs.cloud.google.com/vertex-ai/generative-ai/docs/multimodal/function-calling#streaming-fc".into() });
    }
    if is_vertex && option("serviceTier").is_some() {
        warnings.push(Warning::Other { message: "'serviceTier' is a Gemini API option and is not supported on Vertex AI. Use 'sharedRequestType' (and optionally 'requestType') instead. See https://docs.cloud.google.com/vertex-ai/generative-ai/docs/priority-paygo".into() });
    }
    if !is_vertex && (option("sharedRequestType").is_some() || option("requestType").is_some()) {
        warnings.push(Warning::Other { message: "'sharedRequestType' and 'requestType' are Vertex AI options and are ignored with the current Google provider (google.generative-ai).".into() });
    }
    let option_warning_count = warnings.len();
    let mut prompt_warnings = Vec::new();
    let mut thinking_warnings = Vec::new();
    let omit_penalties =
        !is_vertex && matches_prefix_boundary(&model_id.to_lowercase(), "gemini-2.5");
    if model_id.to_lowercase().starts_with("gemma-")
        && let Some(system) = system_instruction.take()
        && let Some(first) = contents.first_mut()
        && first["role"] == "user"
        && let Some(parts) = first["parts"].as_array_mut()
    {
        let texts = system["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        parts.insert(0, json!({ "text": format!("{texts}\n\n") }));
    }
    if get_google_model_capabilities(model_id).uses_gemini_3_features {
        let mut missing = Vec::new();
        for content in &mut contents {
            if content["role"] != "model" {
                continue;
            }
            let mut has_signed_call = false;
            for part in content["parts"].as_array_mut().unwrap() {
                let standard = part.get("functionCall").is_some();
                let server = part.get("toolCall").is_some();
                if !standard && !server {
                    continue;
                }
                if part.get("thoughtSignature").is_some() {
                    if standard {
                        has_signed_call = true;
                    }
                } else if server || !has_signed_call {
                    part["thoughtSignature"] = json!("skip_thought_signature_validator");
                    missing.push(
                        part.get("functionCall")
                            .or_else(|| part.get("toolCall"))
                            .unwrap()
                            .get("name")
                            .or_else(|| part["toolCall"].get("toolType"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    );
                }
            }
        }
        if !missing.is_empty() {
            let mut names = Vec::new();
            for name in &missing {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            prompt_warnings.push(Warning::Other { message: format!(
                "Replayed {} `functionCall` part(s) for a Gemini 3 model without a `thoughtSignature` (tools: {}). Injected the documented `skip_thought_signature_validator` sentinel to keep the request from failing with HTTP 400. The likely cause is application code that drops `providerOptions.google.thoughtSignature` when persisting or serializing assistant tool-call messages. See https://ai.google.dev/gemini-api/docs/thought-signatures.",
                missing.len(), names.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")) });
        }
    }

    let mut generation_config = Map::new();

    if let Some(max_tokens) = options.max_output_tokens {
        generation_config.insert("maxOutputTokens".to_string(), json!(max_tokens));
    }
    if let Some(temp) = options.temperature {
        generation_config.insert("temperature".to_string(), json!(temp));
    }
    if let Some(top_p) = options.top_p {
        generation_config.insert("topP".to_string(), json!(top_p));
    }
    if let Some(top_k) = options.top_k {
        generation_config.insert("topK".to_string(), json!(top_k));
    }
    for (name, value) in [
        ("frequencyPenalty", options.frequency_penalty),
        ("presencePenalty", options.presence_penalty),
    ] {
        if let Some(value) = value {
            if omit_penalties {
                warnings.push(Warning::Unsupported {
                    feature: name.into(),
                    details: None,
                });
            } else {
                generation_config.insert(name.into(), json!(value));
            }
        }
    }
    if let Some(stop) = &options.stop_sequences {
        generation_config.insert("stopSequences".to_string(), json!(stop));
    }
    if let Some(seed) = options.seed {
        generation_config.insert("seed".to_string(), json!(seed));
    }

    // JSON output uses responseMimeType and the sanitized responseJsonSchema.
    if let Some(rf) = &options.response_format
        && let ResponseFormat::Json { schema, .. } = rf
    {
        generation_config.insert("responseMimeType".to_string(), json!("application/json"));
        if let Some(schema) = schema
            && structured_outputs
        {
            generation_config.insert(
                "responseJsonSchema".into(),
                sanitize_response_json_schema(schema),
            );
        }
    }

    // Provider options (`providerOptions.google`, or `googleVertex` for
    // Vertex): the generation-config members, then the top-level ones.
    for name in ["responseModalities", "audioTimestamp", "mediaResolution"] {
        if let Some(value) = option(name)
            && (name != "audioTimestamp" || value == &Value::Bool(true))
        {
            generation_config.insert(name.into(), value.clone());
        }
    }
    let mut thinking = resolve_thinking(options.reasoning, model_id, &mut thinking_warnings);
    if let Some(explicit) = option("thinkingConfig").and_then(Value::as_object) {
        thinking
            .get_or_insert_with(Map::new)
            .extend(explicit.clone());
    }
    if let Some(thinking) = thinking {
        generation_config.insert("thinkingConfig".into(), Value::Object(thinking));
    }
    if let Some(image) = option("imageConfig") {
        let mut image = image.clone();
        if !is_vertex && let Some(config) = image.as_object_mut() {
            let mut dropped = Vec::new();
            for name in ["personGeneration", "prominentPeople", "imageOutputOptions"] {
                if config.remove(name).is_some() {
                    dropped.push(format!("'imageConfig.{name}'"));
                }
            }
            if !dropped.is_empty() {
                warnings.insert(option_warning_count, Warning::Other {
                    message: format!(
                        "{} {} ignored with the current Google provider (google.generative-ai).",
                        dropped.join(", "),
                        if dropped.len() == 1 {
                            "is a Vertex AI option and is"
                        } else {
                            "are Vertex AI options and are"
                        }
                    ),
                });
            }
        }
        generation_config.insert("imageConfig".into(), image);
    }
    for (name, keys) in [
        (
            "thinkingConfig",
            &["thinkingBudget", "includeThoughts", "thinkingLevel"][..],
        ),
        (
            "imageConfig",
            &[
                "aspectRatio",
                "imageSize",
                "personGeneration",
                "prominentPeople",
                "imageOutputOptions",
            ][..],
        ),
    ] {
        if let Some(Value::Object(config)) = generation_config.get_mut(name) {
            config.retain(|key, _| keys.contains(&key.as_str()));
            if let Some(Value::Object(output)) = config.get_mut("imageOutputOptions") {
                output.retain(|key, _| ["mimeType", "compressionQuality"].contains(&key.as_str()));
            }
        }
    }
    let mut body = Map::new();
    body.insert("contents".to_string(), Value::Array(contents));
    if let Some(sys) = system_instruction {
        body.insert("systemInstruction".to_string(), sys);
    }
    // Always present, even when empty (`generationConfig: {}`), like the SDK.
    body.insert(
        "generationConfig".to_string(),
        Value::Object(generation_config),
    );
    for name in ["safetySettings", "cachedContent", "labels", "serviceTier"] {
        if let Some(value) = option(name)
            && !(is_vertex && name == "serviceTier")
        {
            body.insert(name.to_string(), value.clone());
        }
    }
    if option("safetySettings").is_none()
        && let Some(threshold) = option("threshold")
    {
        body.insert(
            "safetySettings".into(),
            json!(
                [
                    "HARM_CATEGORY_HATE_SPEECH",
                    "HARM_CATEGORY_DANGEROUS_CONTENT",
                    "HARM_CATEGORY_HARASSMENT",
                    "HARM_CATEGORY_SEXUALLY_EXPLICIT"
                ]
                .map(|category| json!({ "category": category, "threshold": threshold }))
            ),
        );
    }
    if let Some(Value::Array(settings)) = body.get_mut("safetySettings") {
        for setting in settings {
            if let Some(setting) = setting.as_object_mut() {
                setting.retain(|key, _| ["category", "threshold"].contains(&key.as_str()));
            }
        }
    }

    let prepared = prepare_all_tools(&options.tools, options.tool_choice.as_ref(), model_id);
    if let Some(tools) = prepared.tools {
        body.insert("tools".to_string(), Value::Array(tools));
    }
    let mut tool_config = prepared.tool_config;
    if is_vertex && let Some(Value::Object(config)) = tool_config.as_mut() {
        config.remove("includeServerSideToolInvocations");
    }
    if let Some(retrieval) = option("retrievalConfig") {
        let mut config = match tool_config.take() {
            Some(Value::Object(config)) => config,
            _ => Map::new(),
        };
        let mut retrieval = retrieval.clone();
        if let Some(retrieval) = retrieval.as_object_mut() {
            retrieval.retain(|key, _| key == "latLng");
            if let Some(Value::Object(coordinates)) = retrieval.get_mut("latLng") {
                coordinates.retain(|key, _| ["latitude", "longitude"].contains(&key.as_str()));
            }
        }
        config.insert("retrievalConfig".to_string(), retrieval);
        tool_config = Some(Value::Object(config));
    }
    if let Some(tc) = tool_config {
        body.insert("toolConfig".to_string(), tc);
    }

    // Model id is *not* part of the body for Gemini — it's in the URL path
    // (`models/{model}:generateContent`). We don't emit it here, matching
    // the TS SDK. (Some callers include it; the API ignores extra fields.)
    let _ = model_id;

    warnings.extend(prompt_warnings);
    warnings.extend(thinking_warnings);
    warnings.extend(prepared.warnings);
    Ok((Value::Object(body), warnings))
}

/// Validate prompt constraints and provider options before sending a request.
pub(crate) fn validate_call_options(
    options: &CallOptions,
) -> Result<(), aimux_core::error::AiMuxError> {
    validate_call_options_for_namespace(options, Namespace::Google)
}

pub(crate) fn validate_call_options_for_namespace(
    options: &CallOptions,
    namespace: Namespace,
) -> Result<(), aimux_core::error::AiMuxError> {
    use aimux_core::error::AiMuxError;
    let mut initial = true;
    for message in &options.prompt {
        if matches!(message, LanguageModelMessage::System { .. }) {
            if !initial {
                return Err(AiMuxError::UnsupportedFunctionality(
                    "system messages are only supported at the beginning of the conversation"
                        .into(),
                ));
            }
        } else {
            initial = false;
        }
        match message {
            LanguageModelMessage::User { content, .. } => {
                for part in content {
                    if let UserPart::File(file) = part {
                        if namespace == Namespace::Vertex
                            && matches!(file.data, FileData::Reference { .. })
                        {
                            return Err(AiMuxError::UnsupportedFunctionality(
                                "file parts with provider references".into(),
                            ));
                        }
                        validate_file_part(file, false)?;
                    }
                }
            }
            LanguageModelMessage::Assistant { content, .. } => {
                for part in content {
                    match part {
                        AssistantPart::File(file) => {
                            if namespace == Namespace::Vertex
                                && matches!(file.data, FileData::Reference { .. })
                            {
                                return Err(AiMuxError::UnsupportedFunctionality(
                                    "file parts with provider references".into(),
                                ));
                            }
                            validate_file_part(file, true)?;
                        }
                        AssistantPart::ReasoningFile(file)
                            if matches!(
                                file.data,
                                aimux_core::shared::GeneratedFileData::Url { .. }
                            ) =>
                        {
                            return Err(AiMuxError::UnsupportedFunctionality(
                                "File data URLs in assistant messages are not supported".into(),
                            ));
                        }
                        _ => {}
                    }
                }
            }
            LanguageModelMessage::Tool { content, .. } => {
                for part in content {
                    if let ToolPart::ToolResult(part) = part
                        && let ToolResultOutput::Content { value } = &part.output
                    {
                        for part in value {
                            if let ToolResultContent::File(file) = part
                                && matches!(file.data, FileData::Data { .. })
                            {
                                aimux_provider_utils::resolve_full_media_type(file)?;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let Some(values) = namespace.read_in(options.provider_options.as_ref()) else {
        return Ok(());
    };
    super::language_model_options::validate_provider_options(values)
}

fn validate_file_part(
    file: &FilePart,
    assistant: bool,
) -> Result<(), aimux_core::error::AiMuxError> {
    use aimux_core::error::AiMuxError;
    if !matches!(file.data, FileData::Text { .. }) {
        aimux_provider_utils::resolve_full_media_type(file)?;
    }
    if assistant && matches!(file.data, FileData::Url { .. }) {
        return Err(AiMuxError::UnsupportedFunctionality(
            "File data URLs in assistant messages are not supported".into(),
        ));
    }
    if let FileData::Reference { reference } = &file.data {
        aimux_provider_utils::resolve_provider_reference(reference, GOOGLE)?;
    }
    Ok(())
}

fn resolve_thinking(
    reasoning: Option<aimux_core::types::ReasoningEffort>,
    model_id: &str,
    warnings: &mut Vec<Warning>,
) -> Option<Map<String, Value>> {
    use aimux_core::types::ReasoningEffort as R;
    let reasoning = reasoning.filter(|effort| *effort != R::ProviderDefault)?;
    let lower = model_id.to_lowercase();
    if get_google_model_capabilities(model_id).uses_gemini_3_features
        && !model_id.contains("gemini-3-pro-image")
    {
        let name = lower.rsplit('/').next().unwrap_or(&lower);
        let minimum = if name == "gemini-flash-latest"
            || name
                .strip_prefix("gemini-")
                .and_then(|s| s.split_once("-flash"))
                .is_some_and(|(version, tail)| {
                    let Some((major, minor)) = version.split_once('.') else {
                        return false;
                    };
                    let major = major.parse::<u32>().unwrap_or(0);
                    let minor = minor.parse::<u32>().unwrap_or(0);
                    (tail.is_empty()
                        || (tail.starts_with('-')
                            && tail != "-lite"
                            && !tail.starts_with("-lite-")))
                        && (major > 3 || major == 3 && minor >= 7)
                }) {
            "low"
        } else {
            "minimal"
        };
        if reasoning == R::None {
            // It's not possible to fully disable thinking with Gemini 3.
            return Some(Map::from_iter([("thinkingLevel".into(), json!(minimum))]));
        }
        let level = aimux_provider_utils::map_reasoning_to_provider_effort(
            reasoning,
            &[
                (R::Minimal, minimum),
                (R::Low, "low"),
                (R::Medium, "medium"),
                (R::High, "high"),
                (R::Xhigh, "high"),
            ],
            warnings,
        )?;
        return Some(Map::from_iter([("thinkingLevel".into(), json!(level))]));
    }
    if reasoning == R::None {
        return Some(Map::from_iter([("thinkingBudget".into(), json!(0))]));
    }
    let max = if lower.contains("2.5-pro") || lower.contains("gemini-3-pro-image") {
        32768
    } else {
        24576
    };
    let budget = aimux_provider_utils::map_reasoning_to_provider_budget(
        reasoning,
        65536,
        max,
        0,
        aimux_provider_utils::DEFAULT_REASONING_BUDGET_PERCENTAGES,
        warnings,
    )?;
    Some(Map::from_iter([("thinkingBudget".into(), json!(budget))]))
}

/// Resolve the caller-facing name for Google's code execution provider tool:
/// `toolNameMapping.toCustomToolName('code_execution')` with the
/// `providerToolNames` of google-language-model.ts. Gemini always uses
/// `code_execution` on the response wire, while callers may rename the
/// provider tool for a particular request.
pub(crate) fn code_execution_tool_name(tools: Option<&[Tool]>) -> String {
    aimux_provider_utils::create_tool_name_mapping(
        tools,
        &[("google.code_execution", "code_execution")],
    )
    .to_custom_tool_name("code_execution")
    .to_owned()
}

// ── source extraction ────────────────────────────────────────────────────────

/// Extract `GenerateContent::Source` items from `groundingMetadata.groundingChunks`,
/// mirroring the TS `extractSources`.
///
/// - `web` → url source (`uri`, `title`)
/// - `image` → url source (`sourceUri`, `title`)
/// - `retrievedContext` with http(s) `uri` → url source
/// - `retrievedContext` with non-http `uri` (e.g. `gs://`) → document source
///   (title defaults to "Unknown Document"; `url` is `None`)
/// - `retrievedContext` with `fileSearchStore` (no `uri`) → document source
/// - `maps` → url source (`uri`, `title`)
pub fn extract_sources(
    grounding_metadata: Option<&Value>,
    id_counter: &mut usize,
) -> Vec<GenerateContent> {
    let mut sources = Vec::new();
    let Some(gm) = grounding_metadata else {
        return sources;
    };
    let Some(chunks) = gm.get("groundingChunks").and_then(|c| c.as_array()) else {
        return sources;
    };

    let next_id = |counter: &mut usize| -> String {
        let id = format!("{counter}");
        *counter += 1;
        id
    };

    for chunk in chunks {
        if let Some(web) = chunk.get("web").filter(|value| !value.is_null()) {
            sources.push(GenerateContent::Source(Source::Url {
                id: next_id(id_counter),
                url: web
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                title: web
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string),
                provider_metadata: None,
            }));
        } else if let Some(image) = chunk.get("image").filter(|value| !value.is_null()) {
            sources.push(GenerateContent::Source(Source::Url {
                id: next_id(id_counter),
                url: image
                    .get("sourceUri")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                title: image
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string),
                provider_metadata: None,
            }));
        } else if let Some(rc) = chunk
            .get("retrievedContext")
            .filter(|value| !value.is_null())
        {
            let uri = rc
                .get("uri")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            let file_search_store = rc
                .get("fileSearchStore")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            let title = rc.get("title").and_then(|v| v.as_str());
            if let Some(uri) = uri {
                if uri.starts_with("http://") || uri.starts_with("https://") {
                    sources.push(GenerateContent::Source(Source::Url {
                        id: next_id(id_counter),
                        url: uri.to_string(),
                        title: title.map(std::string::ToString::to_string),
                        provider_metadata: None,
                    }));
                } else {
                    // Document with a file path (gs://, etc.).
                    let media_type = if uri.ends_with(".pdf") {
                        "application/pdf"
                    } else if uri.ends_with(".txt") {
                        "text/plain"
                    } else if uri.ends_with(".docx") {
                        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                    } else if uri.ends_with(".doc") {
                        "application/msword"
                    } else if uri.ends_with(".md") || uri.ends_with(".markdown") {
                        "text/markdown"
                    } else {
                        "application/octet-stream"
                    };
                    sources.push(GenerateContent::Source(Source::Document {
                        id: next_id(id_counter),
                        media_type: media_type.to_string(),
                        title: title.unwrap_or("Unknown Document").to_string(),
                        filename: uri.rsplit('/').next().map(str::to_string),
                        provider_metadata: None,
                    }));
                }
            } else if let Some(file_search_store) = file_search_store {
                // New File Search format (no uri, has fileSearchStore).
                sources.push(GenerateContent::Source(Source::Document {
                    id: next_id(id_counter),
                    media_type: "application/octet-stream".to_string(),
                    title: title.unwrap_or("Unknown Document").to_string(),
                    filename: file_search_store.rsplit('/').next().map(str::to_string),
                    provider_metadata: None,
                }));
            }
            // else: no uri and no fileSearchStore → no source.
        } else if let Some(maps) = chunk.get("maps").filter(|value| !value.is_null())
            && let Some(uri) = maps
                .get("uri")
                .and_then(|v| v.as_str())
                .filter(|uri| !uri.is_empty())
        {
            sources.push(GenerateContent::Source(Source::Url {
                id: next_id(id_counter),
                url: uri.to_string(),
                title: maps
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string),
                provider_metadata: None,
            }));
        }
    }

    sources
}
