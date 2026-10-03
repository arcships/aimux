//! Hugging Face provider — a thin OpenAI-compatible wrapper.
//!
//! Hugging Face exposes an OpenAI-compatible Chat Completions API through its
//! router at `https://router.huggingface.co/v1`. The TS SDK configures this base
//! URL and the `HUGGINGFACE_API_KEY` environment variable. The Rust
//! chat model appends `/chat/completions` to the configured base URL, yielding
//! `https://router.huggingface.co/v1/chat/completions`.
//!
//! In addition to the Chat Completions API, Hugging Face also exposes a
//! Responses API (the lightest Responses implementation — function tools only,
//! no built-in tools). See [`responses::HuggingFaceResponsesModel`].

pub mod responses;

use aimux_core::error::AiMuxError;
use aimux_core::provider::ProviderDiscovery;
use aimux_provider_utils::load_api_key;

use crate::openai::OpenAIModel;
use crate::openai::config::StaticBearerConfig;

const DEFAULT_BASE_URL: &str = "https://router.huggingface.co/v1";
const ENV_VAR: &str = "HUGGINGFACE_API_KEY";

/// Configuration for the Hugging Face provider.
#[derive(Debug, Clone)]
pub struct HuggingFaceConfig(StaticBearerConfig);

impl HuggingFaceConfig {
    /// Create from an API key, using the default Hugging Face base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(StaticBearerConfig::new(
            "huggingface",
            api_key,
            DEFAULT_BASE_URL,
        ))
    }

    /// Create from the `HUGGINGFACE_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when `HUGGINGFACE_API_KEY` is not
    /// set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "Hugging Face")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_origin(url.into());
        self
    }
}

/// Hugging Face provider — creates [`OpenAIModel`] (chat) and
/// [`responses::HuggingFaceResponsesModel`] (responses) instances pointed at HF.
pub struct HuggingFaceProvider {
    config: HuggingFaceConfig,
}

impl HuggingFaceProvider {
    #[must_use]
    pub fn new(config: HuggingFaceConfig) -> Self {
        Self { config }
    }

    /// Create a chat model instance for the given Hugging Face model id
    /// (e.g. `"meta-llama/Llama-3.3-70B-Instruct"`).
    #[must_use]
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        OpenAIModel::from_config(model_id.to_string(), self.config.0.model_config("chat"))
    }

    /// Create a Responses model instance for the given Hugging Face model id.
    ///
    /// The Hugging Face Responses API is the lightest Responses implementation:
    /// it supports function tools only (no built-in tools), and uses the
    /// `text.format` field for structured output.
    #[must_use]
    pub fn responses_model(&self, model_id: &str) -> responses::HuggingFaceResponsesModel {
        responses::HuggingFaceResponsesModel::new(model_id.to_string(), self.config.clone())
    }
}

crate::impl_single_modality_provider!(HuggingFaceProvider, language_model, |p, id| p.model(id));

impl ProviderDiscovery for HuggingFaceProvider {
    fn list_models(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Vec<aimux_core::model_catalogue::RuntimeModel>, AiMuxError>,
                > + Send
                + '_,
        >,
    > {
        let config = self.config.0.model_config("models");
        Box::pin(async move { crate::openai::model::list_models_once(&config).await })
    }
}
