//! Groq provider — a thin OpenAI-compatible wrapper.
//!
//! Groq exposes an OpenAI-compatible Chat Completions API at
//! `https://api.groq.com/openai/v1`. The only provider-specific detail is the
//! base URL and the `GROQ_API_KEY` environment variable; everything else is
//! delegated to the shared [`OpenAIProvider`](crate::openai::OpenAIProvider).
//!
//! Note: Groq does not support `top_k` and sends streaming usage in `x_groq`.
//! These differences are declared via [`OpenAICompatProfile::groq()`].

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://api.groq.com/openai/v1";
const ENV_VAR: &str = "GROQ_API_KEY";
const PROVIDER_NAME: &str = "groq";

/// Configuration for the Groq provider (wraps [`OpenAIConfig`]).
pub struct GroqConfig(OpenAIConfig);

impl GroqConfig {
    /// Create from an API key, using the default Groq base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::groq()),
        )
    }

    /// Create from the `GROQ_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "Groq")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// Groq provider — creates [`OpenAIModel`] instances pointed at Groq.
pub struct GroqProvider(OpenAIProvider);

impl GroqProvider {
    pub fn new(config: GroqConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given Groq model id (e.g. `"llama-3.3-70b-versatile"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for GroqProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
