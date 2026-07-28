//! Baseten provider — a thin OpenAI-compatible wrapper.
//!
//! Baseten exposes an OpenAI-compatible Chat Completions API. The TS SDK uses a
//! default base URL of `https://inference.baseten.co/v1` and appends the request
//! path (`/chat/completions`) directly, yielding
//! `https://inference.baseten.co/v1/chat/completions`. Provider-specific details
//! are the base URL and the `BASETEN_API_KEY` environment variable; everything
//! else is delegated to the shared [`OpenAIProvider`](crate::openai::OpenAIProvider).
//!
//! Note: Baseten also supports per-model URLs of the form
//! `https://<model>.api.baseten.co/environments/production/sync/v1`; those are
//! not modeled here — callers can supply any endpoint via [`BasetenConfig::with_base_url`].

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://inference.baseten.co/v1";
const ENV_VAR: &str = "BASETEN_API_KEY";
const PROVIDER_NAME: &str = "baseten";

/// Configuration for the Baseten provider (wraps [`OpenAIConfig`]).
pub struct BasetenConfig(OpenAIConfig);

impl BasetenConfig {
    /// Create from an API key, using the default Baseten base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `BASETEN_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "Baseten")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// Baseten provider — creates [`OpenAIModel`] instances pointed at Baseten.
pub struct BasetenProvider(OpenAIProvider);

impl BasetenProvider {
    pub fn new(config: BasetenConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given Baseten model id
    /// (e.g. `"deepseek-ai/DeepSeek-V3-0324"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for BasetenProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
