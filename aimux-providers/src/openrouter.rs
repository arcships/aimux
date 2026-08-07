//! OpenRouter provider — a thin OpenAI-compatible wrapper.
//!
//! OpenRouter exposes an OpenAI-compatible Chat Completions API at
//! `https://openrouter.ai/api/v1`. The only provider-specific detail is the
//! base URL and the `OPENROUTER_API_KEY` environment variable; everything else
//! is delegated to the shared [`OpenAIProvider`](crate::openai::OpenAIProvider).

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{
    OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider, OpenAIResponsesModel,
};

const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
const ENV_VAR: &str = "OPENROUTER_API_KEY";
const PROVIDER_NAME: &str = "openrouter";

/// Configuration for the OpenRouter provider (wraps [`OpenAIConfig`]).
pub struct OpenRouterConfig(OpenAIConfig);

impl OpenRouterConfig {
    /// Create from an API key, using the default OpenRouter base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `OPENROUTER_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "OpenRouter")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// OpenRouter provider — creates [`OpenAIModel`] instances pointed at OpenRouter.
pub struct OpenRouterProvider(OpenAIProvider);

impl OpenRouterProvider {
    pub fn new(config: OpenRouterConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given OpenRouter model id
    /// (e.g. `"openai/gpt-4o-mini"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }

    /// Create a Responses-API model instance for the given OpenRouter model id.
    ///
    /// OpenRouter exposes an OpenAI-compatible Responses API at `/v1/responses`.
    /// This delegates to the underlying [`OpenAIProvider::responses_model`].
    pub fn responses_model(&self, model_id: &str) -> OpenAIResponsesModel {
        self.0.responses_model(model_id)
    }
}

impl Provider for OpenRouterProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }

    crate::delegate_list_models!();
}
