//! Moonshot AI provider — a thin OpenAI-compatible wrapper.
//!
//! Moonshot AI (Kimi) exposes an OpenAI-compatible Chat Completions API at
//! `https://api.moonshot.cn/v1`. Provider-specific details are the base URL
//! and the `MOONSHOT_API_KEY` environment variable; everything else is
//! delegated to the shared [`OpenAIProvider`](crate::openai::OpenAIProvider).

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://api.moonshot.cn/v1";
const ENV_VAR: &str = "MOONSHOT_API_KEY";
const PROVIDER_NAME: &str = "moonshotai";

/// Configuration for the Moonshot AI provider (wraps [`OpenAIConfig`]).
pub struct MoonshotAIConfig(OpenAIConfig);

impl MoonshotAIConfig {
    /// Create from an API key, using the default Moonshot AI base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `MOONSHOT_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "Moonshot AI")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// Moonshot AI provider — creates [`OpenAIModel`] instances pointed at Moonshot AI.
pub struct MoonshotAIProvider(OpenAIProvider);

impl MoonshotAIProvider {
    pub fn new(config: MoonshotAIConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given Moonshot AI model id
    /// (e.g. `"moonshot-v1-8k"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for MoonshotAIProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
