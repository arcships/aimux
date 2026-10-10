//! Hugging Face provider.
//!
//! [`create_huggingface`] is the Rust form of the AI SDK's `createHuggingFace`:
//! it takes [`HuggingFaceProviderSettings`], validates the base URL, fixes the
//! provider name and returns a [`HuggingFaceProvider`]. The API key is not read
//! there; it is loaded in the request headers of every call, from the setting
//! or from `HUGGINGFACE_API_KEY`. [`huggingface()`] is the default instance.
//!
//! The AI SDK's package serves the Responses API only
//! ([`responses::HuggingFaceResponsesModel`], the lightest Responses
//! implementation: function tools only, no built-in tools), and
//! [`language_model`](Provider::language_model) returns it. There is no Chat
//! Completions model in this package; the router's OpenAI-compatible endpoint
//! is reachable through `create_openai_compatible` with
//! `https://router.huggingface.co/v1`.

pub(crate) mod options;
pub mod responses;

pub use responses::HuggingFaceResponsesModel;

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::openai::config::OpenAIModelConfig;
use crate::shared::{Credential, EndpointConfig, provider_headers};

const DEFAULT_BASE_URL: &str = "https://router.huggingface.co/v1";
const API_KEY_ENV_VAR: &str = "HUGGINGFACE_API_KEY";
const DEFAULT_NAME: &str = "huggingface";

/// Settings of [`create_huggingface`] (the AI SDK's
/// `HuggingFaceProviderSettings`).
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct HuggingFaceProviderSettings {
    /// Base URL for the API calls. Default `https://router.huggingface.co/v1`;
    /// a trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `HUGGINGFACE_API_KEY` when a request is made
    /// and fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including `Authorization`. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings
    /// (`"{name}.responses"`). Default `"huggingface"`. The providerOptions
    /// key stays `huggingface`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for HuggingFaceProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HuggingFaceProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Create a Hugging Face provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_huggingface(
    settings: HuggingFaceProviderSettings,
) -> Result<HuggingFaceProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(HuggingFaceProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Hugging Face"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_huggingface` with default settings, created
/// on first use. Creating it reads nothing from the environment and cannot
/// fail; a missing key surfaces from the first request instead.
pub fn huggingface() -> &'static HuggingFaceProvider {
    static DEFAULT: OnceLock<HuggingFaceProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_huggingface(HuggingFaceProviderSettings::default())
            .expect("default Hugging Face settings are always valid")
    })
}

/// A Hugging Face provider (the AI SDK's `HuggingFaceProvider`). Cheap to
/// clone the models out of; it holds no HTTP client.
pub struct HuggingFaceProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl HuggingFaceProvider {
    fn endpoint_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
        )
    }

    fn openai_config(&self, method: &str) -> OpenAIModelConfig {
        OpenAIModelConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
        )
    }

    /// A Responses model (e.g. `"deepseek-ai/DeepSeek-V3-0324"`);
    /// `provider()` is `"{name}.responses"`.
    ///
    /// The Hugging Face Responses API supports function tools only (no
    /// built-in tools), and uses the `text.format` field for structured output.
    #[must_use]
    pub fn responses(&self, model_id: &str) -> HuggingFaceResponsesModel {
        HuggingFaceResponsesModel::from_config(
            model_id.to_string(),
            self.endpoint_config("responses"),
        )
    }

    /// The provider as a function: the default language model for an id. The
    /// AI SDK's callable provider; the same model as
    /// [`responses`](Self::responses) and
    /// [`language_model`](Provider::language_model).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.responses(model_id))
    }
}

impl Provider for HuggingFaceProvider {
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

impl ProviderDiscovery for HuggingFaceProvider {
    /// `GET {base_url}/models`: one exchange, no retry.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.openai_config("models");
        Box::pin(async move { crate::openai::model::list_models_once(&config).await })
    }
}
