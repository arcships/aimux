//! DeepSeek provider.
//!
//! [`create_deepseek`] is the Rust form of the AI SDK's `createDeepSeek`: a
//! provider of [`DeepSeekChatLanguageModel`], the package's own chat model
//! (`@ai-sdk/deepseek` does not build on the OpenAI-compatible package). Its
//! models report `"deepseek.chat"` and read providerOptions and report provider
//! metadata under `deepseek`.
//!
//! As in the other packages the API key is not read when the provider is
//! created: `DEEPSEEK_API_KEY` is loaded on every request unless `api_key` is
//! given. [`deepseek()`] is the default instance.

mod convert;
mod finish_reason;
mod is_v4_model;
mod model;
mod options;
mod prepare_tools;
mod types;
mod usage;

pub use model::DeepSeekChatLanguageModel;

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{Credential, EndpointConfig, provider_headers};
use model::DeepSeekChatConfig;

const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
const API_KEY_ENV_VAR: &str = "DEEPSEEK_API_KEY";

/// The URL patterns the chat model fetches itself (`supportedUrls` of the AI
/// SDK's `DeepSeekChatLanguageModel`): `http(s)` images.
fn chat_supported_urls() -> SupportedUrls {
    let http = regex::Regex::new(r"^https?://.*$").expect("static pattern");
    SupportedUrls(std::iter::once(("image/*".to_string(), vec![http])).collect())
}

/// Settings of [`create_deepseek`] (the AI SDK's `DeepSeekProviderSettings`).
#[derive(Clone, Default)]
pub struct DeepSeekProviderSettings {
    /// Base URL for the API calls. Default `https://api.deepseek.com`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `DEEPSEEK_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included.
    pub api_key: Option<String>,
    /// Extra headers on every request; a `None` value removes the header.
    pub headers: Option<HeaderMapOpt>,
    /// The transport. `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for DeepSeekProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeepSeekProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.is_some())
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Create a DeepSeek provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. The key is not read here.
pub fn create_deepseek(settings: DeepSeekProviderSettings) -> Result<DeepSeekProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(DeepSeekProvider {
        name: "deepseek".to_string(),
        base_url,
        headers: aimux_provider_utils::headers::with_user_agent_suffix_fn(
            provider_headers(
                Credential::explicit_or_env(
                    settings.api_key.map(Resolvable::Value),
                    API_KEY_ENV_VAR,
                    "DeepSeek",
                ),
                Vec::new(),
                settings.headers,
            ),
            "deepseek",
            "3.0.56",
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_deepseek` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail.
pub fn deepseek() -> &'static DeepSeekProvider {
    static DEFAULT: OnceLock<DeepSeekProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_deepseek(DeepSeekProviderSettings::default())
            .expect("default DeepSeek settings are always valid")
    })
}

/// A DeepSeek provider (the AI SDK's `DeepSeekProvider`). Language models
/// only: embedding and image models are `NoSuchModel`.
pub struct DeepSeekProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl DeepSeekProvider {
    fn endpoint_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// A chat model; `provider()` is `"{name}.chat"` (`"deepseek.chat"`).
    #[must_use]
    pub fn chat(&self, model_id: &str) -> DeepSeekChatLanguageModel {
        DeepSeekChatLanguageModel::from_config(
            model_id.to_string(),
            DeepSeekChatConfig {
                endpoint: self
                    .endpoint_config("chat")
                    .with_supported_urls(Arc::new(|_| chat_supported_urls())),
                supports_assistant_prefix_completion: self.base_url.ends_with("/beta"),
                supports_strict_tool_calls: self.base_url.ends_with("/beta"),
            },
        )
    }

    /// The provider as a function: the default language model, the chat model.
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.chat(model_id))
    }
}

impl Provider for DeepSeekProvider {
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

impl ProviderDiscovery for DeepSeekProvider {
    /// `GET {base_url}/models`: one exchange, no retry.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.endpoint_config("models");
        Box::pin(async move {
            crate::shared::list_data_models(
                &config,
                aimux_provider_utils::create_standard_json_error_response_handler(),
            )
            .await
        })
    }
}
