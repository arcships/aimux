//! GitHub Copilot provider — a thin OpenAI-compatible wrapper.
//!
//! GitHub Copilot exposes an OpenAI-compatible Chat Completions API at
//! `https://api.githubcopilot.com`. The Rust
//! [`OpenAIProvider`](crate::openai::OpenAIProvider) appends `/chat/completions`
//! to this base URL, yielding `https://api.githubcopilot.com/chat/completions`.
//! Provider-specific details are the base URL and the `COPILOT_API_KEY`
//! environment variable; everything else is delegated to the shared
//! `OpenAIProvider`.
//!
//! Note: Copilot also exposes `/responses`, `/embeddings`, and `/models`
//! endpoints. Only the Chat Completions surface is wired up by this thin
//! wrapper; callers needing the Responses API can reach it through the shared
//! `OpenAIProvider` directly.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://api.githubcopilot.com";
const ENV_VAR: &str = "COPILOT_API_KEY";
const PROVIDER_NAME: &str = "copilot";

/// Configuration for the GitHub Copilot provider (wraps [`OpenAIConfig`]).
pub struct CopilotConfig(OpenAIConfig);

impl CopilotConfig {
    /// Create from an API key, using the default Copilot base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `COPILOT_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "GitHub Copilot")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// GitHub Copilot provider — creates [`OpenAIModel`] instances pointed at Copilot.
pub struct CopilotProvider(OpenAIProvider);

impl CopilotProvider {
    pub fn new(config: CopilotConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given Copilot model id (e.g. `"gpt-4o"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for CopilotProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
