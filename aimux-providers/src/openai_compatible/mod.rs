//! OpenAI-compatible provider.
//!
//! [`create_openai_compatible`] is the Rust form of the AI SDK's
//! `createOpenAICompatible`: it takes [`OpenAICompatibleProviderSettings`],
//! validates the base URL and returns an [`OpenAICompatibleProvider`]
//! whose models speak the chat-completions, embeddings and image endpoints of
//! any OpenAI-compatible server.
//!
//! What is fixed when the provider is created, and what is not:
//!
//! - The explicit settings (`name`, `base_url`, `headers`, `query_params`,
//!   `fetch`, the capability flags, `transform_request_body`) are fixed in the
//!   factory.
//! - A non-empty `api_key` produces a bearer header. `None` or `""` sends
//!   no `Authorization` header, which is what a local server wants.
//!
//! Identity: every model reports `"{name}.{method}"` (`groq.chat`,
//! `local.embedding`, ...). The providerOptions namespace is the same name:
//! options are read from `openaiCompatible`, then the name, then its camelCase
//! form (later wins), fields of the name's own namespace that the generic
//! schema does not know are passed through to the body, and provider metadata
//! is reported under the name.
//!
//! Vendor packages ([`groq`](crate::groq), [`deepseek`](crate::deepseek)) and the
//! registry presets ([`preset`](crate::preset)) are built on the same
//! internals; the shared model never branches on a vendor name.

pub(crate) mod chat;
pub mod completion;
pub(crate) mod config;
pub(crate) mod convert;
pub mod embedding;
pub mod image;
mod types;

pub use chat::OpenAICompatibleChatModel;
pub use completion::OpenAICompatibleCompletionModel;
pub use config::{
    ConvertUsage, MetadataExtractor, StreamMetadataExtractor, SupportedUrlsFn, TransformRequestBody,
};
pub use embedding::OpenAICompatibleEmbeddingModel;
pub use image::OpenAICompatibleImageModel;

use std::collections::HashMap;
use std::sync::Arc;

use futures::future::BoxFuture;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_provider_utils::{
    FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, combine_headers, validate_base_url,
};

use crate::shared::{Credential, provider_headers};
use config::{BaseUrl, ChatDialect, ChatSettings, CompatModelConfig};

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
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
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
    let user_headers = settings.headers.clone();
    let mut provider = OpenAICompatibleProvider::assemble(Assembly {
        name: settings.name,
        base_url: BaseUrl::Fixed(base_url),
        credential: credential.clone(),
        fixed_headers: Vec::new(),
        headers: settings.headers,
        query_params: settings.query_params,
        fetch: settings.fetch,
        transform_request_body: settings.transform_request_body,
        profile: ChatProfile {
            include_usage: settings.include_usage.unwrap_or(false),
            supports_structured_outputs: settings.supports_structured_outputs.unwrap_or(false),
            dialect: ChatDialect::baseline(),
        },
    })?;
    let dialect = Arc::make_mut(&mut provider.chat.dialect);
    dialect.metadata_extractor = settings.metadata_extractor;
    dialect.supported_urls = settings.supported_urls;
    dialect.convert_usage = settings.convert_usage;
    let headers = compatible_headers(credential, user_headers);
    provider.headers = aimux_provider_utils::headers::with_user_agent_suffix_fn(
        headers,
        "openai-compatible",
        "3.0.59",
    );
    Ok(provider)
}

/// The chat-endpoint behavior of a compatible vendor: the capability flags the
/// public settings expose, and the [`ChatDialect`] they cannot. A vendor
/// package or preset supplies its own; the public factory uses the AI SDK
/// baseline.
#[derive(Clone)]
pub(crate) struct ChatProfile {
    pub include_usage: bool,
    pub supports_structured_outputs: bool,
    pub dialect: ChatDialect,
}

/// Everything a compatible provider is made of. The public factory fills it
/// from the settings; vendor packages and presets fill it themselves.
pub(crate) struct Assembly {
    pub name: String,
    pub base_url: BaseUrl,
    pub credential: Credential,
    /// Headers sent before the user's (organization, project, ...).
    pub fixed_headers: Vec<(String, String)>,
    pub headers: Option<HeaderMapOpt>,
    pub query_params: Option<HashMap<String, String>>,
    pub fetch: Option<FetchFunction>,
    pub transform_request_body: Option<TransformRequestBody>,
    pub profile: ChatProfile,
}

/// An OpenAI-compatible provider (the AI SDK's `OpenAICompatibleProvider`).
/// Cheap to take models out of; it holds no HTTP client.
pub struct OpenAICompatibleProvider {
    name: String,
    base_url: BaseUrl,
    query_params: Option<Arc<Vec<(String, String)>>>,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
    chat: ChatSettings,
}

fn compatible_headers(credential: Credential, user: Option<HeaderMapOpt>) -> HeadersFn {
    Resolvable::from_async_fn(move || {
        let credential = credential.clone();
        let user = user.clone();
        async move {
            let mut headers = HeaderMapOpt::new();
            if let Some(key) = credential.secret().await?.filter(|key| !key.is_empty()) {
                headers.insert("Authorization".into(), Some(format!("Bearer {key}")));
            }
            Ok(match user {
                Some(user) => combine_headers(&[&headers, &user]),
                None => headers,
            })
        }
    })
}

impl OpenAICompatibleProvider {
    pub(crate) fn assemble(assembly: Assembly) -> Result<Self, AiMuxError> {
        let name = assembly.name;
        let query_params = assembly.query_params.map(|params| {
            let mut pairs: Vec<(String, String)> = params.into_iter().collect();
            pairs.sort();
            Arc::new(pairs)
        });
        Ok(Self {
            name,
            base_url: assembly.base_url,
            query_params,
            headers: aimux_provider_utils::headers::with_user_agent_suffix_fn(
                provider_headers(
                    assembly.credential,
                    assembly.fixed_headers,
                    assembly.headers,
                ),
                "openai-compatible",
                "3.0.59",
            ),
            fetch: assembly.fetch,
            transform_request_body: assembly.transform_request_body,
            chat: ChatSettings {
                include_usage: assembly.profile.include_usage,
                supports_structured_outputs: assembly.profile.supports_structured_outputs,
                supported_urls: SupportedUrls::default(),
                dialect: Arc::new(assembly.profile.dialect),
            },
        })
    }

    fn model_config(&self, method: &str) -> CompatModelConfig {
        CompatModelConfig {
            provider: format!("{}.{method}", self.name),
            base_url: self.base_url.clone(),
            query_params: self.query_params.clone(),
            headers: self.headers.clone(),
            fetch: self.fetch.clone(),
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
        OpenAICompatibleChatModel::from_config(model_id.to_string(), self.model_config("chat"))
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
}

impl ProviderDiscovery for OpenAICompatibleProvider {
    /// `GET {base_url}/models`: one exchange, no retry. Without a key no
    /// `Authorization` header is sent.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.model_config("models");
        Box::pin(async move { chat::list_models_once(&config).await })
    }
}
