//! The OpenAI-compatible chat language model (`{name}.chat`).
//!
//! The Rust form of `OpenAICompatibleChatLanguageModel`. Request construction
//! lives in [`super::convert`]; vendor differences arrive through the
//! [`ChatSettings`](super::config::ChatSettings) of the model's config.

use aimux_core::tool::RawToolCall;
use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, Source, StreamResult};
use aimux_core::shared::{
    RequestInfo, ResponseInfo, SharedProviderMetadata, StreamResponseInfo, Warning,
    provider_namespace,
};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, InputTokenUsage, OutputTokenUsage, ResponseMetadata, Usage,
};
use aimux_provider_utils::{
    StreamingToolCallDelta, StreamingToolCallTracker, TransformStreamController, Transformer,
    generate_id, pipe_through,
};

use super::config::{CompatModelConfig, ConvertUsage};
use super::convert::{ChatBodySpec, RequestBodyResult, build_request_body, parse_finish_reason};
use super::types::{ChatCompletionResponse, StreamChunk, UsageResponse};

/// An OpenAI-compatible chat model. Holds no HTTP client: the request helpers
/// of `aimux-provider-utils` use the injected or process-default transport.
pub struct OpenAICompatibleChatModel {
    model_id: String,
    config: CompatModelConfig,
}

impl OpenAICompatibleChatModel {
    pub(crate) fn from_config(model_id: String, config: CompatModelConfig) -> Self {
        Self { model_id, config }
    }

    /// The JSON body a call would send, with the warnings raised while
    /// building it. Nothing is read from the environment and nothing is sent:
    /// a key is not resolved here.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a malformed provider option or an unconvertible
    /// prompt part.
    pub(crate) fn request_body(
        &self,
        options: &CallOptions,
        stream: bool,
    ) -> Result<RequestBodyResult, AiMuxError> {
        build_request_body(
            &self.model_id,
            options,
            stream,
            &ChatBodySpec {
                provider_options_name: self.config.provider_options_name(),
                chat: &self.config.chat,
            },
        )
    }
}

// ── Usage ────────────────────────────────────────────────────────────────────

/// Core usage from a parsed OpenAI-shaped `usage` object.
///
/// `raw` is the provider's original object, kept verbatim in `Usage.raw`.
/// Cache reads come from `prompt_tokens_details`, as in the upstream converter.
pub(crate) fn convert_usage(usage: &UsageResponse, raw: Option<&Value>) -> Usage {
    let prompt_tokens = usage.prompt_tokens.unwrap_or(0);
    let completion_tokens = usage.completion_tokens.unwrap_or(0);

    let cached = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|d| d.cached_tokens)
        .unwrap_or(0);

    // Saturating: some servers report more cached or reasoning tokens than
    // prompt or completion tokens.
    let no_cache = prompt_tokens.saturating_sub(cached);
    let reasoning_tokens = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens)
        .unwrap_or(0);

    Usage {
        input_tokens: InputTokenUsage {
            total: Some(prompt_tokens),
            no_cache: Some(no_cache),
            cache_read: Some(cached),
            cache_write: None,
        },
        output_tokens: OutputTokenUsage {
            total: Some(completion_tokens),
            text: Some(completion_tokens.saturating_sub(reasoning_tokens)),
            reasoning: Some(reasoning_tokens),
        },
        raw: raw.and_then(Value::as_object).cloned(),
    }
}

/// Usage from a raw `usage` value.
pub(crate) fn usage_from_raw(raw: Option<&Value>) -> Usage {
    let Some(raw) = raw.filter(|value| !value.is_null()) else {
        return Usage::default();
    };
    match serde_json::from_value::<UsageResponse>(raw.clone()) {
        Ok(parsed) => convert_usage(&parsed, Some(raw)),
        Err(_) => Usage {
            raw: raw.as_object().cloned(),
            ..Usage::default()
        },
    }
}

fn prediction_tokens(usage: Option<&UsageResponse>, metadata: &mut Map<String, Value>) {
    if let Some(details) = usage.and_then(|u| u.completion_tokens_details.as_ref()) {
        if let Some(accepted) = details.accepted_prediction_tokens {
            metadata.insert("acceptedPredictionTokens".into(), json!(accepted));
        }
        if let Some(rejected) = details.rejected_prediction_tokens {
            metadata.insert("rejectedPredictionTokens".into(), json!(rejected));
        }
    }
}

fn thought_signature(extra_content: Option<&Value>) -> Option<String> {
    extra_content
        .and_then(|extra| extra.pointer("/google/thought_signature"))
        .and_then(Value::as_str)
        .filter(|signature| !signature.is_empty())
        .map(str::to_string)
}

fn stream_error(
    name: &str,
    error: &Value,
    url: &str,
    request_body_values: Value,
    response_headers: HashMap<String, String>,
) -> AiMuxError {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{name} stream failed before any output was generated"));
    let code = error.get("code").or_else(|| error.get("type"));
    let provider_code = code.and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    });
    // Only a numeric HTTP status in the payload is a status; a string code
    // must not be laundered into a retryable 500.
    let status_code = code
        .and_then(Value::as_u64)
        .filter(|status| (400..=599).contains(status))
        .map(|status| status as u16);
    aimux_provider_utils::stream_error_api_call(
        message,
        provider_code,
        status_code,
        error,
        url,
        request_body_values,
        response_headers,
    )
}

// ── LanguageModel ────────────────────────────────────────────────────────────

#[async_trait]
impl LanguageModel for OpenAICompatibleChatModel {
    /// `"{name}.chat"`.
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn supported_urls(&self) -> SupportedUrls {
        self.config.chat.supported_urls.clone()
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let built = self.request_body(options, false)?;
        let metadata_key = built.metadata_key;
        let body = built.body;
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let http = self
            .config
            .http_request("/chat/completions", headers, options)?;

        let resp = aimux_provider_utils::post_json_to_api(
            http,
            body.clone(),
            aimux_provider_utils::create_json_response_handler::<ChatCompletionResponse>(),
            self.config.failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let raw = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

        let choice = data.choices.into_iter().next().ok_or_else(|| {
            AiMuxError::InvalidResponseData("Response did not contain any choices.".to_string())
        })?;

        let mut content = Vec::new();
        content.extend(content_parts(choice.message.content.as_ref()));
        let reasoning = choice
            .message
            .reasoning_content
            .or(choice.message.reasoning)
            .filter(|text| !text.is_empty());
        if let Some(text) = reasoning {
            content.push(GenerateContent::Reasoning(ReasoningOutput {
                text,
                provider_metadata: None,
            }));
        }
        for call in choice.message.tool_calls.into_iter().flatten() {
            let signature = thought_signature(call.extra_content.as_ref());
            content.push(GenerateContent::ToolCall(RawToolCall {
                tool_call_id: call
                    .id
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(generate_id),
                tool_name: call.function.name,
                input: call.function.arguments.unwrap_or_default(),
                provider_executed: None,
                dynamic: None,
                provider_metadata: signature.as_ref().map(|signature| {
                    provider_namespace(&metadata_key, json!({ "thoughtSignature": signature }))
                        .expect("provider metadata must be an object")
                }),
            }));
        }
        for (i, annotation) in choice.message.annotations.iter().flatten().enumerate() {
            if annotation.get("type").and_then(Value::as_str) == Some("url_citation")
                && let Some(citation) = annotation.get("url_citation")
            {
                content.push(GenerateContent::Source(Source::Url {
                    id: format!("annotation-{i}"),
                    url: citation
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    title: citation
                        .get("title")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    provider_metadata: None,
                }));
            }
        }

        let finish_reason = choice
            .finish_reason
            .as_deref()
            .map(parse_finish_reason)
            .unwrap_or(FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            });

        let usage = self.config.chat.convert_usage.convert(raw.get("usage"));

        let mut metadata = HashMap::new();
        metadata.insert(metadata_key.clone(), Map::new());

        prediction_tokens(
            data.usage.as_ref(),
            metadata.entry(metadata_key).or_default(),
        );

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: built.warnings,
            provider_metadata: Some(metadata),
            response: Some(ResponseInfo {
                id: data.id,
                timestamp: data
                    .created
                    .filter(|created| *created != 0)
                    .and_then(|secs| chrono::DateTime::from_timestamp(secs as i64, 0))
                    .map(|dt| dt.to_rfc3339()),
                model_id: data.model,
                headers: Some(response_headers),
                body: Some(raw),
            }),
            request: Some(RequestInfo { body: Some(body) }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let built = self.request_body(options, true)?;
        let metadata_key = built.metadata_key;
        let warnings = built.warnings;
        let body = built.body;
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let http = self
            .config
            .http_request("/chat/completions", headers, options)?;
        let endpoint = http.url.clone();

        let resp = aimux_provider_utils::post_json_to_api(
            http,
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            self.config.failed_response_handler(),
        )
        .await?;
        let response_headers = resp.response_headers;
        let sse_stream = resp.value;

        let transformer = OpenAICompatibleChatStream {
            warnings,
            metadata_key,
            convert_usage: self.config.chat.convert_usage,
            stream_usage_key: self.config.chat.stream_usage_key.clone(),
            include_raw_chunks: options.include_raw_chunks == Some(true),
            url: endpoint,
            request_body: body.clone(),
            response_headers: response_headers.clone(),
            text_started: false,
            reasoning_started: false,
            final_usage_raw: None,
            final_usage_parsed: None,
            final_finish_reason: None,
            response_metadata_emitted: false,
            tool_calls: StreamingToolCallTracker::new().with_generate_id(generate_id),
            tool_parts: Vec::new(),
            pending: HashMap::new(),
            forwarded: HashSet::new(),
        };
        let stream = pipe_through(sse_stream, transformer);

        Ok(StreamResult {
            stream: Box::pin(stream),
            request: Some(RequestInfo { body: Some(body) }),
            response: Some(StreamResponseInfo {
                headers: Some(response_headers),
            }),
        })
    }
}

const TEXT_ID: &str = "txt-0";
const REASONING_ID: &str = "reasoning-0";

/// The `TransformStream` of [`OpenAICompatibleChatModel::do_stream`]: turns
/// the parsed SSE events into stream parts.
struct OpenAICompatibleChatStream {
    warnings: Vec<Warning>,
    metadata_key: String,
    convert_usage: ConvertUsage,
    stream_usage_key: Option<String>,
    include_raw_chunks: bool,
    /// Request context for in-stream `error` payloads.
    url: String,
    request_body: Value,
    response_headers: HashMap<String, String>,

    text_started: bool,
    reasoning_started: bool,
    final_usage_raw: Option<Value>,
    final_usage_parsed: Option<UsageResponse>,
    final_finish_reason: Option<FinishReason>,
    response_metadata_emitted: bool,
    tool_calls: StreamingToolCallTracker,
    tool_parts: Vec<StreamPart>,
    /// Some compatible servers send the first delta of a call without
    /// `function.name`; buffer by index until the name is known.
    pending: HashMap<usize, PendingToolCall>,
    forwarded: HashSet<usize>,
}

impl OpenAICompatibleChatStream {
    fn signature_metadata(&self, extra: Option<&Value>) -> Option<SharedProviderMetadata> {
        thought_signature(extra).map(|signature| {
            provider_namespace(&self.metadata_key, json!({ "thoughtSignature": signature }))
                .expect("provider metadata must be an object")
        })
    }

    fn enqueue_tool_parts(&mut self, controller: &mut TransformStreamController<StreamPart>) {
        for part in self.tool_parts.drain(..) {
            controller.enqueue(part);
        }
    }

    fn error_finish_reason() -> FinishReason {
        FinishReason {
            unified: FinishReasonUnified::Error,
            raw: None,
        }
    }
}

impl Transformer for OpenAICompatibleChatStream {
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
        let parsed = match event {
            Ok(parsed) => parsed,
            Err(error) => {
                self.final_finish_reason = Some(Self::error_finish_reason());
                controller.enqueue(StreamPart::Error { error });
                return;
            }
        };
        if self.include_raw_chunks {
            controller.enqueue(StreamPart::Raw {
                raw_value: parsed.clone(),
            });
        }
        if let Some(error) = parsed.get("error") {
            controller.enqueue(StreamPart::Error {
                error: stream_error(
                    &self.metadata_key,
                    error,
                    &self.url,
                    self.request_body.clone(),
                    self.response_headers.clone(),
                ),
            });
            self.final_finish_reason = Some(Self::error_finish_reason());
            return;
        }

        let chunk_usage_raw: Option<Value> = match &self.stream_usage_key {
            Some(key) => parsed.get(key).and_then(|v| v.get("usage")).cloned(),
            None => parsed.get("usage").cloned(),
        }
        .filter(|usage| !usage.is_null());

        let chunk: StreamChunk = match serde_json::from_value(parsed) {
            Ok(chunk) => chunk,
            Err(e) => {
                self.final_finish_reason = Some(Self::error_finish_reason());
                controller.enqueue(StreamPart::Error { error: e.into() });
                return;
            }
        };

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
                    .and_then(|secs| chrono::DateTime::from_timestamp(secs as i64, 0))
                    .map(|dt| dt.to_rfc3339()),
                model_id: chunk.model.clone(),
            }));
        }

        if let Some(raw_usage) = &chunk_usage_raw {
            self.final_usage_raw = Some(raw_usage.clone());
            self.final_usage_parsed = serde_json::from_value(raw_usage.clone()).ok();
        }

        for choice in chunk.choices.into_iter().take(1) {
            let reasoning_delta = choice
                .delta
                .reasoning_content
                .clone()
                .or_else(|| choice.delta.reasoning.clone());
            let mut delta_content = Vec::new();
            if let Some(text) = reasoning_delta.filter(|text| !text.is_empty()) {
                delta_content.push(GenerateContent::Reasoning(ReasoningOutput {
                    text,
                    provider_metadata: None,
                }));
            }
            delta_content.extend(content_parts(choice.delta.content.as_ref()));
            for part in delta_content {
                match part {
                    GenerateContent::Reasoning(ReasoningOutput { text, .. }) => {
                        if self.text_started {
                            controller.enqueue(StreamPart::TextEnd {
                                id: TEXT_ID.to_string(),
                                provider_metadata: None,
                            });
                            self.text_started = false;
                        }
                        if !self.reasoning_started {
                            self.reasoning_started = true;
                            controller.enqueue(StreamPart::ReasoningStart {
                                id: REASONING_ID.to_string(),
                                provider_metadata: None,
                            });
                        }
                        controller.enqueue(StreamPart::ReasoningDelta {
                            id: REASONING_ID.to_string(),
                            delta: text,
                            provider_metadata: None,
                        });
                    }
                    GenerateContent::Text { text, .. } => {
                        if self.reasoning_started {
                            controller.enqueue(StreamPart::ReasoningEnd {
                                id: REASONING_ID.to_string(),
                                provider_metadata: None,
                            });
                            self.reasoning_started = false;
                        }
                        if !self.text_started {
                            self.text_started = true;
                            controller.enqueue(StreamPart::TextStart {
                                id: TEXT_ID.to_string(),
                                provider_metadata: None,
                            });
                        }
                        controller.enqueue(StreamPart::TextDelta {
                            id: TEXT_ID.to_string(),
                            delta: text,
                            provider_metadata: None,
                        });
                    }
                    _ => unreachable!(),
                }
            }

            if let Some(deltas) = choice.delta.tool_calls.filter(|deltas| !deltas.is_empty()) {
                if self.reasoning_started {
                    controller.enqueue(StreamPart::ReasoningEnd {
                        id: REASONING_ID.to_string(),
                        provider_metadata: None,
                    });
                    self.reasoning_started = false;
                }
                for dtc in &deltas {
                    let function = dtc.function.as_ref();
                    let name = function
                        .and_then(|f| f.name.as_deref())
                        .filter(|n| !n.trim().is_empty());
                    let arguments = function.and_then(|f| f.arguments.as_deref());
                    let buffered;
                    let delta = match dtc.index {
                        Some(index) if !self.forwarded.contains(&index) => {
                            let entry = self.pending.entry(index).or_default();
                            if entry.id.is_none() {
                                entry.id.clone_from(&dtc.id);
                            }
                            if entry.extra.is_null()
                                && let Some(extra) = &dtc.extra_content
                            {
                                entry.extra = extra.clone();
                            }
                            if let Some(arguments) = arguments {
                                entry.arguments.push_str(arguments);
                            }
                            let Some(name) = name else { continue };
                            buffered = self.pending.remove(&index).unwrap_or_default();
                            self.forwarded.insert(index);
                            StreamingToolCallDelta {
                                index: Some(index),
                                id: buffered.id.as_deref(),
                                name: Some(name),
                                arguments: Some(&buffered.arguments),
                                provider_metadata: self.signature_metadata(Some(&buffered.extra)),
                                ..Default::default()
                            }
                        }
                        _ => StreamingToolCallDelta {
                            index: dtc.index,
                            id: dtc.id.as_deref(),
                            name: function.and_then(|f| f.name.as_deref()),
                            arguments,
                            provider_metadata: self.signature_metadata(dtc.extra_content.as_ref()),
                            ..Default::default()
                        },
                    };
                    if let Err(error) = self.tool_calls.process(delta, &mut self.tool_parts) {
                        controller.error(error.into());
                        return;
                    }
                    self.enqueue_tool_parts(controller);
                }
            }

            if let Some(annotations) = choice.delta.annotations {
                for (i, annotation) in annotations.iter().enumerate() {
                    if annotation.get("type").and_then(Value::as_str) == Some("url_citation")
                        && let Some(citation) = annotation.get("url_citation")
                    {
                        controller.enqueue(StreamPart::Source(Source::Url {
                            id: format!("annotation-{i}"),
                            url: citation
                                .get("url")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            title: citation
                                .get("title")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            provider_metadata: None,
                        }));
                    }
                }
            }

            if let Some(reason) = choice.finish_reason {
                self.final_finish_reason = Some(parse_finish_reason(&reason));
            }
        }
    }

    fn flush(mut self, controller: &mut TransformStreamController<StreamPart>) {
        if self.reasoning_started {
            controller.enqueue(StreamPart::ReasoningEnd {
                id: REASONING_ID.to_string(),
                provider_metadata: None,
            });
        }
        if self.text_started {
            controller.enqueue(StreamPart::TextEnd {
                id: TEXT_ID.to_string(),
                provider_metadata: None,
            });
        }
        for (index, entry) in std::mem::take(&mut self.pending) {
            let delta = StreamingToolCallDelta {
                index: Some(index),
                id: entry.id.as_deref(),
                arguments: Some(&entry.arguments),
                provider_metadata: self.signature_metadata(Some(&entry.extra)),
                ..Default::default()
            };
            if let Err(error) = self.tool_calls.process(delta, &mut self.tool_parts) {
                controller.error(error.into());
                return;
            }
            self.enqueue_tool_parts(controller);
        }
        // A parsable argument buffer can still be a prefix of a longer
        // input: finalize only when the stream flushes.
        self.tool_calls.finish(&mut self.tool_parts);
        for part in self.tool_parts.drain(..) {
            controller.enqueue(part);
        }

        let mut metadata = HashMap::new();
        metadata.insert(self.metadata_key.clone(), Map::new());

        prediction_tokens(
            self.final_usage_parsed.as_ref(),
            metadata.entry(self.metadata_key).or_default(),
        );

        if self.final_finish_reason.is_none() {
            controller.enqueue(StreamPart::Error {
                error: AiMuxError::InvalidResponseData(
                    "Response stream ended without a finish reason.".to_string(),
                ),
            });
        }
        controller.enqueue(StreamPart::Finish {
            finish_reason: self
                .final_finish_reason
                .unwrap_or_else(Self::error_finish_reason),
            usage: self.convert_usage.convert(self.final_usage_raw.as_ref()),
            provider_metadata: Some(metadata),
        });
    }
}

#[derive(Default)]
struct PendingToolCall {
    id: Option<String>,
    arguments: String,
    extra: Value,
}

/// Normalize text and thinking parts, preserving their order.
fn content_parts(content: Option<&Value>) -> Vec<GenerateContent> {
    let parts: Vec<(bool, String)> = match content {
        Some(Value::String(text)) if !text.is_empty() => vec![(false, text.clone())],
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                Some("text") => part
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|text| (false, text.to_string())),
                Some("thinking") => part
                    .get("thinking")
                    .and_then(Value::as_array)
                    .map(|chunks| {
                        (
                            true,
                            chunks
                                .iter()
                                .filter(|chunk| {
                                    chunk.get("type").and_then(Value::as_str) == Some("text")
                                })
                                .filter_map(|chunk| chunk.get("text").and_then(Value::as_str))
                                .collect::<String>(),
                        )
                    }),
                _ => None,
            })
            .filter(|(_, text)| !text.is_empty())
            .collect(),
        _ => Vec::new(),
    };
    parts
        .into_iter()
        .map(|(reasoning, text)| {
            if reasoning {
                GenerateContent::Reasoning(ReasoningOutput {
                    text,
                    provider_metadata: None,
                })
            } else {
                GenerateContent::Text {
                    text,
                    provider_metadata: None,
                }
            }
        })
        .collect()
}

/// One `GET {base_url}/models` exchange for discovery: no retry, no
/// recording.
pub(crate) async fn list_models_once(
    config: &CompatModelConfig,
) -> Result<Vec<aimux_core::model_catalogue::RuntimeModel>, AiMuxError> {
    #[derive(serde::Deserialize)]
    struct ModelsList {
        #[serde(default)]
        data: Vec<ModelEntry>,
    }
    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
        #[serde(default)]
        owned_by: Option<String>,
        #[serde(default)]
        created: Option<u64>,
    }

    let headers = config.request_headers(None).await?;
    let (url, origin) = config.url("/models")?;
    let resp = aimux_provider_utils::get_from_api(
        config.with_transport(
            aimux_provider_utils::HttpRequest {
                url,
                headers,
                ..Default::default()
            },
            origin,
        ),
        aimux_provider_utils::create_json_response_handler::<ModelsList>(),
        config.failed_response_handler(),
    )
    .await?;
    Ok(resp
        .value
        .data
        .into_iter()
        .map(|m| aimux_core::model_catalogue::RuntimeModel {
            id: m.id,
            owned_by: m.owned_by,
            created: m.created,
        })
        .collect())
}
