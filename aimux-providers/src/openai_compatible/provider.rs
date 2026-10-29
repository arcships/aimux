//! The OpenAI-compatible provider (`openai-compatible-provider.ts`):
//! [`create_openai_compatible`], its settings, and the configuration every
//! model of the provider gets (the AI SDK's `getCommonModelConfig`).
//!
//! Like the native OpenAI config, [`CompatModelConfig`] is the Rust shape of the
//! object `createOpenAICompatible` hands each model class (`{ provider, url,
//! headers, fetch, ... }`). It has no getters for the credential or the settings
//! that produced it. Where the AI SDK passes `url` and `headers` functions, it
//! holds data, or a package's own data behind [`ResolveBaseUrl`] /
//! [`ResolveFetchHeaders`]. The shared model never compares a provider name.

use std::collections::HashMap;
use std::sync::Arc;

use aimux_core::ErrorCause;
use aimux_core::SourceError;
use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_provider_utils::{
    ExchangeContext, Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HeaderMapOpt,
    HttpRequest, Resolvable, combine_headers, normalize_headers, validate_base_url,
};
use futures::future::BoxFuture;
use serde_json::Value;

use crate::shared::aisdk_pins::Package;
use crate::shared::{AuthScheme, Credential, ProviderHeaders};

use super::chat::{ChatSettings, ConvertUsage, MetadataExtractor, SupportedUrlsFn};
use super::request_rewrite::CompatibleBodyRewrite;
use super::{
    OpenAICompatibleChatModel, OpenAICompatibleCompletionModel, OpenAICompatibleEmbeddingModel,
    OpenAICompatibleImageModel, TransformRequestBody,
};

/// Settings of [`create_openai_compatible`] (the AI SDK's
/// `OpenAICompatibleProviderSettings`).
#[derive(Clone, Default)]
pub struct OpenAICompatibleProviderSettings {
    /// The provider name: the first segment of every model's `provider()`
    /// string and the providerOptions namespace. Preserved as supplied.
    pub name: String,
    /// Base URL for the API calls. Required, `http(s)` with a host; a trailing
    /// slash is removed.
    pub base_url: String,
    /// The API key. `None` or `""` sends no `Authorization` header;
    /// a non-empty value is sent as `Bearer <value>`.
    pub api_key: Option<String>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the `Authorization` one. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// Query parameters appended to every request URL (sorted by name).
    pub query_params: Option<HashMap<String, String>>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Ask streaming responses to include usage (`stream_options.include_usage`).
    pub include_usage: Option<bool>,
    /// Whether the chat endpoint accepts `json_schema` response formats; when
    /// not, a schema degrades to `json_object` with a warning.
    pub supports_structured_outputs: Option<bool>,
    /// Whether chat tool results may carry multi-part content: a `content`
    /// tool output is then sent as an array of content parts instead of a
    /// JSON string. Default `false`.
    pub supports_multi_part_tool_content: Option<bool>,
    /// Rewrites each chat JSON request body once, after it is serialized and
    /// before it is sent.
    pub transform_request_body: Option<TransformRequestBody>,
    /// Extracts metadata from chat responses and streaming chunks.
    pub metadata_extractor: Option<Arc<dyn MetadataExtractor>>,
    /// URL patterns supported by chat models, evaluated when requested.
    pub supported_urls: Option<SupportedUrlsFn>,
    /// Custom chat token accounting.
    pub convert_usage: Option<ConvertUsage>,
}

impl std::fmt::Debug for OpenAICompatibleProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAICompatibleProviderSettings")
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.is_some())
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("query_params", &self.query_params)
            .field("fetch", &self.fetch.is_some())
            .field("include_usage", &self.include_usage)
            .field(
                "supports_structured_outputs",
                &self.supports_structured_outputs,
            )
            .field(
                "supports_multi_part_tool_content",
                &self.supports_multi_part_tool_content,
            )
            .field(
                "transform_request_body",
                &self.transform_request_body.is_some(),
            )
            .field("metadata_extractor", &self.metadata_extractor.is_some())
            .field("supported_urls", &self.supported_urls.is_some())
            .field("convert_usage", &self.convert_usage.is_some())
            .finish()
    }
}

/// Create an OpenAI-compatible provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
/// URL with a host. The key is not read
/// here.
pub fn create_openai_compatible(
    settings: OpenAICompatibleProviderSettings,
) -> Result<OpenAICompatibleProvider, AiMuxError> {
    let base_url = validate_base_url(&settings.base_url)?;
    let credential = settings
        .api_key
        .clone()
        .filter(|key| !key.is_empty())
        .map_or(Credential::None, |key| {
            Credential::Explicit(Resolvable::Value(key))
        });
    let headers = ProviderHeaders::bearer(credential, Vec::new(), settings.headers.clone());
    build(
        settings,
        BaseUrl::Fixed(base_url),
        headers,
        None,
        None,
        crate::shared::aisdk_pins::OPENAI_COMPATIBLE,
    )
}

/// [`create_openai_compatible`] for a package that builds on it with its own
/// base URL source and provider headers (the AI SDK's sub-providers that
/// call `createOpenAICompatible`). `settings.base_url`, `api_key` and
/// `headers` are not read; the user-agent part of `package` (the
/// openai-compatible version the calling package depends on) is added to
/// `headers`. `fetch_headers` go over the per-call headers, as headers a
/// package adds in its own fetch wrapper do. `body_rewrite` is the package's
/// own change to the chat request body, run before
/// `settings.transform_request_body`.
pub(crate) fn build(
    settings: OpenAICompatibleProviderSettings,
    base_url: BaseUrl,
    headers: ProviderHeaders,
    fetch_headers: Option<Arc<dyn ResolveFetchHeaders>>,
    body_rewrite: Option<CompatibleBodyRewrite>,
    package: Package,
) -> Result<OpenAICompatibleProvider, AiMuxError> {
    let defaults = ChatSettings::default();
    let mut provider = OpenAICompatibleProvider::assemble(Assembly {
        name: settings.name,
        base_url,
        credential: Credential::None,
        fixed_headers: Vec::new(),
        headers: None,
        query_params: settings.query_params,
        fetch: settings.fetch,
        body_rewrite,
        transform_request_body: settings.transform_request_body,
        package,
        chat: ChatSettings {
            include_usage: settings.include_usage.unwrap_or(false),
            supports_structured_outputs: settings.supports_structured_outputs.unwrap_or(false),
            supports_multi_part_tool_content: settings
                .supports_multi_part_tool_content
                .unwrap_or(false),
            supported_urls: settings.supported_urls,
            convert_usage: settings.convert_usage,
            metadata_extractor: settings.metadata_extractor,
            ..defaults
        },
    })?;
    provider.headers = headers.with_user_agent(package.name, package.version);
    provider.fetch_headers = fetch_headers;
    Ok(provider)
}

/// Everything a compatible provider is made of. The public factory fills it
/// from the settings; presets fill it from their registry row.
pub(crate) struct Assembly {
    pub name: String,
    pub base_url: BaseUrl,
    pub credential: Credential,
    /// Headers sent before the user's (organization, project, ...).
    pub fixed_headers: Vec<(String, String)>,
    pub headers: Option<HeaderMapOpt>,
    pub query_params: Option<HashMap<String, String>>,
    pub fetch: Option<FetchFunction>,
    /// A host package's own rewrite of the chat request body.
    pub body_rewrite: Option<CompatibleBodyRewrite>,
    /// The user's rewrite of the chat request body, after `body_rewrite`.
    pub transform_request_body: Option<TransformRequestBody>,
    pub chat: ChatSettings,
    /// The package the user-agent names: its `ai-sdk-{name}/{version}` part
    /// in the provider headers, and its provider-utils in the transport.
    pub package: Package,
}

/// An OpenAI-compatible provider (the AI SDK's `OpenAICompatibleProvider`).
/// Cheap to take models out of; it holds no HTTP client.
pub struct OpenAICompatibleProvider {
    name: String,
    base_url: BaseUrl,
    query_params: Option<Arc<Vec<(String, String)>>>,
    headers: ProviderHeaders,
    fetch_headers: Option<Arc<dyn ResolveFetchHeaders>>,
    fetch: Option<FetchFunction>,
    body_rewrite: Option<CompatibleBodyRewrite>,
    transform_request_body: Option<TransformRequestBody>,
    chat: ChatSettings,
}

impl OpenAICompatibleProvider {
    pub(crate) fn assemble(assembly: Assembly) -> Result<Self, AiMuxError> {
        let query_params = assembly.query_params.map(|params| {
            let mut pairs: Vec<(String, String)> = params.into_iter().collect();
            pairs.sort();
            Arc::new(pairs)
        });
        Ok(Self {
            name: assembly.name,
            base_url: assembly.base_url,
            query_params,
            headers: ProviderHeaders {
                credential: assembly.credential,
                scheme: AuthScheme::Bearer,
                fixed: assembly.fixed_headers,
                user: assembly.headers,
                user_agent: Some((assembly.package.name, assembly.package.version)),
            },
            fetch_headers: None,
            fetch: Some(crate::shared::package_fetch(
                assembly.fetch,
                assembly.package,
            )),
            body_rewrite: assembly.body_rewrite,
            transform_request_body: assembly.transform_request_body,
            chat: assembly.chat,
        })
    }

    fn model_config(&self, method: &str) -> CompatModelConfig {
        CompatModelConfig {
            provider: format!("{}.{method}", self.name),
            base_url: self.base_url.clone(),
            query_params: self.query_params.clone(),
            headers: self.headers.clone(),
            fetch_headers: self.fetch_headers.clone(),
            fetch: self.fetch.clone(),
            body_rewrite: self.body_rewrite.filter(|_| method == "chat"),
            transform_request_body: if method == "chat" {
                self.transform_request_body.clone()
            } else {
                None
            },
            chat: self.chat.clone(),
        }
    }

    /// A chat-completions model; `provider()` is `"{name}.chat"`.
    #[must_use]
    pub fn chat(&self, model_id: &str) -> OpenAICompatibleChatModel {
        let mut config = self.model_config("chat");
        config.chat = config.chat.for_model(model_id);
        OpenAICompatibleChatModel::from_config(model_id.to_string(), config)
    }

    /// A text-completion model; `provider()` is `"{name}.completion"`.
    #[must_use]
    pub fn completion(&self, model_id: &str) -> OpenAICompatibleCompletionModel {
        OpenAICompatibleCompletionModel::from_config(
            model_id.to_string(),
            self.model_config("completion"),
        )
    }

    /// An embedding model; `provider()` is `"{name}.embedding"`.
    #[must_use]
    pub fn embedding(&self, model_id: &str) -> OpenAICompatibleEmbeddingModel {
        OpenAICompatibleEmbeddingModel::from_config(
            model_id.to_string(),
            self.model_config("embedding"),
        )
    }

    /// An image model; `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> OpenAICompatibleImageModel {
        OpenAICompatibleImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }

    /// The provider as a function: the default language model for an id, the
    /// chat model. The AI SDK's callable provider.
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.chat(model_id))
    }
}

impl Provider for OpenAICompatibleProvider {
    fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
        Some(self)
    }

    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Ok(Arc::new(self.embedding(model_id)))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Ok(Arc::new(self.image(model_id)))
    }

    /// Native decisions of the official runtimes (Ollama, llama.cpp, LocalAI,
    /// Laya, vLLM, SGLang, Cloudflare); any other preset offers none.
    fn decision_model(
        &self,
        model_id: &str,
    ) -> Option<Result<Arc<dyn aimux_core::DecisionModel>, AiMuxError>> {
        if !crate::runtime_decision::supported(&self.name) {
            return None;
        }
        Some(
            crate::runtime_decision::RuntimeDecisionModel::new(
                &self.name,
                model_id,
                Arc::new(self.model_config("decision")),
            )
            .map(|model| Arc::new(model) as Arc<dyn aimux_core::DecisionModel>),
        )
    }
}

impl ProviderDiscovery for OpenAICompatibleProvider {
    /// `GET {base_url}/models`: one exchange, no retry. Without a key no
    /// `Authorization` header is sent.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.model_config("models");
        Box::pin(async move { list_models_once(&config).await })
    }
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
/// A plain provider has a [`Fixed`](Self::Fixed) URL. A URL read from the
/// environment or built from settings (a preset's template, a Vertex AI
/// partner's project and location) is [`Dynamic`](Self::Dynamic): it is
/// evaluated per request, so creating the provider never reads the
/// environment and never fails.
#[derive(Clone, Debug)]
pub(crate) enum BaseUrl {
    Fixed(String),
    /// A package's own base URL, resolved when a request is made.
    Dynamic(Arc<dyn ResolveBaseUrl>),
}

impl BaseUrl {
    pub(crate) fn resolve(&self) -> Result<String, AiMuxError> {
        match self {
            Self::Fixed(url) => Ok(url.clone()),
            Self::Dynamic(resolver) => resolver.resolve(),
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
    /// The host package's rewrite of the request body (chat only).
    pub body_rewrite: Option<CompatibleBodyRewrite>,
    /// The user's rewrite of the request body (chat only), after
    /// `body_rewrite`.
    pub transform_request_body: Option<TransformRequestBody>,
    pub chat: ChatSettings,
}

/// The decision endpoint of a runtime preset: the base URL, credential and
/// transport of its other models.
#[async_trait::async_trait]
impl crate::decision_support::DecisionEndpoint for CompatModelConfig {
    fn url(&self, path: &str) -> Result<String, AiMuxError> {
        Ok(format!(
            "{}{}",
            self.base_url.resolve()?.trim_end_matches('/'),
            path
        ))
    }

    async fn request(
        &self,
        url: &str,
        options: &aimux_core::decision_model::DecisionCallOptions,
    ) -> Result<HttpRequest, AiMuxError> {
        let headers = self.request_headers(options.headers.as_ref()).await?;
        let origin = self.base_url.resolve()?;
        Ok(self.with_transport(
            HttpRequest::new(url, headers.list, options),
            origin,
            headers.fetch_user_agent,
        ))
    }
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

    /// Run the host's body rewrite, then the user's, when there are any.
    pub(crate) fn transform_body(&self, body: Value) -> Value {
        let body = match self.body_rewrite {
            Some(rewrite) => rewrite.apply(body),
            None => body,
        };
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
