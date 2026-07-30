//! Hugging Face provider — a thin OpenAI-compatible wrapper.
//!
//! Hugging Face exposes an OpenAI-compatible Chat Completions API through its
//! router at `https://router.huggingface.co/v1`. The TS SDK configures this base
//! URL and the `HUGGINGFACE_API_KEY` environment variable. The Rust
//! [`OpenAIProvider`](crate::openai::OpenAIProvider) appends `/chat/completions`
//! to the configured base URL, yielding
//! `https://router.huggingface.co/v1/chat/completions`. Everything else is
//! delegated to the shared `OpenAIProvider`.
//!
//! In addition to the Chat Completions API, Hugging Face also exposes a
//! Responses API (the lightest Responses implementation — function tools only,
//! no built-in tools). See [`responses::HuggingFaceResponsesModel`].

pub mod responses;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAIConfig, OpenAIModel};

const DEFAULT_BASE_URL: &str = "https://router.huggingface.co/v1";
const ENV_VAR: &str = "HUGGINGFACE_API_KEY";
const PROVIDER_NAME: &str = "huggingface";

/// Configuration for the Hugging Face provider (wraps [`OpenAIConfig`]).
#[derive(Debug, Clone)]
pub struct HuggingFaceConfig(OpenAIConfig);

impl HuggingFaceConfig {
    /// Create from an API key, using the default Hugging Face base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(OpenAIConfig::new(api_key).with_base_url(DEFAULT_BASE_URL))
    }

    /// Create from the `HUGGINGFACE_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "Hugging Face")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
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
    pub fn new(config: HuggingFaceConfig) -> Self {
        Self { config }
    }

    /// Create a chat model instance for the given Hugging Face model id
    /// (e.g. `"meta-llama/Llama-3.3-70B-Instruct"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        OpenAIModel::new(model_id.to_string(), self.config.0.clone())
    }

    /// Create a Responses model instance for the given Hugging Face model id.
    ///
    /// The Hugging Face Responses API is the lightest Responses implementation:
    /// it supports function tools only (no built-in tools), and uses the
    /// `text.format` field for structured output.
    pub fn responses_model(&self, model_id: &str) -> responses::HuggingFaceResponsesModel {
        responses::HuggingFaceResponsesModel::new(
            model_id.to_string(),
            self.config.clone(),
        )
    }
}

impl Provider for HuggingFaceProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
