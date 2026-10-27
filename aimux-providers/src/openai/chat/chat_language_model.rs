//! The Chat Completions language model (openai-chat-language-model.ts):
//! request building, generation and streaming.
//!
//! The HTTP request/response handling lives in the crate-internal functions
//! `execute_generate` and `execute_stream`, which take a prepared request (URL,
//! headers, transport) and a model id. They call the `aimux-provider-utils` API helpers —
//! **no `reqwest` types cross this boundary**. This lets other providers that
//! speak the OpenAI chat-completions wire format (notably Azure OpenAI) reuse
//! the conversion + streaming logic while supplying their own URL and auth.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, Source, StreamResult};
use aimux_core::shared::{SharedProviderOptions, provider_namespace};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, RawToolCall, Tool};
use aimux_core::types::{FinishReason, ReasoningEffort, ResponseMetadata, Usage, Warning};
use aimux_provider_utils::{HttpRequest, Transformer, pipe_through};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};

use crate::openai::chat::chat_api::{ChatCompletionResponse, StreamChunk, UsageResponse};
use crate::openai::chat::chat_language_model_options::{
    openai_option, parse_chat_provider_options,
};
use crate::openai::chat::chat_prepare_tools::prepare_tools;
use crate::openai::chat::convert_chat_usage::convert_usage;
use crate::openai::chat::convert_to_chat_messages::convert_prompt_to_openai_messages_with_mode_fallible;
use crate::openai::chat::map_finish_reason::parse_finish_reason;
use crate::openai::config::{OpenAIModelConfig, TransformRequestBody};
use crate::openai::language_model_capabilities::{
    ModelCapabilities, SystemMessageMode, get_model_capabilities,
};
use crate::openai::normalize_json_schema::normalize_json_schema;
use crate::stream::{
    ChatToolCalls, Controller, Emit, FINISH_ERROR, FINISH_OTHER, StreamContext, ToolCallDelta,
};

/// An OpenAI chat-completions language model.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct OpenAIModel {
    model_id: String,
    config: OpenAIModelConfig,
}

impl OpenAIModel {
    pub(crate) fn from_config(model_id: String, config: OpenAIModelConfig) -> Self {
        Self { model_id, config }
    }
}

/// `choices[].logprobs.content` as the AI SDK's chat schemas parse it
/// (openai-chat-api.ts:71-87, chunks :159-175): an entry keeps `token`,
/// `logprob` and `top_logprobs[].{token, logprob}`, so `bytes` is dropped.
/// A `null` or missing `content` sets no metadata
/// (openai-chat-language-model.ts:459-461, :613-615).
fn logprobs_content(logprobs: &Value) -> Option<Value> {
    fn token(entry: &Value) -> serde_json::Map<String, Value> {
        ["token", "logprob"]
            .into_iter()
            .filter_map(|key| entry.get(key).map(|value| (key.to_owned(), value.clone())))
            .collect()
    }
    let content = logprobs.get("content")?.as_array()?;
    Some(
        content
            .iter()
            .map(|entry| {
                let mut out = token(entry);
                if let Some(top) = entry.get("top_logprobs").and_then(Value::as_array) {
                    out.insert(
                        "top_logprobs".to_owned(),
                        top.iter().map(|top| Value::Object(token(top))).collect(),
                    );
                }
                Value::Object(out)
            })
            .collect(),
    )
}

#[async_trait]
impl LanguageModel for OpenAIModel {
    /// Provider identity for recording/routing: `"{name}.chat"` (`"openai.chat"`
    /// by default).
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn supported_urls(&self) -> aimux_core::language_model::SupportedUrls {
        self.config.supported_urls.clone()
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let http =
            self.config
                .http_request(self.config.url("/chat/completions")?, headers, options);
        execute_generate(
            http,
            &self.model_id,
            options,
            self.config.transform_request_body.as_ref(),
            self.config.chat_options,
        )
        .await
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let http =
            self.config
                .http_request(self.config.url("/chat/completions")?, headers, options);
        execute_stream(
            http,
            &self.model_id,
            options,
            self.config.transform_request_body.as_ref(),
            self.config.chat_options,
        )
        .await
    }
}

// ── Shared OpenAI chat-completions execution ─────────────────────────────────
//
// These free functions contain the actual HTTP + response-parsing logic. They
// are shared with providers that speak the OpenAI chat wire format (Azure
// OpenAI), which supply their own request: URL, auth headers and transport.

/// Execute a non-streaming OpenAI chat-completion request.
///
/// `http` carries the full chat-completions URL, the auth and request headers
/// and the transport; `model_id` is placed in the request body's `model`
/// field. `transform_request_body`, when set, rewrites the finished body once
/// before it is sent (and the rewritten body is what the result reports).
///
/// # Errors
///
/// Returns request-build conversion errors, `ApiCall` for HTTP/transport
/// failures, `JsonParse` for a malformed body, and `InvalidResponseData` when
/// `choices` is empty.
pub(crate) async fn execute_generate(
    http: HttpRequest,
    model_id: &str,
    options: &CallOptions,
    transform_request_body: Option<&TransformRequestBody>,
    parse_chat_options: crate::openai::chat::chat_language_model_options::ChatOptionsParser,
) -> Result<GenerateResult, AiMuxError> {
    let request_result =
        build_request_body_with_chat_options(model_id, options, false, parse_chat_options)?;
    let body = match transform_request_body {
        Some(transform) => transform(request_result.body),
        None => request_result.body,
    };

    let resp = aimux_provider_utils::post_json_to_api(
        http,
        body.clone(),
        aimux_provider_utils::create_json_response_handler::<ChatCompletionResponse>(),
        crate::openai::error::openai_failed_response_handler(),
    )
    .await?;

    let response_headers = resp.response_headers;

    // Parse the raw body once: the `Value` keeps the provider's original
    // fields (incl. vendor-specific usage fields) for `Usage.raw` (M10).
    let response_value = resp.raw_value.unwrap_or(Value::Null);
    let data = resp.value;

    let choice = data.choices.into_iter().next().ok_or_else(|| {
        AiMuxError::invalid_response_data("Response did not contain any choices.")
    })?;

    // Build content array.
    let mut content = Vec::new();
    if let Some(text) = choice
        .message
        .content
        .filter(|text| !text.is_empty())
        .or_else(|| choice.message.audio.and_then(|audio| audio.transcript))
        && !text.is_empty()
    {
        content.push(GenerateContent::Text {
            text,
            provider_metadata: None,
        });
    }
    // Reasoning: prefer reasoning_content over reasoning.
    let reasoning_text = choice
        .message
        .reasoning_content
        .clone()
        .or_else(|| choice.message.reasoning.clone());
    if let Some(reasoning) = reasoning_text
        && !reasoning.is_empty()
    {
        content.push(GenerateContent::Reasoning(ReasoningOutput {
            text: reasoning,
            provider_metadata: None,
        }));
    }
    if let Some(tool_calls) = choice.message.tool_calls {
        for tc in tool_calls {
            let input = tc.function.arguments;
            content.push(GenerateContent::ToolCall(RawToolCall {
                tool_call_id: tc.id,
                tool_name: tc.function.name,
                input,
                provider_executed: None,
                dynamic: None,
                provider_metadata: None,
            }));
        }
    }
    // Parse annotations (URL citations) → Source content items.
    if let Some(annotations) = choice.message.annotations {
        for (i, ann) in annotations.iter().enumerate() {
            if ann.get("type").and_then(|v| v.as_str()) == Some("url_citation")
                && let Some(uc) = ann.get("url_citation")
            {
                content.push(GenerateContent::Source(Source::Url {
                    id: format!("annotation-{i}"),
                    url: uc
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    title: uc
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string),
                    provider_metadata: None,
                }));
            }
        }
    }

    let finish_reason = choice
        .finish_reason
        .as_deref()
        .map(parse_finish_reason)
        .unwrap_or(FINISH_OTHER);

    let usage = data.usage.as_ref().map_or_else(Usage::default, |usage| {
        convert_usage(usage, response_value.get("usage"))
    });
    let raw_usage = data.usage.unwrap_or_default();

    // Build provider metadata: logprobs + prediction tokens.
    let mut pm_openai = serde_json::json!({});
    if let Some(content) = choice.logprobs.as_ref().and_then(logprobs_content) {
        pm_openai["logprobs"] = content;
    }
    // Accepted/rejected prediction tokens from completion_tokens_details.
    if let Some(ref details) = raw_usage.completion_tokens_details {
        if let Some(apt) = details.accepted_prediction_tokens {
            pm_openai["acceptedPredictionTokens"] = json!(apt);
        }
        if let Some(rpt) = details.rejected_prediction_tokens {
            pm_openai["rejectedPredictionTokens"] = json!(rpt);
        }
    }
    let provider_metadata =
        Some(provider_namespace("openai", pm_openai).expect("provider metadata must be an object"));

    Ok(GenerateResult {
        content,
        finish_reason,
        usage,
        warnings: request_result.warnings,
        provider_metadata,
        response: Some(aimux_core::shared::ResponseInfo {
            id: data.id,
            timestamp: data
                .created
                .filter(|created| *created != 0)
                .and_then(|secs| aimux_core::Timestamp::from_unix_seconds(secs as f64)),
            model_id: data.model,
            headers: Some(response_headers),
            body: Some(response_value),
        }),
        request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
    })
}

/// Execute a streaming OpenAI chat-completion request.
///
/// `http` carries the full chat-completions URL, the auth and request headers
/// and the transport; `model_id` is placed in the request body's `model`
/// field. `transform_request_body` is applied as in [`execute_generate`].
///
/// # Errors
///
/// Returns request-build conversion errors and `ApiCall` when establishing the
/// stream fails; transport errors surface as `Err` items in the stream.
pub(crate) async fn execute_stream(
    http: HttpRequest,
    model_id: &str,
    options: &CallOptions,
    transform_request_body: Option<&TransformRequestBody>,
    parse_chat_options: crate::openai::chat::chat_language_model_options::ChatOptionsParser,
) -> Result<StreamResult, AiMuxError> {
    let request_result =
        build_request_body_with_chat_options(model_id, options, true, parse_chat_options)?;
    // M9 (RFC-0016): keep the warnings computed while building the body —
    // they are emitted in `StreamStart` below instead of being dropped.
    let RequestBodyResult { body, warnings } = request_result;
    let body = match transform_request_body {
        Some(transform) => transform(body),
        None => body,
    };
    let endpoint = http.url.clone();

    let resp = aimux_provider_utils::post_json_to_api(
        http,
        body.clone(),
        aimux_provider_utils::create_event_source_response_handler::<Value>(),
        crate::openai::error::openai_failed_response_handler(),
    )
    .await?;

    let response_headers = resp.response_headers;

    let mut sse_stream = resp.value;

    // Aimux checks only the first SSE event before returning the stream. The
    // baseline OpenAI provider scans until semantic output; this narrower peek
    // still keeps an immediately reported provider error inside Core's
    // operation-retry boundary (RFC-0031 §8.3). A normal event is chained back
    // below and is never consumed.
    let first_event = match sse_stream.next().await {
        Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
        first_event => first_event,
    };
    if let Some(Ok(ref event)) = first_event
        && let Some(err_obj) = event.get("error")
    {
        return Err(crate::openai::stream_error::openai_stream_error(
            err_obj,
            &endpoint,
            body.clone(),
            response_headers.clone(),
        ));
    }

    let transformer = OpenAIChatStream {
        ctx: StreamContext::new(options, warnings, endpoint, &body, &response_headers),
        ..OpenAIChatStream::default()
    };
    // Process the first event (already peeked) then the rest.
    let events = futures::stream::iter(first_event).chain(sse_stream);
    let stream = pipe_through(events, transformer);

    Ok(StreamResult {
        stream: Box::pin(stream),
        request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
        response: Some(aimux_core::shared::StreamResponseInfo {
            headers: Some(response_headers),
        }),
    })
}

const TEXT_ID: &str = "0";

const REASONING_ID: &str = "reasoning-0";

/// The `TransformStream` of the Chat Completions stream ([`execute_stream`]):
/// turns the parsed SSE events into stream parts.
#[derive(Default)]
pub(crate) struct OpenAIChatStream {
    ctx: StreamContext,
    /// Finalized on flush.
    tool_calls: ChatToolCalls,
    text_started: bool,
    reasoning_started: bool,
    final_usage: Usage,
    final_usage_raw: Option<UsageResponse>,
    final_finish_reason: Option<FinishReason>,
    response_metadata_emitted: bool,
    final_logprobs: Option<Value>,
    stream_errored: bool,
}

/// A chat stream with an empty request context and tool-call ids from
/// `generate_id`, for tests that drive it chunk by chunk.
#[cfg(test)]
pub(crate) fn chat_test_stream(
    generate_id: std::sync::Arc<dyn Fn() -> String + Send + Sync>,
) -> impl Transformer<Input = Result<Value, AiMuxError>, Output = StreamPart> {
    OpenAIChatStream {
        tool_calls: ChatToolCalls::new(generate_id),
        ..OpenAIChatStream::default()
    }
}

impl OpenAIChatStream {
    fn end_reasoning(&mut self, controller: &mut Controller) {
        if self.reasoning_started {
            controller.reasoning_end(REASONING_ID);
            self.reasoning_started = false;
        }
    }
}

impl Transformer for OpenAIChatStream {
    type Input = Result<Value, AiMuxError>;
    type Output = StreamPart;

    fn start(&mut self, controller: &mut Controller) {
        self.ctx.start(controller);
    }

    fn transform(&mut self, event: Result<Value, AiMuxError>, controller: &mut Controller) {
        let parsed = match event {
            Ok(parsed) => parsed,
            Err(error) => {
                let recoverable = error.is_recoverable_stream_error();
                self.stream_errored = true;
                controller.enqueue(StreamPart::Error { error });
                if !recoverable {
                    controller.terminate();
                }
                return;
            }
        };

        // M2 (RFC-0016): emit the raw provider chunk for debugging before it
        // is consumed below.
        self.ctx.raw(&parsed, controller);

        // Check for mid-stream error.
        if let Some(err_obj) = parsed.get("error") {
            controller.enqueue(StreamPart::Error {
                error: crate::openai::stream_error::openai_stream_error(
                    err_obj,
                    &self.ctx.url,
                    self.ctx.request_body.clone(),
                    self.ctx.response_headers.clone(),
                ),
            });
            self.stream_errored = true;
            return;
        }

        // The chunk's top-level `usage`, taken from the raw JSON before the
        // chunk is consumed below. `chunk_usage_raw` keeps the provider's
        // original object for `Usage.raw` (M10).
        let chunk_usage_raw: Option<Value> = parsed.get("usage").cloned();
        let chunk_usage: Option<UsageResponse> = chunk_usage_raw
            .as_ref()
            .and_then(|u| serde_json::from_value(u.clone()).ok());

        let chunk: StreamChunk = match serde_json::from_value(parsed) {
            Ok(c) => c,
            Err(e) => {
                self.stream_errored = true;
                controller.enqueue(StreamPart::Error { error: e.into() });
                return;
            }
        };

        // Emit ResponseMetadata from the first valid chunk.
        if !self.response_metadata_emitted
            && (chunk.id.as_ref().is_some_and(|id| !id.is_empty())
                || chunk.model.as_ref().is_some_and(|model| !model.is_empty())
                || chunk.created.is_some_and(|created| created != 0))
        {
            self.response_metadata_emitted = true;
            controller.enqueue(StreamPart::ResponseMetadata(ResponseMetadata {
                id: chunk.id.clone(),
                timestamp: chunk
                    .created
                    .filter(|created| *created != 0)
                    .and_then(|secs| aimux_core::Timestamp::from_unix_seconds(secs as f64)),
                model_id: chunk.model.clone(),
            }));
        }

        // Update usage from the chunk that carries it.
        if let Some(usage) = &chunk_usage {
            self.final_usage = convert_usage(usage, chunk_usage_raw.as_ref());
            self.final_usage_raw = Some(usage.clone());
        }

        for choice in chunk.choices {
            // The last chunk that carries logprobs sets them.
            if let Some(content) = choice.logprobs.as_ref().and_then(logprobs_content) {
                self.final_logprobs = Some(content);
            }

            // Reasoning delta: prefer reasoning_content over reasoning.
            let reasoning_delta = choice
                .delta
                .reasoning_content
                .clone()
                .or_else(|| choice.delta.reasoning.clone());
            if let Some(reasoning) = reasoning_delta
                && !reasoning.is_empty()
            {
                if !self.reasoning_started {
                    self.reasoning_started = true;
                    controller.reasoning_start(REASONING_ID);
                }
                controller.reasoning_delta(REASONING_ID, reasoning);
            }

            // Text delta; an active reasoning block ends before text starts.
            if let Some(content) = choice.delta.content {
                self.end_reasoning(controller);
                if !self.text_started {
                    self.text_started = true;
                    controller.text_start(TEXT_ID);
                }
                controller.text_delta(TEXT_ID, content);
            }

            // Tool-call deltas; an active reasoning block ends first.
            if let Some(tool_call_deltas) = choice.delta.tool_calls {
                self.end_reasoning(controller);
                for dtc in &tool_call_deltas {
                    let function = dtc.function.as_ref();
                    let delta = ToolCallDelta {
                        index: Some(dtc.index),
                        id: dtc.id.as_deref(),
                        name: function.and_then(|f| f.name.as_deref()),
                        arguments: function.and_then(|f| f.arguments.as_deref()),
                        // `type` is optional, but must be 'function' when present.
                        invalid_type: dtc.r#type.as_deref().is_some_and(|t| t != "function"),
                        provider_metadata: None,
                    };
                    // A malformed delta (new call without a function name, or
                    // a non-`function` type) is invalid response data, as in
                    // the AI SDK; the stream ends.
                    if !self.tool_calls.delta(delta, controller) {
                        return;
                    }
                }
            }

            // Annotations / citations (URL citations → Source).
            if let Some(annotations) = choice.delta.annotations {
                for (i, ann) in annotations.iter().enumerate() {
                    if ann.get("type").and_then(|v| v.as_str()) == Some("url_citation")
                        && let Some(uc) = ann.get("url_citation")
                    {
                        controller.enqueue(StreamPart::Source(Source::Url {
                            id: format!("annotation-{i}"),
                            url: uc
                                .get("url")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            title: uc
                                .get("title")
                                .and_then(|v| v.as_str())
                                .map(std::string::ToString::to_string),
                            provider_metadata: None,
                        }));
                    }
                }
            }

            // Finish reason; closes any open reasoning segment.
            if let Some(reason) = choice.finish_reason {
                self.end_reasoning(controller);
                self.stream_errored = false;
                self.final_finish_reason = Some(parse_finish_reason(&reason));
            }
        }
    }

    fn flush(mut self, controller: &mut Controller) {
        // Close any remaining open reasoning and text segments.
        self.end_reasoning(controller);
        if self.text_started {
            controller.text_end(TEXT_ID);
        }

        self.tool_calls.finish(controller);

        // Provider metadata for the Finish part: logprobs + prediction tokens.
        let mut pm_openai = serde_json::json!({});
        if let Some(ref logprobs) = self.final_logprobs {
            pm_openai["logprobs"] = json!(logprobs);
        }
        if let Some(ref raw_usage) = self.final_usage_raw
            && let Some(ref details) = raw_usage.completion_tokens_details
        {
            if let Some(apt) = details.accepted_prediction_tokens {
                pm_openai["acceptedPredictionTokens"] = json!(apt);
            }
            if let Some(rpt) = details.rejected_prediction_tokens {
                pm_openai["rejectedPredictionTokens"] = json!(rpt);
            }
        }
        let provider_metadata =
            provider_namespace("openai", pm_openai).expect("provider metadata must be an object");

        controller.enqueue(StreamPart::Finish {
            finish_reason: if self.stream_errored {
                FINISH_ERROR
            } else {
                self.final_finish_reason.unwrap_or(FINISH_OTHER)
            },
            usage: self.final_usage,
            provider_metadata: Some(provider_metadata),
        });
    }
}

/// Result of building a request body, including warnings.
#[derive(Debug, Clone)]
pub struct RequestBodyResult {
    pub body: Value,
    pub warnings: Vec<Warning>,
}

/// Resolve the effective reasoning effort — direct passthrough (v3: no built-in
/// vendor normalization). `providerOptions.reasoningEffort` wins over top-level
/// `reasoning`; custom top-level levels map verbatim to `reasoning_effort`.
fn resolve_reasoning_effort(
    provider_opts: &Option<SharedProviderOptions>,
    reasoning: &Option<ReasoningEffort>,
) -> Option<String> {
    openai_option(provider_opts, "reasoningEffort")
        .map(|v| {
            v.as_str()
                .map(std::string::ToString::to_string)
                .unwrap_or(v.to_string())
        })
        .or_else(|| {
            if reasoning.is_some_and(ReasoningEffort::is_custom) {
                reasoning.map(|r| r.to_string())
            } else {
                None
            }
        })
}

fn resolve_is_reasoning_model(
    provider_opts: &Option<SharedProviderOptions>,
    caps: &ModelCapabilities,
) -> bool {
    openai_option(provider_opts, "forceReasoning")
        .map(|v| v.as_bool().unwrap_or(false))
        .unwrap_or(caps.is_reasoning_model)
}

fn resolve_system_message_mode(
    provider_opts: &Option<SharedProviderOptions>,
    is_reasoning_model: bool,
    caps: &ModelCapabilities,
) -> SystemMessageMode {
    openai_option(provider_opts, "systemMessageMode")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string))
        .map(|s| match s.as_str() {
            "developer" => SystemMessageMode::Developer,
            "remove" => SystemMessageMode::Remove,
            _ => SystemMessageMode::System,
        })
        .unwrap_or(if is_reasoning_model {
            SystemMessageMode::Developer
        } else {
            caps.system_message_mode
        })
}

/// Insert `max_tokens` / `max_completion_tokens`: reasoning models take the
/// latter, the rest the former; an explicit `maxCompletionTokens` option is
/// always sent as `max_completion_tokens`.
fn apply_max_tokens(
    body: &mut Value,
    options: &CallOptions,
    provider_opts: &Option<SharedProviderOptions>,
    is_reasoning_model: bool,
) {
    let max_completion_tokens_opt = openai_option(provider_opts, "maxCompletionTokens");

    if let Some(max_tokens) = options.max_output_tokens {
        let key = if is_reasoning_model {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        body[key] = json!(max_tokens);
    }
    if let Some(mct) = max_completion_tokens_opt {
        body["max_completion_tokens"] = json!(mct);
    }
}

/// Sampling parameters that survive the reasoning-model / search-preview
/// capability filtering.
#[derive(Default)]
struct SamplingParams {
    temperature: Option<f64>,
    top_p: Option<f64>,
    frequency_penalty: Option<f64>,
    presence_penalty: Option<f64>,
}

/// Remove unsupported sampling settings for reasoning models and the search
/// preview models, pushing a compatibility warning for each removal.
fn strip_sampling_params(
    options: &CallOptions,
    model_id: &str,
    caps: &ModelCapabilities,
    is_reasoning_model: bool,
    resolved_reasoning_effort: &Option<String>,
    warnings: &mut Vec<Warning>,
) -> SamplingParams {
    let mut params = SamplingParams {
        temperature: options.temperature,
        top_p: options.top_p,
        frequency_penalty: options.frequency_penalty,
        presence_penalty: options.presence_penalty,
    };

    if is_reasoning_model {
        let allow_non_reasoning = resolved_reasoning_effort.as_deref() == Some("none")
            && caps.supports_non_reasoning_parameters;

        if !allow_non_reasoning {
            if params.temperature.is_some() {
                params.temperature = None;
                warnings.push(Warning::Unsupported {
                    feature: "temperature".to_string(),
                    details: Some("temperature is not supported for reasoning models".to_string()),
                });
            }
            if params.top_p.is_some() {
                params.top_p = None;
                warnings.push(Warning::Unsupported {
                    feature: "topP".to_string(),
                    details: Some("topP is not supported for reasoning models".to_string()),
                });
            }
        }

        if params.frequency_penalty.is_some() {
            params.frequency_penalty = None;
            warnings.push(Warning::Unsupported {
                feature: "frequencyPenalty".to_string(),
                details: Some("frequencyPenalty is not supported for reasoning models".to_string()),
            });
        }
        if params.presence_penalty.is_some() {
            params.presence_penalty = None;
            warnings.push(Warning::Unsupported {
                feature: "presencePenalty".to_string(),
                details: Some("presencePenalty is not supported for reasoning models".to_string()),
            });
        }
    } else if (model_id.starts_with("gpt-4o-search-preview")
        || model_id.starts_with("gpt-4o-mini-search-preview"))
        && params.temperature.is_some()
    {
        params.temperature = None;
        warnings.push(Warning::Unsupported {
            feature: "temperature".to_string(),
            details: Some(
                "temperature is not supported for the search preview models and has been removed."
                    .to_string(),
            ),
        });
    }

    params
}

/// Write the surviving sampling params (plus stop/seed) into the body.
fn insert_sampling_params(body: &mut Value, params: &SamplingParams, options: &CallOptions) {
    if let Some(temp) = params.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(tp) = params.top_p {
        body["top_p"] = json!(tp);
    }
    if let Some(fp) = params.frequency_penalty {
        body["frequency_penalty"] = json!(fp);
    }
    if let Some(pp) = params.presence_penalty {
        body["presence_penalty"] = json!(pp);
    }
    if let Some(ref stop) = options.stop_sequences {
        body["stop"] = json!(stop);
    }
    if let Some(seed) = options.seed {
        body["seed"] = json!(seed);
    }
}

/// `response_format` → body: a schema is sent as `json_schema` (strict), a
/// bare JSON request as `json_object`.
fn apply_response_format(
    body: &mut Value,
    options: &CallOptions,
    warnings: &mut Vec<Warning>,
) -> Result<(), AiMuxError> {
    let Some(ref rf) = options.response_format else {
        return Ok(());
    };
    match rf {
        ResponseFormat::Text => {}
        ResponseFormat::Json {
            schema,
            name,
            description,
        } => {
            if let Some(schema) = schema {
                let mut schema_obj = json!({});
                schema_obj["schema"] = normalize_json_schema(schema, warnings)?;
                schema_obj["name"] = json!(name.clone().unwrap_or_else(|| "response".to_string()));
                if let Some(d) = description {
                    schema_obj["description"] = json!(d);
                }
                schema_obj["strict"] = openai_option(&options.provider_options, "strictJsonSchema")
                    .unwrap_or(json!(true));
                body["response_format"] = json!({
                    "type": "json_schema",
                    "json_schema": schema_obj,
                });
            } else {
                body["response_format"] = json!({ "type": "json_object" });
            }
        }
    }
    Ok(())
}

/// Pass through the simple provider-specific options that map 1:1 to a body
/// field.
fn apply_provider_option_passthrough(
    body: &mut Value,
    provider_opts: &Option<SharedProviderOptions>,
) {
    let mut set = |key: &str, body_key: &str| {
        if let Some(val) = openai_option(provider_opts, key) {
            body[body_key] = val;
        }
    };
    set("logitBias", "logit_bias");
    set("user", "user");
    set("parallelToolCalls", "parallel_tool_calls");
    set("textVerbosity", "verbosity");
    set("store", "store");
    set("metadata", "metadata");
    set("prediction", "prediction");
    set("promptCacheKey", "prompt_cache_key");
    set("promptCacheRetention", "prompt_cache_retention");
    set("promptCacheOptions", "prompt_cache_options");
    set("safetyIdentifier", "safety_identifier");
    if let Some(logprobs) = openai_option(provider_opts, "logprobs")
        && (logprobs == true || logprobs.is_number())
    {
        body["logprobs"] = json!(true);
        body["top_logprobs"] = if logprobs.is_number() {
            logprobs
        } else {
            json!(0)
        };
    }
}

/// `service_tier` with model-capability validation.
fn apply_service_tier(
    body: &mut Value,
    provider_opts: &Option<SharedProviderOptions>,
    caps: &ModelCapabilities,
    warnings: &mut Vec<Warning>,
) {
    let service_tier = openai_option(provider_opts, "serviceTier")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string));
    if let Some(ref st) = service_tier {
        match st.as_str() {
            "flex" => {
                if caps.supports_flex_processing {
                    body["service_tier"] = json!(st);
                } else {
                    warnings.push(Warning::Unsupported {
                        feature: "serviceTier".to_string(),
                        details: Some(
                            "flex processing is only available for o3, o4-mini, and gpt-5 models"
                                .to_string(),
                        ),
                    });
                }
            }
            "priority" | "fast" => {
                if caps.supports_priority_processing {
                    body["service_tier"] = json!(st);
                } else {
                    warnings.push(Warning::Unsupported {
                        feature: "serviceTier".to_string(),
                        details: Some(
                            "priority processing is only available for supported models (gpt-4, gpt-5, gpt-5-mini, o3, o4-mini) and requires Enterprise access. gpt-5-nano is not supported".to_string(),
                        ),
                    });
                }
            }
            _ => {
                body["service_tier"] = json!(st);
            }
        }
    }
}

/// Function tools → `tools` / `tool_choice`.
fn apply_tools(
    body: &mut Value,
    options: &CallOptions,
    warnings: &mut Vec<Warning>,
) -> Result<(), AiMuxError> {
    let function_tools: Option<Vec<FunctionTool>> = options.tools.as_ref().map(|tools| {
        tools
            .iter()
            .filter_map(|t| match t {
                Tool::Function(ft) => Some(ft.clone()),
                Tool::Provider(_) => None,
            })
            .collect()
    });

    let mut function_tools = function_tools;
    for tool in function_tools.iter_mut().flatten() {
        tool.input_schema = normalize_json_schema(&tool.input_schema, warnings)?;
    }
    for tool in options.tools.iter().flatten() {
        if matches!(tool, Tool::Provider(_)) {
            warnings.push(Warning::Unsupported {
                feature: "tool type: provider".into(),
                details: None,
            });
        }
    }
    if options
        .tools
        .as_ref()
        .is_some_and(|tools| !tools.is_empty())
        && function_tools.as_ref().is_some_and(Vec::is_empty)
    {
        body["tools"] = json!([]);
    }
    let prepared = prepare_tools(&function_tools, options.tool_choice.as_ref());
    if let Some(tools) = prepared.tools {
        body["tools"] = json!(tools);
        if let Some(tc) = prepared.tool_choice {
            body["tool_choice"] = tc;
        }
    }
    Ok(())
}

/// Convert `CallOptions` to an OpenAI request body, returning warnings.
///
/// Conversion errors propagate to the caller (fail-fast, issue H2): the old
/// behaviour of silently returning `body: null` sent empty requests upstream
/// and made conversion failures invisible.
///
/// # Errors
///
/// Propagates conversion errors, fail-fast: e.g. `InvalidArgument` for an
/// unconvertible prompt part or an invalid option combination.
#[cfg(test)]
pub(crate) fn build_request_body_with_warnings(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<RequestBodyResult, AiMuxError> {
    build_request_body_with_chat_options(
        model_id,
        options,
        stream,
        crate::openai::chat::chat_language_model_options::parse_chat_options,
    )
}

pub(crate) fn build_request_body_with_chat_options(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
    parse: crate::openai::chat::chat_language_model_options::ChatOptionsParser,
) -> Result<RequestBodyResult, AiMuxError> {
    let mut warnings: Vec<Warning> = Vec::new();
    let caps = get_model_capabilities(model_id);
    let parsed_provider_opts = parse_chat_provider_options(
        options.provider_options.as_ref(),
        crate::openai::chat::chat_language_model_options::NAMESPACE,
        parse,
    )?
    .map(|parsed| {
        aimux_core::shared::NamespaceMap::from([(
            crate::openai::chat::chat_language_model_options::NAMESPACE.to_string(),
            parsed,
        )])
    });
    let provider_opts = &parsed_provider_opts;

    let mut resolved_reasoning_effort = resolve_reasoning_effort(provider_opts, &options.reasoning);
    if let (Some(effort), Some(supported)) =
        (&resolved_reasoning_effort, caps.supported_reasoning_efforts)
        && !supported.contains(&effort.as_str())
    {
        warnings.push(Warning::Unsupported {
            feature: "reasoningEffort".into(),
            details: Some(format!(
                "{model_id} only supports the following reasoning efforts: {}",
                supported.join(", ")
            )),
        });
        resolved_reasoning_effort = None;
    }
    let is_reasoning_model = resolve_is_reasoning_model(provider_opts, &caps);
    let system_message_mode = resolve_system_message_mode(provider_opts, is_reasoning_model, &caps);

    // OpenAI has no top_k.
    if options.top_k.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "topK".to_string(),
            details: None,
        });
    }

    if system_message_mode == SystemMessageMode::Remove {
        for _ in options
            .prompt
            .iter()
            .filter(|m| matches!(m, LanguageModelMessage::System { .. }))
        {
            warnings.push(Warning::Other {
                message: "system messages are removed for this model".into(),
            });
        }
    }
    let messages =
        convert_prompt_to_openai_messages_with_mode_fallible(&options.prompt, system_message_mode)?;

    let mut body = json!({
        "model": model_id,
        "messages": messages,
    });

    if stream {
        body["stream"] = json!(true);
        body["stream_options"] = json!({ "include_usage": true });
    }

    apply_max_tokens(&mut body, options, provider_opts, is_reasoning_model);

    let sampling = strip_sampling_params(
        options,
        model_id,
        &caps,
        is_reasoning_model,
        &resolved_reasoning_effort,
        &mut warnings,
    );
    insert_sampling_params(&mut body, &sampling, options);

    apply_response_format(&mut body, options, &mut warnings)?;
    apply_provider_option_passthrough(&mut body, provider_opts);

    if caps.supported_reasoning_efforts.is_some() && body.get("prompt_cache_retention").is_some() {
        body.as_object_mut()
            .unwrap()
            .remove("prompt_cache_retention");
        warnings.push(Warning::Unsupported { feature: "promptCacheRetention".into(), details: Some("promptCacheRetention is not supported by GPT-6 and later models; use promptCacheOptions instead".into()) });
    }
    if is_reasoning_model {
        let allow_logprobs = resolved_reasoning_effort.as_deref() == Some("none")
            && caps.supports_non_reasoning_parameters;
        for (key, feature) in [
            ("logprobs", "logprobs"),
            ("logit_bias", "logitBias"),
            ("top_logprobs", "topLogprobs"),
        ] {
            if key == "logprobs" && allow_logprobs {
                continue;
            }
            if body.as_object_mut().unwrap().remove(key).is_some() {
                warnings.push(Warning::Other {
                    message: format!("{feature} is not supported for reasoning models"),
                });
            }
        }
    }

    // Provider reasoning takes precedence over the standardized setting.
    if let Some(ref effort) = resolved_reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }

    apply_service_tier(&mut body, provider_opts, &caps, &mut warnings);
    apply_tools(&mut body, options, &mut warnings)?;

    Ok(RequestBodyResult { body, warnings })
}
