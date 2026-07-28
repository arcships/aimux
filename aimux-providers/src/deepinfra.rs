//! DeepInfra provider — a thin OpenAI-compatible wrapper.
//!
//! DeepInfra exposes an OpenAI-compatible Chat Completions API. The TS SDK
//! configures a base URL of `https://api.deepinfra.com/v1` and appends the
//! `/openai` prefix plus the request path (e.g. `/chat/completions`), yielding
//! `https://api.deepinfra.com/v1/openai/chat/completions`. The Rust
//! [`OpenAIProvider`](crate::openai::OpenAIProvider) appends `/chat/completions`
//! to the configured base URL directly, so we bake the `/openai` suffix into the
//! default base URL. Provider-specific details are the base URL and the
//! `DEEPINFRA_API_KEY` environment variable; everything else is delegated to the
//! shared `OpenAIProvider`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://api.deepinfra.com/v1/openai";
const ENV_VAR: &str = "DEEPINFRA_API_KEY";
const PROVIDER_NAME: &str = "deepinfra";

/// Configuration for the DeepInfra provider (wraps [`OpenAIConfig`]).
pub struct DeepInfraConfig(OpenAIConfig);

impl DeepInfraConfig {
    /// Create from an API key, using the default DeepInfra base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `DEEPINFRA_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "DeepInfra")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// DeepInfra provider — creates [`OpenAIModel`] instances pointed at DeepInfra.
pub struct DeepInfraProvider(OpenAIProvider);

impl DeepInfraProvider {
    pub fn new(config: DeepInfraConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given DeepInfra model id
    /// (e.g. `"meta-llama/Meta-Llama-3-70B-Instruct"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for DeepInfraProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
