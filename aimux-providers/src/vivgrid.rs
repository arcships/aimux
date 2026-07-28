//! Vivgrid provider — a thin OpenAI-compatible wrapper.
//!
//! Exposes an OpenAI-compatible Chat Completions API at
//! `https://api.vivgrid.com/v1`. Provider-specific details are the base URL
//! and the `VIVGRID_API_KEY` environment variable; everything else is delegated
//! to the shared [`OpenAIProvider`](crate::openai::OpenAIProvider).

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://api.vivgrid.com/v1";
const ENV_VAR: &str = "VIVGRID_API_KEY";
const PROVIDER_NAME: &str = "vivgrid";

/// Configuration for the Vivgrid provider (wraps [`OpenAIConfig`]).
pub struct VivgridConfig(OpenAIConfig);

impl VivgridConfig {
    /// Create from an API key, using the default base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `VIVGRID_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "Vivgrid")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// Vivgrid provider — creates [`OpenAIModel`] instances.
pub struct VivgridProvider(OpenAIProvider);

impl VivgridProvider {
    pub fn new(config: VivgridConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance (e.g. `messages`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for VivgridProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
