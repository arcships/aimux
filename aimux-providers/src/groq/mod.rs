//! Groq provider.
//!
//! [`create_groq`] is the Rust form of the AI SDK's `createGroq`: it takes
//! [`GroqProviderSettings`], validates the base URL, fixes the provider name
//! and returns a [`GroqProvider`] whose chat models are Groq's own
//! [`GroqChatLanguageModel`] (`"groq.chat"`). The API key is not read there;
//! it is loaded in the request headers of every call, from the setting or
//! from `GROQ_API_KEY`. [`groq()`] is the default instance.
//!
//! The files mirror the package's: `model.rs` is
//! `groq-chat-language-model.ts`, `convert.rs` is
//! `convert-to-groq-chat-messages.ts`, `prepare_tools.rs` is
//! `groq-prepare-tools.ts`, `usage.rs` is `convert-groq-usage.ts`,
//! `finish_reason.rs` is `map-groq-finish-reason.ts`, `options.rs` is
//! `groq-chat-language-model-options.ts` and
//! `groq-transcription-model-options.ts`, `transcription.rs` is
//! `groq-transcription-model.ts`, `error.rs` is `groq-error.ts` and
//! `browser_search_models.rs` is `groq-browser-search-models.ts`.

mod browser_search_models;
mod convert;
mod error;
mod finish_reason;
mod model;
mod options;
mod prepare_tools;
mod transcription;
mod types;
mod usage;

pub use model::GroqChatLanguageModel;
pub use transcription::GroqTranscriptionModel;

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::transcription_model::TranscriptionModel;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{Credential, EndpointConfig, provider_headers};

const DEFAULT_BASE_URL: &str = "https://api.groq.com/openai/v1";
const API_KEY_ENV_VAR: &str = "GROQ_API_KEY";
const DEFAULT_NAME: &str = "groq";

/// Settings of [`create_groq`] (the AI SDK's `GroqProviderSettings`).
#[derive(Clone, Default)]
pub struct GroqProviderSettings {
    /// Base URL for the API calls. Default `https://api.groq.com/openai/v1`;
    /// a trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `GROQ_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included.
    pub api_key: Option<String>,
    /// Extra headers on every request; a `None` value removes the header.
    pub headers: Option<HeaderMapOpt>,
    /// The transport. `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for GroqProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroqProviderSettings")
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

/// Create a Groq provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. The key is not read here.
pub fn create_groq(settings: GroqProviderSettings) -> Result<GroqProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(GroqProvider {
        name: DEFAULT_NAME.to_string(),
        base_url,
        headers: aimux_provider_utils::headers::with_user_agent_suffix_fn(
            provider_headers(
                Credential::explicit_or_env(
                    settings.api_key.map(Resolvable::Value),
                    API_KEY_ENV_VAR,
                    "Groq",
                ),
                Vec::new(),
                settings.headers,
            ),
            "groq",
            "4.0.52",
        ),
        fetch: settings.fetch,
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
/// and transcription models: embedding and image models are `NoSuchModel`.
pub struct GroqProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl GroqProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// A chat model; `provider()` is `"{name}.chat"` (`"groq.chat"`).
    #[must_use]
    pub fn chat(&self, model_id: &str) -> GroqChatLanguageModel {
        GroqChatLanguageModel::from_config(model_id.to_string(), self.model_config("chat"))
    }

    /// A transcription model; `provider()` is `"{name}.transcription"`
    /// (`"groq.transcription"`).
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> GroqTranscriptionModel {
        GroqTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        )
    }

    /// The provider as a function: the default language model, the chat model.
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.chat(model_id))
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

    fn transcription_model(
        &self,
        model_id: &str,
    ) -> Option<Result<Arc<dyn TranscriptionModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.transcription(model_id))))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }
}

impl ProviderDiscovery for GroqProvider {
    /// `GET {base_url}/models`: one exchange, no retry.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.model_config("models");
        Box::pin(async move {
            crate::shared::list_data_models(&config, error::groq_failed_response_handler()).await
        })
    }
}
