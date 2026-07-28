//! Vercel (v0) provider — a thin OpenAI-compatible wrapper.
//!
//! Vercel's v0 exposes an OpenAI-compatible Chat Completions API at
//! `https://api.v0.dev/v1`. The TS SDK configures this base URL and the
//! `VERCEL_API_KEY` environment variable; the Rust
//! [`OpenAIProvider`](crate::openai::OpenAIProvider) appends `/chat/completions`
//! to the configured base URL. Everything else is delegated to the shared
//! `OpenAIProvider`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://api.v0.dev/v1";
const ENV_VAR: &str = "VERCEL_API_KEY";
const PROVIDER_NAME: &str = "vercel";

/// Configuration for the Vercel provider (wraps [`OpenAIConfig`]).
pub struct VercelConfig(OpenAIConfig);

impl VercelConfig {
    /// Create from an API key, using the default Vercel base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `VERCEL_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "Vercel")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// Vercel provider — creates [`OpenAIModel`] instances pointed at v0.
pub struct VercelProvider(OpenAIProvider);

impl VercelProvider {
    pub fn new(config: VercelConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given Vercel model id
    /// (e.g. `"v0-1.5-md"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for VercelProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
