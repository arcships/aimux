//! The private per-model configuration of the OpenAI-compatible package.
//!
//! Like the native OpenAI config this is the Rust shape of the object
//! `createOpenAICompatible` hands each model class (`{ provider, url, headers,
//! fetch, ... }`). It has no getters for the credential or the settings that
//! produced it. Where the AI SDK passes `url` and `headers` functions, this
//! config holds data, or a package's own data behind [`ResolveBaseUrl`] /
//! [`ResolveFetchHeaders`]; the functions a caller or a vendor package passes
//! (`supportedUrls`, `convertUsage`, `errorStructure`, `metadataExtractor`,
//! `transformRequestBody`) are kept as given. The shared model never compares
//! a provider name.

use aimux_core::SourceError;
use futures::future::BoxFuture;
use std::collections::HashMap;
use std::sync::Arc;

use aimux_core::types::{ProviderMetadata, Usage};
use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_core::ErrorCause;
use aimux_core::language_model::SupportedUrls;
use aimux_provider_utils::{
    ExchangeContext, Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HeaderMapOpt,
    HttpRequest, ProviderErrorParts, combine_headers, normalize_headers,
};

use crate::preset::PresetDescriptor;
use crate::shared::ProviderHeaders;

pub use crate::shared::TransformRequestBody;

/// Converts the raw optional token usage of a chat response.
pub type ConvertUsage = Arc<dyn Fn(Option<&Value>) -> Usage + Send + Sync>;

/// Returns the URLs supported by chat models, read each time a chat model's
/// supported URLs are: `supportedUrls?: () => LanguageModelV4['supportedUrls']`
/// (openai-compatible@3.0.62 `openai-compatible-chat-language-model.ts:84`,
/// `:161-163`), whose value may be a `PromiseLike`.
pub type SupportedUrlsFn = Arc<dyn Fn() -> BoxFuture<'static, SupportedUrls> + Send + Sync>;

/// Captures vendor-specific metadata from responses (the AI SDK's
/// `MetadataExtractor`). The returned map is merged into `provider_metadata`
/// next to the namespace entry.
pub trait MetadataExtractor: Send + Sync {
    /// Metadata of a non-streaming response body.
    fn extract_metadata(&self, parsed_body: &Value) -> Option<ProviderMetadata>;

    /// A fresh extractor for one streaming response.
    fn create_stream_extractor(&self) -> Box<dyn StreamMetadataExtractor>;
}

/// The per-stream half of [`MetadataExtractor`].
pub trait StreamMetadataExtractor: Send {
    /// Called with every parsed chunk (including the one carrying usage).
    fn process_chunk(&mut self, parsed_chunk: &Value);

    /// Metadata merged into the finish part's `provider_metadata`.
    fn build_metadata(&self) -> Option<ProviderMetadata>;
}

/// A base URL a package builds from settings read when a request is made
/// (a Vertex AI partner sub-provider's project and location).
pub(crate) trait ResolveBaseUrl: std::fmt::Debug + Send + Sync {
    /// The base URL of one request.
    fn resolve(&self) -> Result<String, AiMuxError>;
}

/// The headers a package adds in its own fetch wrapper
/// ([`CompatModelConfig::fetch_headers`]).
#[async_trait::async_trait]
pub(crate) trait ResolveFetchHeaders: Send + Sync {
    /// The headers of one request.
    async fn resolve(&self) -> Result<HeaderMapOpt, AiMuxError>;
}

/// Where the base URL comes from.
///
/// A plain provider has a [`Fixed`](Self::Fixed) URL. A preset whose URL is
/// read from the environment or expanded from template parameters is
/// [`Preset`](Self::Preset): it is evaluated per request, so creating the
/// provider (and the preset default instances) never reads the environment
/// and never fails.
#[derive(Clone, Debug)]
pub(crate) enum BaseUrl {
    Fixed(String),
    Preset {
        descriptor: &'static PresetDescriptor,
        /// The explicit template parameters of the settings.
        params: Arc<HashMap<String, String>>,
    },
    /// A package's own base URL, resolved when a request is made.
    Dynamic(Arc<dyn ResolveBaseUrl>),
}

impl BaseUrl {
    pub(crate) fn resolve(&self) -> Result<String, AiMuxError> {
        match self {
            Self::Fixed(url) => Ok(url.clone()),
            Self::Preset { descriptor, params } => {
                crate::preset::resolve_base_url(descriptor, params)
            }
            Self::Dynamic(resolver) => resolver.resolve(),
        }
    }
}

/// Maps a provider error payload to the message and code of the API error
/// (the AI SDK's `errorStructure`).
pub(crate) type ErrorStructure = Arc<dyn Fn(&Value) -> ProviderErrorParts + Send + Sync>;

/// `{ error: { message, type?, param?, code? } }`, the structure nearly every
/// compatible server answers errors with (the AI SDK's default
/// `errorStructure`).
pub(crate) fn default_error_structure(data: &Value) -> ProviderErrorParts {
    let error = data.get("error").unwrap_or(data);
    ProviderErrorParts {
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        provider_code: error.get("code").or_else(|| error.get("type")).and_then(
            |value| match value {
                Value::String(value) => Some(value.clone()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            },
        ),
    }
}

/// The chat settings of a compatible endpoint: the AI SDK's
/// `includeUsage` / `supportsStructuredOutputs` /
/// `supportsMultiPartToolContent` / `supportedUrls` / `convertUsage` /
/// `metadataExtractor`, plus the preset capability data of RFC-0036 section
/// 5.
#[derive(Clone)]
pub(crate) struct ChatSettings {
    pub include_usage: bool,
    pub supports_structured_outputs: bool,
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
            None => super::chat::usage_from_raw(raw),
        }
    }
}

/// What a model needs to talk to the API.
#[derive(Clone)]
pub(crate) struct CompatModelConfig {
    /// The identity the model reports: `"{name}.{method}"`.
    pub provider: String,
    pub base_url: BaseUrl,
    /// Appended to every request URL, in this order.
    pub query_params: Option<Arc<Vec<(String, String)>>>,
    /// Provider headers (credential, user headers), produced on every request.
    pub headers: ProviderHeaders,
    /// Headers layered over the per-call headers, resolved on every request
    /// after `headers`: what an AI SDK package that builds on
    /// `createOpenAICompatible` adds in its own fetch wrapper (google-vertex's
    /// Node MaaS and xAI providers). Their `user-agent` replaces the whole
    /// header, the helper layer's part included ([`FetchUserAgent`]).
    pub fetch_headers: Option<Arc<dyn ResolveFetchHeaders>>,
    /// Transport; `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    pub transform_request_body: Option<TransformRequestBody>,
    pub chat: ChatSettings,
}

impl CompatModelConfig {
    /// The providerOptions namespace of this provider: the first segment of
    /// the provider string.
    pub(crate) fn provider_options_name(&self) -> &str {
        self.provider.split('.').next().unwrap_or_default().trim()
    }

    /// The full URL of an endpoint path (with the query parameters) and the
    /// origin credentialed headers may be sent to.
    pub(crate) fn url(&self, path: &str) -> Result<(String, String), AiMuxError> {
        let base = self.base_url.resolve()?;
        let mut url =
            url::Url::parse(&format!("{base}{path}")).map_err(|e| AiMuxError::InvalidArgument {
                message: format!("invalid request URL for {}: {e}", self.provider),
                cause: Some(ErrorCause::source(&e)),
            })?;
        if let Some(params) = &self.query_params {
            // As the AI SDK does: the configured parameters replace whatever
            // query the base URL carried.
            url.set_query(None);
            if !params.is_empty() {
                url.query_pairs_mut()
                    .extend_pairs(params.iter().map(|(k, v)| (k.as_str(), v.as_str())));
            }
        }
        Ok((url.to_string(), base))
    }

    /// Resolve the provider headers, layer the per-call headers over them,
    /// then the fetch headers (case-insensitive, later wins, `None` removes),
    /// and return the list that goes on the wire with the fetch headers'
    /// `user-agent`.
    pub(crate) async fn request_headers(
        &self,
        call_headers: Option<&HashMap<String, String>>,
    ) -> Result<RequestHeaders, AiMuxError> {
        let provider = self.headers.resolve().await?;
        let call: HeaderMapOpt = call_headers
            .map(|headers| {
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), Some(value.clone())))
                    .collect()
            })
            .unwrap_or_default();
        let fetch = match &self.fetch_headers {
            Some(headers) => combine_headers(&[&headers.resolve().await?]),
            None => HeaderMapOpt::new(),
        };
        let fetch_user_agent = fetch.get("user-agent").cloned().flatten();
        Ok(RequestHeaders {
            list: normalize_headers(combine_headers(&[&provider, &call, &fetch])),
            fetch_user_agent,
        })
    }

    /// An exchange for `path` that inherits the operation's cancellation and
    /// recording context and carries this config's transport and credential
    /// origin.
    pub(crate) fn http_request(
        &self,
        path: &str,
        headers: RequestHeaders,
        options: &impl ExchangeContext,
    ) -> Result<HttpRequest, AiMuxError> {
        let (url, origin) = self.url(path)?;
        Ok(self.with_transport(
            HttpRequest::new(url, headers.list, options),
            origin,
            headers.fetch_user_agent,
        ))
    }

    /// Attach the transport and the credential origin to a request. A fetch
    /// headers' `user-agent` is sent as the whole header by the transport.
    pub(crate) fn with_transport(
        &self,
        mut request: HttpRequest,
        origin: String,
        fetch_user_agent: Option<String>,
    ) -> HttpRequest {
        request.fetch = match fetch_user_agent {
            Some(user_agent) => Some(Arc::new(FetchUserAgent {
                inner: self.fetch.clone(),
                user_agent,
            })),
            None => self.fetch.clone(),
        };
        request.credentialed_origin = Some(origin);
        request
    }

    /// Run the provider-level body rewrite, when there is one.
    pub(crate) fn transform_body(&self, body: Value) -> Value {
        match &self.transform_request_body {
            Some(transform) => transform(body),
            None => body,
        }
    }

    /// The failed-response handler of this provider's error structure.
    pub(crate) fn failed_response_handler(
        &self,
    ) -> aimux_provider_utils::ResponseHandler<AiMuxError> {
        let structure = self.chat.error_structure.clone();
        aimux_provider_utils::create_json_error_response_handler(move |data| structure(data))
    }
}

/// The headers of one request ([`CompatModelConfig::request_headers`]).
pub(crate) struct RequestHeaders {
    /// What goes on the wire, before the helper layer adds its
    /// `ai-sdk-provider-utils` part.
    pub list: Vec<(String, String)>,
    /// The `user-agent` of the fetch headers, when they set one.
    pub fetch_user_agent: Option<String>,
}

/// The transport of a request whose fetch headers set `user-agent`: it sends
/// that value as the whole header, over the one the helper layer and the
/// package transport built. google-vertex's Node wrappers spread their
/// headers over `init.headers` after `postToApi` added its part
/// (google-vertex-maas-provider-node.ts:43-49,
/// google-vertex-xai-provider-node.ts:41-47), so nothing of it is left.
struct FetchUserAgent {
    /// The package transport (`None`: the process default).
    inner: Option<FetchFunction>,
    user_agent: String,
}

#[async_trait::async_trait]
impl Fetch for FetchUserAgent {
    async fn fetch(&self, mut request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let value =
            reqwest::header::HeaderValue::try_from(self.user_agent.as_str()).map_err(|error| {
                FetchError::Other(SourceError::with_cause(
                    "invalid header value for user-agent",
                    &error,
                ))
            })?;
        request.headers.insert(reqwest::header::USER_AGENT, value);
        self.inner
            .clone()
            .unwrap_or_else(aimux_provider_utils::default_fetch)
            .fetch(request)
            .await
    }
}
