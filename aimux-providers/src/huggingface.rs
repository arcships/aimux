//! Hugging Face provider — a thin OpenAI-compatible wrapper.
//!
//! Hugging Face exposes an OpenAI-compatible Chat Completions API through its
//! router at `https://router.huggingface.co/v1`. The TS SDK configures this base
//! URL and the `HUGGINGFACE_API_KEY` environment variable. The Rust
//! [`OpenAIConfigProvider`] appends `/chat/completions`
//! to the configured base URL, yielding
//! `https://router.huggingface.co/v1/chat/completions`. Everything else is
//! delegated to the shared `OpenAIConfigProvider`.
//!
//! In addition to the Chat Completions API, Hugging Face also exposes a
//! Responses API (the lightest Responses implementation — function tools only,
//! no built-in tools). See [`responses::HuggingFaceResponsesModel`].

pub mod responses;

use aimux_core::error::AiMuxError;
use aimux_core::provider::ProviderDiscovery;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAIConfig, OpenAIConfigProvider, OpenAIModel};

const DEFAULT_BASE_URL: &str = "https://router.huggingface.co/v1";
const ENV_VAR: &str = "HUGGINGFACE_API_KEY";

/// Configuration for the Hugging Face provider (wraps [`OpenAIConfig`]).
#[derive(Debug, Clone)]
pub struct HuggingFaceConfig(OpenAIConfig);

impl HuggingFaceConfig {
    /// Create from an API key, using the default Hugging Face base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(OpenAIConfig::new(api_key).with_base_url(DEFAULT_BASE_URL))
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
        self.0 = self.0.with_base_url(url);
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
        OpenAIModel::new(model_id.to_string(), self.config.0.clone())
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
        // HuggingFaceProvider holds an OpenAIConfig (not an
        // OpenAIConfigProvider directly), so build one for the discovery call.
        let config = self.config.0.clone();
        Box::pin(async move { OpenAIConfigProvider::new(config).list_models().await })
    }
}
