//! Groq provider.
//!
//! [`create_groq`] is the Rust form of the AI SDK's `createGroq`: an
//! OpenAI-compatible chat provider named `groq` whose models report
//! `"groq.chat"`, read providerOptions from the `groq` key (and the generic
//! `openaiCompatible` one) and report provider metadata under `groq`. Groq's
//! differences (streaming usage in `x_groq`, no `top_k`, `max_completion_tokens`,
//! structured-output rules, the `browser_search` tool, the `reasoning`
//! message field) live in the package's dialect module and are injected into the shared
//! compatible chat model.
//!
//! As in the other packages the API key is not read when the provider is
//! created: `GROQ_API_KEY` is loaded on every request unless `api_key` is
//! given. [`groq()`] is the default instance.

mod dialect;

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::openai_compatible::config::BaseUrl;
use crate::openai_compatible::{
    Assembly, OpenAICompatibleChatModel, OpenAICompatibleProvider, TransformRequestBody,
};
use crate::shared::Credential;

pub(crate) use dialect::profile;

const DEFAULT_BASE_URL: &str = "https://api.groq.com/openai/v1";
const API_KEY_ENV_VAR: &str = "GROQ_API_KEY";

/// Settings of [`create_groq`] (the AI SDK's `GroqProviderSettings`).
#[derive(Clone, Default)]
pub struct GroqProviderSettings {
    /// Base URL for the API calls. Default `https://api.groq.com/openai/v1`;
    /// a trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `GROQ_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request; a `None` value removes the header.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of every model's `provider()` string and
    /// the providerOptions namespace. Default `"groq"`.
    pub name: Option<String>,
    /// The transport. `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Rewrites every JSON request body once, before it is sent.
    pub transform_request_body: Option<TransformRequestBody>,
}

impl std::fmt::Debug for GroqProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroqProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .field(
                "transform_request_body",
                &self.transform_request_body.is_some(),
            )
            .finish()
    }
}

/// Create a Groq provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host or `name` is empty / contains `.`. The key is not read
/// here.
pub fn create_groq(settings: GroqProviderSettings) -> Result<GroqProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(GroqProvider {
        inner: OpenAICompatibleProvider::assemble(Assembly {
            name: settings.name.unwrap_or_else(|| "groq".to_string()),
            base_url: BaseUrl::Fixed(base_url),
            credential: Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Groq"),
            fixed_headers: Vec::new(),
            headers: settings.headers,
            query_params: None,
            fetch: settings.fetch,
            transform_request_body: settings.transform_request_body,
            profile: profile(),
        })?,
    })
}

/// The default provider: `create_groq` with default settings, created on first
/// use. Creating it reads nothing from the environment and cannot fail.
pub fn groq() -> &'static GroqProvider {
    static DEFAULT: OnceLock<GroqProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_groq(GroqProviderSettings::default())
            .expect("default Groq settings are always valid")
    })
}

/// A Groq provider (the AI SDK's `GroqProvider`). Groq serves language models
/// only here: embedding and image models are `NoSuchModel`.
pub struct GroqProvider {
    inner: OpenAICompatibleProvider,
}

impl GroqProvider {
    /// A chat model; `provider()` is `"{name}.chat"` (`"groq.chat"`).
    #[must_use]
    pub fn chat(&self, model_id: &str) -> OpenAICompatibleChatModel {
        self.inner.chat(model_id)
    }

    /// The provider as a function: the default language model, the chat model.
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        self.inner.call(model_id)
    }
}

impl Provider for GroqProvider {
    fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
        Some(self)
    }

    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }
}

impl ProviderDiscovery for GroqProvider {
    /// `GET {base_url}/models`: one exchange, no retry.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        self.inner.list_models()
    }
}
