//! The OpenAI-compatible chat language model (`{name}.chat`,
//! `openai-compatible-chat-language-model.ts`).
//!
//! The Rust form of `OpenAICompatibleChatLanguageModel`, with its config
//! ([`ChatSettings`]) and `getArgs` ([`build_request_body`]). Vendor differences
//! arrive as [`ChatSettings`] data; the functions a caller or a vendor package
//! passes (`supportedUrls`, `convertUsage`, `errorStructure`,
//! `metadataExtractor`) are kept as given.
//!
//! The body is built in the AI SDK's order (model, `user`, sampling settings,
//! `response_format`, `stop`, `seed`, the pass-through fields of the provider's
//! own options namespace, then `reasoning_effort`, `verbosity`, `messages`,
//! `tools`, `tool_choice`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, Source, StreamResult};
use aimux_core::shared::{
    RequestInfo, ResponseInfo, SharedProviderMetadata, StreamResponseInfo, provider_namespace,
};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::RawToolCall;
use aimux_core::types::{FinishReason, Usage, Warning};
use aimux_provider_utils::{Transformer, generate_id, pipe_through};
use async_trait::async_trait;
use futures::future::BoxFuture;
use serde_json::{Map, Value, json};

use crate::openai_compatible::CompatModelConfig;
use crate::openai_compatible::error::{ErrorStructure, default_error_structure};
use crate::openai_compatible::utils::to_camel_case;
use crate::stream::{
    ChatToolCalls, Controller, Emit, FINISH_ERROR, FINISH_OTHER, StreamContext, ToolCallDelta,
};

use super::api_types::{ChatCompletionResponse, StreamChunk, UsageResponse};
use super::chat_language_model_options::{SCHEMA_KEYS, option_string, validate_options};
use super::convert_chat_usage::usage_from_raw;
use super::convert_to_chat_messages::{MessageSpec, convert_messages};
use super::map_finish_reason::parse_finish_reason;
use super::metadata_extractor::{MetadataExtractor, StreamMetadataExtractor};
use super::prepare_tools::prepare_function_tools;

/// Converts the raw optional token usage of a chat response.
pub type ConvertUsage = Arc<dyn Fn(Option<&Value>) -> Usage + Send + Sync>;

/// Returns the URLs supported by chat models, read each time a chat model's
/// supported URLs are: `supportedUrls?: () => LanguageModelV4['supportedUrls']`
/// (openai-compatible@3.0.62 `openai-compatible-chat-language-model.ts:84`,
/// `:161-163`), whose value may be a `PromiseLike`.
pub type SupportedUrlsFn = Arc<dyn Fn() -> BoxFuture<'static, SupportedUrls> + Send + Sync>;

/// The chat settings of a compatible endpoint: the AI SDK's
/// `includeUsage` / `supportsStructuredOutputs` /
/// `supportsMultiPartToolContent` / `supportedUrls` / `convertUsage` /
/// `metadataExtractor`, plus the preset capability data of RFC-0036 section
/// 5.
#[derive(Clone)]
pub(crate) struct ChatSettings {
    pub include_usage: bool,
    pub supports_structured_outputs: bool,
    /// When set, only these chat models take structured outputs, whatever
    /// `supports_structured_outputs` says. Ids compare exactly; an entry
    /// ending in `*` matches every id with that prefix. A preset's
    /// `profile.structured_output_models`, the vendor package's per-model
    /// `supportsStructuredOutputs`.
    pub structured_output_models: Option<&'static [String]>,
    /// `supportsMultiPartToolContent`: a `content` tool output is sent as an
    /// array of content parts instead of a JSON string.
    pub supports_multi_part_tool_content: bool,
    /// URL patterns supported by chat models, evaluated when requested.
    /// `None`: none.
    pub supported_urls: Option<SupportedUrlsFn>,
    /// The vendor's or the caller's usage accounting. `None`: the generic
    /// OpenAI-shaped usage (`convertOpenAICompatibleChatUsage`).
    pub convert_usage: Option<ConvertUsage>,
    pub metadata_extractor: Option<Arc<dyn MetadataExtractor>>,
    /// Send `top_k`. The AI SDK baseline never does (it warns and drops it).
    pub supports_top_k: bool,
    /// Send `tools` / `tool_choice`.
    pub supports_tools: bool,
    /// Send `response_format`.
    pub supports_response_format: bool,
    /// The only `max-tokens` key the vendor accepts: `"max_tokens"` or
    /// `"max_completion_tokens"`.
    pub max_tokens_key: Option<&'static str>,
    /// Streaming usage rides in `chunk[key].usage` instead of `chunk.usage`.
    pub stream_usage_key: Option<String>,
    pub error_structure: ErrorStructure,
}

impl Default for ChatSettings {
    /// The AI SDK baseline.
    fn default() -> Self {
        Self {
            include_usage: false,
            supports_structured_outputs: false,
            structured_output_models: None,
            supports_multi_part_tool_content: false,
            supported_urls: None,
            convert_usage: None,
            metadata_extractor: None,
            supports_top_k: false,
            supports_tools: true,
            supports_response_format: true,
            max_tokens_key: None,
            stream_usage_key: None,
            error_structure: Arc::new(default_error_structure),
        }
    }
}

impl ChatSettings {
    /// These settings for the chat model `model_id`.
    pub(crate) fn for_model(mut self, model_id: &str) -> Self {
        if let Some(models) = self.structured_output_models {
            self.supports_structured_outputs = models.iter().any(|model| {
                model
                    .strip_suffix('*')
                    .map_or(model == model_id, |prefix| model_id.starts_with(prefix))
            });
        }
        self
    }

    /// The URL patterns chat models fetch themselves.
    pub(crate) async fn supported_urls(&self) -> SupportedUrls {
        match &self.supported_urls {
            Some(supported_urls) => supported_urls().await,
            None => SupportedUrls::default(),
        }
    }

    /// The core usage of a chat response's raw `usage` object.
    pub(crate) fn convert_usage(&self, raw: Option<&Value>) -> Usage {
        match &self.convert_usage {
            Some(convert) => convert(raw),
            None => usage_from_raw(raw),
        }
    }
}

/// An OpenAI-compatible chat model. Holds no HTTP client: the request helpers
/// of `aimux-provider-utils` use the injected or process-default transport.
pub struct OpenAICompatibleChatModel {
    model_id: String,
    pub(in crate::openai_compatible) config: CompatModelConfig,
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

/// What the converter needs to know about the provider it builds for.
pub(crate) struct ChatBodySpec<'a> {
    /// First segment of the provider string: the providerOptions namespace.
    pub provider_options_name: &'a str,
    pub chat: &'a ChatSettings,
}

/// A request body and what building it produced.
#[derive(Debug, Clone)]
pub(crate) struct RequestBodyResult {
    /// The JSON body, before the provider's `transform_request_body`.
    pub body: Value,
    /// Warnings raised while building it.
    pub warnings: Vec<Warning>,
    /// The key provider metadata is reported under.
    pub metadata_key: String,
}

fn namespace<'a>(options: &'a CallOptions, key: &str) -> Option<&'a Map<String, Value>> {
    options
        .provider_options
        .as_ref()
        .and_then(|all| all.get(key))
}

/// The key provider metadata is reported under (`resolveProviderOptionsKey`):
/// the camelCase name when supplied, otherwise the raw provider name.
fn resolve_metadata_key(name: &str, options: &CallOptions) -> String {
    let camel = to_camel_case(name);
    if camel != name && namespace(options, &camel).is_some() {
        camel
    } else {
        name.to_string()
    }
}

/// Build the request body (`getArgs`).
///
/// # Errors
///
/// `InvalidArgument` for a malformed provider option or a prompt part that has
/// no wire form.
pub(crate) fn build_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
    spec: &ChatBodySpec<'_>,
) -> Result<RequestBodyResult, AiMuxError> {
    let name = spec.provider_options_name;
    let camel = to_camel_case(name);
    let chat = spec.chat;
    let mut warnings = Vec::new();

    // Resolved provider options: the generic namespace, then the provider's
    // own, then its camelCase form; later wins per key.
    let mut merged = Map::new();
    if let Some(deprecated) = namespace(options, "openai-compatible") {
        validate_options(deprecated, "openai-compatible")?;
        warnings.push(Warning::Deprecated {
            setting: "providerOptions key 'openai-compatible'".to_string(),
            message: "Use 'openaiCompatible' instead.".to_string(),
        });
        merged.extend(deprecated.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    for key in ["openaiCompatible", name, camel.as_str()] {
        if let Some(object) = namespace(options, key) {
            validate_options(object, key)?;
            merged.extend(object.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    if camel != name && namespace(options, name).is_some() {
        warnings.push(Warning::Deprecated {
            setting: format!("providerOptions key '{name}'"),
            message: format!("Use '{camel}' instead."),
        });
    }

    let user = option_string(&merged, name, "user")?;
    let reasoning_effort = option_string(&merged, name, "reasoningEffort")?.or_else(|| {
        options
            .reasoning
            .filter(|effort| effort.is_custom())
            .map(|effort| effort.to_string())
    });
    let verbosity = option_string(&merged, name, "textVerbosity")?;
    let strict_json_schema = merged
        .get("strictJsonSchema")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let metadata_key = resolve_metadata_key(name, options);
    let mut body = Map::new();
    body.insert("model".into(), json!(model_id));
    if let Some(user) = user {
        body.insert("user".into(), json!(user));
    }
    if let Some(max_tokens) = options.max_output_tokens {
        body.insert(
            chat.max_tokens_key.unwrap_or("max_tokens").into(),
            json!(max_tokens),
        );
    }
    for (key, value) in [
        ("temperature", options.temperature),
        ("top_p", options.top_p),
        ("frequency_penalty", options.frequency_penalty),
        ("presence_penalty", options.presence_penalty),
    ] {
        if let Some(value) = value {
            body.insert(key.into(), json!(value));
        }
    }
    if let Some(top_k) = options.top_k {
        if chat.supports_top_k {
            body.insert("top_k".into(), json!(top_k));
        } else {
            warnings.push(Warning::Unsupported {
                feature: "topK".to_string(),
                details: None,
            });
        }
    }

    match &options.response_format {
        Some(ResponseFormat::Json {
            schema,
            name: format_name,
            description,
        }) if chat.supports_response_format => match schema {
            Some(schema) if chat.supports_structured_outputs => {
                let mut json_schema = Map::new();
                json_schema.insert("schema".into(), schema.clone());
                json_schema.insert("strict".into(), json!(strict_json_schema));
                json_schema.insert(
                    "name".into(),
                    json!(format_name.clone().unwrap_or_else(|| "response".into())),
                );
                if let Some(description) = description {
                    json_schema.insert("description".into(), json!(description));
                }
                body.insert(
                    "response_format".into(),
                    json!({ "type": "json_schema", "json_schema": json_schema }),
                );
            }
            Some(_) => {
                warnings.push(Warning::Unsupported {
                    feature: "responseFormat".to_string(),
                    details: Some(
                        "JSON response format schema is only supported with structuredOutputs"
                            .to_string(),
                    ),
                });
                body.insert("response_format".into(), json!({ "type": "json_object" }));
            }
            None => {
                body.insert("response_format".into(), json!({ "type": "json_object" }));
            }
        },
        Some(ResponseFormat::Json { .. }) => warnings.push(Warning::Unsupported {
            feature: "responseFormat".to_string(),
            details: Some("response_format is not supported by this provider".to_string()),
        }),
        Some(ResponseFormat::Text) | None => {}
    }

    if let Some(stop) = &options.stop_sequences {
        body.insert("stop".into(), json!(stop));
    }
    if let Some(seed) = options.seed {
        body.insert("seed".into(), json!(seed));
    }

    // Fields of the provider's own namespace that the schema does not know go
    // to the body as given (the generic `openaiCompatible` namespace is
    // schema-only: unknown fields there are dropped).
    for key in [name, camel.as_str()] {
        if let Some(object) = namespace(options, key) {
            for (field, value) in object {
                if !SCHEMA_KEYS.contains(&field.as_str()) {
                    body.insert(field.clone(), value.clone());
                }
            }
        }
    }

    if let Some(effort) = reasoning_effort {
        body.insert("reasoning_effort".into(), json!(effort));
    }
    if let Some(verbosity) = verbosity {
        body.insert("verbosity".into(), json!(verbosity));
    }

    let messages = convert_messages(
        &options.prompt,
        &MessageSpec {
            metadata_key: &metadata_key,
            supports_multi_part_tool_content: chat.supports_multi_part_tool_content,
        },
    )?;
    body.insert("messages".into(), Value::Array(messages));

    if chat.supports_tools {
        let prepared = prepare_function_tools(options);
        warnings.extend(prepared.warnings);
        if let Some(tools) = prepared.tools {
            body.insert("tools".into(), Value::Array(tools));
        }
        if let Some(choice) = prepared.tool_choice {
            body.insert("tool_choice".into(), choice);
        }
    } else if options
        .tools
        .as_ref()
        .is_some_and(|tools| !tools.is_empty())
    {
        warnings.push(Warning::Unsupported {
            feature: "tools".to_string(),
            details: Some("tools are not supported by this provider".to_string()),
        });
    }

    if stream {
        body.insert("stream".into(), json!(true));
        if chat.include_usage {
            body.insert("stream_options".into(), json!({ "include_usage": true }));
        }
    }

    Ok(RequestBodyResult {
        body: Value::Object(body),
        warnings,
        metadata_key,
    })
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
pub(in crate::openai_compatible) fn stream_error(
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
    aimux_provider_utils::create_provider_stream_error(
        aimux_provider_utils::ProviderStreamError {
            message,
            provider_code,
            status_code: None,
            is_retryable: None,
            data: error.clone(),
        },
        error,
        url,
        request_body_values,
        response_headers,
    )
}

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
                headers: Some(response_headers),
                body: Some(raw),
                // `created || undefined`: some providers use 0 as a
                // placeholder (openai-compatible chat/get-response-metadata.ts).
                ..aimux_provider_utils::create_language_model_response_metadata(
                    data.id,
                    data.model,
                    data.created
                        .filter(|created| *created != 0)
                        .map(|secs| secs as f64),
                )
                .into()
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

        Ok(crate::stream::strict_guard(
            StreamResult {
                stream: Box::pin(stream),
                request: Some(RequestInfo { body: Some(body) }),
                response: Some(StreamResponseInfo {
                    headers: Some(response_headers),
                }),
            },
            options,
        ))
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
            controller.enqueue(StreamPart::ResponseMetadata(
                aimux_provider_utils::create_language_model_response_metadata(
                    chunk.id.clone(),
                    chunk.model.clone(),
                    chunk
                        .created
                        .filter(|created| *created != 0)
                        .map(|secs| secs as f64),
                ),
            ));
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
        if self.ctx.fail_without_terminal(
            self.final_finish_reason.is_some(),
            "a finish_reason",
            controller,
        ) {
            return;
        }
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
