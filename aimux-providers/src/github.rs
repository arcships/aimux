//! GitHub Models provider — a thin OpenAI-compatible wrapper.
//!
//! GitHub Models exposes an OpenAI-compatible Chat Completions API at
//! `https://models.inference.ai.azure.com`. The Rust
//! [`OpenAIProvider`](crate::openai::OpenAIProvider) appends `/chat/completions`
//! to this base URL. Provider-specific details are the base URL and the
//! `GITHUB_TOKEN` environment variable; everything else is delegated to the
//! shared `OpenAIProvider`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://models.inference.ai.azure.com";
const ENV_VAR: &str = "GITHUB_TOKEN";
const PROVIDER_NAME: &str = "github";

/// Configuration for the GitHub Models provider (wraps [`OpenAIConfig`]).
pub struct GithubConfig(OpenAIConfig);

impl GithubConfig {
    /// Create from an API key, using the default GitHub Models base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `GITHUB_TOKEN` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "GitHub Models")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// GitHub Models provider — creates [`OpenAIModel`] instances pointed at GitHub.
pub struct GithubProvider(OpenAIProvider);

impl GithubProvider {
    pub fn new(config: GithubConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given GitHub model id
    /// (e.g. `"gpt-4o"` or `"Phi-3.5-mini-instruct"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for GithubProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
