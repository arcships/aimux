//! Mistral AI provider.
//!
//! [`create_mistral`] is the Rust form of the AI SDK's `createMistral`: it
//! takes [`MistralProviderSettings`], validates the base URL, fixes the
//! provider name and returns a [`MistralProvider`]. The API key is not read
//! there; it is loaded in the request headers of every call, from the setting
//! or from `MISTRAL_API_KEY`. [`mistral()`] is the default instance.
//!
//! OpenAI-compatible chat completions API with Mistral-specific differences:
//! - Tool choice uses `"any"` instead of `"required"`
//! - Content can be a string or an array of typed parts (text, thinking, image_url)
//! - Usage supports `num_cached_tokens`
//! - Finish reasons include `model_length`

pub mod convert;
pub mod embedding;
mod model;
pub(crate) mod options;
mod types;

pub use embedding::MistralEmbeddingModel;
pub use model::MistralModel;

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;
use serde_json::Value;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{Credential, EndpointConfig, provider_headers};

pub(crate) fn mistral_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: data
                .get("code")
                .or_else(|| data.get("type"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    })
}

pub(crate) fn mistral_stream_error(
    error: &Value,
    url: &str,
    request_body_values: Value,
    response_headers: std::collections::HashMap<String, String>,
) -> AiMuxError {
    let status_code = error
        .get("status_code")
        .or_else(|| error.get("status"))
        .or_else(|| error.get("code"))
        .and_then(Value::as_u64)
        .and_then(|status| u16::try_from(status).ok())
        .filter(|status| (400..=599).contains(status));
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Mistral stream failed before any output was generated")
        .to_owned();
    let provider_code =
        error
            .get("code")
            .or_else(|| error.get("type"))
            .and_then(|value| match value {
                Value::String(code) => Some(code.clone()),
                Value::Number(code) => Some(code.to_string()),
                _ => None,
            });
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

const DEFAULT_BASE_URL: &str = "https://api.mistral.ai/v1";
const API_KEY_ENV_VAR: &str = "MISTRAL_API_KEY";
const DEFAULT_NAME: &str = "mistral";

/// The URL patterns the chat model fetches itself (`supportedUrls` of the AI
/// SDK's `MistralChatLanguageModel`): `https` PDFs.
fn chat_supported_urls() -> SupportedUrls {
    let https = regex::Regex::new(r"^https://.*$").expect("static pattern");
    SupportedUrls(std::iter::once(("application/pdf".to_string(), vec![https])).collect())
}

/// Settings of [`create_mistral`] (the AI SDK's `MistralProviderSettings`).
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct MistralProviderSettings {
    /// Base URL for the API calls. Default `https://api.mistral.ai/v1`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `MISTRAL_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment.
    pub api_key: Option<String>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including `Authorization`. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Generates unique IDs for chat output.
    pub generate_id: Option<Arc<dyn Fn() -> String + Send + Sync>>,
}

impl std::fmt::Debug for MistralProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MistralProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.is_some())
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("fetch", &self.fetch.is_some())
            .field("generate_id", &self.generate_id.is_some())
            .finish()
    }
}

/// Create a Mistral provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_mistral(settings: MistralProviderSettings) -> Result<MistralProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(MistralProvider {
        name: DEFAULT_NAME.to_string(),
        base_url,
        headers: aimux_provider_utils::headers::with_user_agent_suffix_fn(
            provider_headers(
                Credential::explicit_or_env(
                    settings.api_key.map(Resolvable::Value),
                    API_KEY_ENV_VAR,
                    "Mistral",
                ),
                Vec::new(),
                settings.headers,
            ),
            options::NAMESPACE,
            "4.0.54",
        ),
        fetch: settings.fetch,
        generate_id: settings.generate_id,
    })
}

/// The default provider: `create_mistral` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn mistral() -> &'static MistralProvider {
    static DEFAULT: OnceLock<MistralProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_mistral(MistralProviderSettings::default())
            .expect("default Mistral settings are always valid")
    })
}

/// A Mistral provider (the AI SDK's `MistralProvider`). Cheap to clone the
/// models out of; it holds no HTTP client.
pub struct MistralProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
    generate_id: Option<Arc<dyn Fn() -> String + Send + Sync>>,
}

impl MistralProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// A chat model; `provider()` is `"{name}.chat"`.
    #[must_use]
    pub fn chat(&self, model_id: &str) -> MistralModel {
        MistralModel::from_config(
            model_id.to_string(),
            self.model_config("chat")
                .with_supported_urls(Arc::new(|_| chat_supported_urls())),
        )
        .with_generate_id(self.generate_id.clone())
    }

    /// An embedding model (e.g. `"mistral-embed"`); `provider()` is
    /// `"{name}.embedding"`.
    #[must_use]
    pub fn embedding(&self, model_id: &str) -> MistralEmbeddingModel {
        MistralEmbeddingModel::from_config(model_id.to_string(), self.model_config("embedding"))
    }

    /// The provider as a function: the default language model for an id. The
    /// AI SDK's callable provider; the same model as [`chat`](Self::chat) and
    /// [`language_model`](Provider::language_model).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.chat(model_id))
    }
}

impl Provider for MistralProvider {
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
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }
}

impl ProviderDiscovery for MistralProvider {
    /// `GET {base_url}/models`: one exchange, no retry.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.model_config("models");
        Box::pin(async move {
            crate::shared::list_data_models(&config, mistral_failed_response_handler()).await
        })
    }
}
