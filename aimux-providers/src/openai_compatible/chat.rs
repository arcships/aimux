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
    RequestInfo, ResponseInfo, SharedProviderMetadata, StreamResponseInfo, provider_namespace,
};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, InputTokenUsage, OutputTokenUsage, ResponseMetadata, Usage};
use aimux_provider_utils::{Transformer, generate_id, pipe_through};

use super::config::{ChatSettings, CompatModelConfig, StreamMetadataExtractor};
use super::convert::{ChatBodySpec, RequestBodyResult, build_request_body, parse_finish_reason};
use super::types::{ChatCompletionResponse, StreamChunk, UsageResponse};
use crate::stream::{
    ChatToolCalls, Controller, Emit, FINISH_ERROR, FINISH_OTHER, StreamContext, ToolCallDelta,
};

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

    /// The JSON body a call would send, before the provider's
    /// `transform_request_body`, with the warnings raised while building it.
    /// Nothing is read from the environment and nothing is sent: a key is not
    /// resolved here.
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

/// The error of an `{"error": ...}` chunk. Upstream passes the chunk's
/// `error` value on as the error part as it is
/// (openai-compatible-chat-language-model.ts:588-595,
/// openai-compatible-completion-language-model.ts:320-325): no status is
/// read from its `code`, numeric or not, so nothing makes it retryable.
pub(super) fn stream_error(
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
    aimux_provider_utils::stream_error_api_call(
        message,
        provider_code,
        None,
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

    async fn supported_urls(&self) -> SupportedUrls {
        self.config.chat.supported_urls().await
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let built = self.request_body(options, false)?;
        let metadata_key = built.metadata_key;
        let body = self.config.transform_body(built.body);
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
            AiMuxError::invalid_response_data("Response did not contain any choices.")
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
            .unwrap_or(FINISH_OTHER);

        let chat = &self.config.chat;
        let usage = chat.convert_usage(raw.get("usage"));

        let mut metadata = aimux_core::shared::SharedProviderMetadata::new();
        metadata.insert(metadata_key.clone(), Map::new());
        if let Some(extractor) = &chat.metadata_extractor
            && let Some(extra) = extractor.extract_metadata(&raw)
        {
            for (key, value) in extra {
                metadata.insert(key, value);
            }
        }

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
                    .and_then(|secs| aimux_core::Timestamp::from_unix_seconds(secs as f64)),
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
        let body = self.config.transform_body(built.body);
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
            ctx: StreamContext::new(options, warnings, endpoint, &body, &response_headers),
            metadata_key,
            metadata_extractor: self
                .config
                .chat
                .metadata_extractor
                .as_ref()
                .map(|extractor| extractor.create_stream_extractor()),
            chat: self.config.chat.clone(),
            ..OpenAICompatibleChatStream::default()
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
#[derive(Default)]
struct OpenAICompatibleChatStream {
    ctx: StreamContext,
    metadata_key: String,
    /// The chat settings: usage accounting and the streaming usage key.
    chat: ChatSettings,
    /// The caller's extractor, fed every parsed chunk.
    metadata_extractor: Option<Box<dyn StreamMetadataExtractor>>,

    text_started: bool,
    reasoning_started: bool,
    final_usage_raw: Option<Value>,
    final_usage_parsed: Option<UsageResponse>,
    final_finish_reason: Option<FinishReason>,
    response_metadata_emitted: bool,
    tool_calls: ChatToolCalls,
    /// Some compatible servers send the first delta of a call without
    /// `function.name`; buffer by index until the name is known.
    pending: HashMap<usize, PendingToolCall>,
    forwarded: HashSet<usize>,
}

/// A chat stream with an empty request context and tool-call ids from
/// `generate_id`, for tests that drive it chunk by chunk.
/// `metadata_key` is the provider options key.
#[cfg(test)]
pub(crate) fn chat_test_stream(
    metadata_key: &str,
    generate_id: std::sync::Arc<dyn Fn() -> String + Send + Sync>,
) -> impl Transformer<Input = Result<Value, AiMuxError>, Output = StreamPart> {
    OpenAICompatibleChatStream {
        metadata_key: metadata_key.to_string(),
        tool_calls: ChatToolCalls::new(generate_id),
        ..OpenAICompatibleChatStream::default()
    }
}

impl OpenAICompatibleChatStream {
    fn signature_metadata(&self, extra: Option<&Value>) -> Option<SharedProviderMetadata> {
        thought_signature(extra).map(|signature| {
            provider_namespace(&self.metadata_key, json!({ "thoughtSignature": signature }))
                .expect("provider metadata must be an object")
        })
    }
}

impl Transformer for OpenAICompatibleChatStream {
    type Input = Result<Value, AiMuxError>;
    type Output = StreamPart;

    fn start(&mut self, controller: &mut Controller) {
        self.ctx.start(controller);
    }

    fn transform(&mut self, event: Result<Value, AiMuxError>, controller: &mut Controller) {
        let parsed = match event {
            Ok(parsed) => parsed,
            Err(error) => {
                self.final_finish_reason = Some(FINISH_ERROR);
                controller.enqueue(StreamPart::Error { error });
                return;
            }
        };
        self.ctx.raw(&parsed, controller);
        if let Some(extractor) = self.metadata_extractor.as_mut() {
            extractor.process_chunk(&parsed);
        }
        if let Some(error) = parsed.get("error") {
            controller.enqueue(StreamPart::Error {
                error: stream_error(
                    &self.metadata_key,
                    error,
                    &self.ctx.url,
                    self.ctx.request_body.clone(),
                    self.ctx.response_headers.clone(),
                ),
            });
            self.final_finish_reason = Some(FINISH_ERROR);
            return;
        }

        let chunk_usage_raw: Option<Value> = match &self.chat.stream_usage_key {
            Some(key) => parsed.get(key).and_then(|v| v.get("usage")).cloned(),
            None => parsed.get("usage").cloned(),
        }
        .filter(|usage| !usage.is_null());

        let chunk: StreamChunk = match serde_json::from_value(parsed) {
            Ok(chunk) => chunk,
            Err(e) => {
                self.final_finish_reason = Some(FINISH_ERROR);
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
                    .and_then(|secs| aimux_core::Timestamp::from_unix_seconds(secs as f64)),
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
                            controller.text_end(TEXT_ID);
                            self.text_started = false;
                        }
                        if !self.reasoning_started {
                            self.reasoning_started = true;
                            controller.reasoning_start(REASONING_ID);
                        }
                        controller.reasoning_delta(REASONING_ID, text);
                    }
                    GenerateContent::Text { text, .. } => {
                        if self.reasoning_started {
                            controller.reasoning_end(REASONING_ID);
                            self.reasoning_started = false;
                        }
                        if !self.text_started {
                            self.text_started = true;
                            controller.text_start(TEXT_ID);
                        }
                        controller.text_delta(TEXT_ID, text);
                    }
                    _ => unreachable!(),
                }
            }

            if let Some(deltas) = choice.delta.tool_calls.filter(|deltas| !deltas.is_empty()) {
                if self.reasoning_started {
                    controller.reasoning_end(REASONING_ID);
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
                            ToolCallDelta {
                                index: Some(index),
                                id: buffered.id.as_deref(),
                                name: Some(name),
                                arguments: Some(&buffered.arguments),
                                provider_metadata: self.signature_metadata(Some(&buffered.extra)),
                                ..ToolCallDelta::default()
                            }
                        }
                        _ => ToolCallDelta {
                            index: dtc.index,
                            id: dtc.id.as_deref(),
                            name: function.and_then(|f| f.name.as_deref()),
                            arguments,
                            provider_metadata: self.signature_metadata(dtc.extra_content.as_ref()),
                            ..ToolCallDelta::default()
                        },
                    };
                    if !self.tool_calls.delta(delta, controller) {
                        return;
                    }
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

    fn flush(mut self, controller: &mut Controller) {
        if self.reasoning_started {
            controller.reasoning_end(REASONING_ID);
        }
        if self.text_started {
            controller.text_end(TEXT_ID);
        }
        for (index, entry) in std::mem::take(&mut self.pending) {
            let delta = ToolCallDelta {
                index: Some(index),
                id: entry.id.as_deref(),
                arguments: Some(&entry.arguments),
                provider_metadata: self.signature_metadata(Some(&entry.extra)),
                ..ToolCallDelta::default()
            };
            if !self.tool_calls.delta(delta, controller) {
                return;
            }
        }
        self.tool_calls.finish(controller);

        let mut metadata = aimux_core::shared::SharedProviderMetadata::new();
        metadata.insert(self.metadata_key.clone(), Map::new());
        if let Some(extractor) = &self.metadata_extractor
            && let Some(extra) = extractor.build_metadata()
        {
            for (key, value) in extra {
                metadata.insert(key, value);
            }
        }

        prediction_tokens(
            self.final_usage_parsed.as_ref(),
            metadata.entry(self.metadata_key).or_default(),
        );

        if self.final_finish_reason.is_none() {
            controller.enqueue(StreamPart::Error {
                error: AiMuxError::invalid_response_data(
                    "Response stream ended without a finish reason.".to_string(),
                ),
            });
        }
        controller.enqueue(StreamPart::Finish {
            finish_reason: self.final_finish_reason.unwrap_or(FINISH_ERROR),
            usage: self.chat.convert_usage(self.final_usage_raw.as_ref()),
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
                headers: headers.list,
                ..Default::default()
            },
            origin,
            headers.fetch_user_agent,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use aimux_core::error::AiMuxError;
    use aimux_core::language_model::LanguageModel;
    use aimux_core::language_model_message::LanguageModelMessage;
    use aimux_core::options::CallOptions;
    use aimux_provider_utils::ProviderErrorParts;

    use crate::openai_compatible::config::{BaseUrl, ChatSettings};
    use crate::openai_compatible::{Assembly, OpenAICompatibleProvider};
    use crate::shared::Credential;

    fn hello() -> CallOptions {
        CallOptions::new(vec![LanguageModelMessage::user_text("hi")])
    }

    fn provider(base_url: BaseUrl, chat: ChatSettings) -> OpenAICompatibleProvider {
        OpenAICompatibleProvider::assemble(Assembly {
            name: "acme".to_string(),
            base_url,
            credential: Credential::None,
            fixed_headers: Vec::new(),
            headers: None,
            query_params: None,
            fetch: None,
            transform_request_body: None,
            chat,
            package: crate::shared::aisdk_pins::OPENAI_COMPATIBLE,
        })
        .unwrap()
    }

    async fn serve(server: &MockServer, response: ResponseTemplate) {
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(response)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn an_error_structure_decides_the_message_and_code() {
        let server = MockServer::start().await;
        serve(
            &server,
            ResponseTemplate::new(400).set_body_json(json!({"detail": "bad prompt", "kind": 42})),
        )
        .await;
        let chat = ChatSettings {
            error_structure: Arc::new(|data: &Value| ProviderErrorParts {
                message: data["detail"].as_str().unwrap_or_default().to_string(),
                provider_code: data["kind"].as_u64().map(|k| k.to_string()),
            }),
            ..ChatSettings::default()
        };
        let error = provider(BaseUrl::Fixed(server.uri()), chat)
            .chat("m")
            .do_generate(&hello())
            .await
            .unwrap_err();
        match error {
            AiMuxError::ApiCall(detail) => {
                assert_eq!(detail.message, "bad prompt");
                assert_eq!(detail.provider_code.as_deref(), Some("42"));
            }
            other => panic!("expected ApiCall, got {other:?}"),
        }
    }

    /// A preset base URL is data the request resolves: creating the provider
    /// and the model reads nothing, a request that cannot resolve it fails,
    /// and the next request resolves it again.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_lazy_base_url_is_evaluated_per_request_and_its_error_fails_the_request() {
        use crate::preset::{AuthMode, ParamSpec, PresetDescriptor};

        const VAR: &str = "AIMUX_TEST_COMPAT_LAZY_BASE_URL";
        let server = MockServer::start().await;
        serve(
            &server,
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "c", "model": "m",
                "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}]
            })),
        )
        .await;
        // SAFETY: the test is `#[serial]` and no other test reads `VAR`.
        unsafe { std::env::remove_var(VAR) };
        let descriptor: &'static PresetDescriptor = Box::leak(Box::new(PresetDescriptor {
            name: "acme".to_string(),
            display: "Acme".to_string(),
            base_url: "http://{host}/v1".to_string(),
            env_var: String::new(),
            auth: AuthMode::None,
            base_url_env: Some(VAR.to_string()),
            max_tokens_key: None,
            convert_usage: None,
            params: vec![ParamSpec {
                name: "host".to_string(),
                env: Vec::new(),
                default: None,
                derive: None,
            }],
            user_agent: ("openai-compatible", "3.0.62"),
            structured_output_models: None,
        }));
        let lazy = BaseUrl::Preset {
            descriptor,
            params: Arc::new(std::collections::HashMap::new()),
        };
        let provider = provider(lazy, ChatSettings::default());
        let model = provider.chat("m");
        let error = model.do_generate(&hello()).await.unwrap_err();
        assert!(
            matches!(error, AiMuxError::InvalidArgument { .. }),
            "{error:?}"
        );
        // SAFETY: as above.
        unsafe { std::env::set_var(VAR, server.uri()) };
        let result = model.do_generate(&hello()).await;
        // SAFETY: as above.
        unsafe { std::env::remove_var(VAR) };
        result.expect("resolved on the next request");
    }
}
