//! DeepSeek provider — an OpenAI-compatible wrapper with reasoning support.
//!
//! DeepSeek exposes an OpenAI-compatible Chat Completions API at
//! `https://api.deepseek.com/v1`. DeepSeek's reasoning models return a
//! `reasoning_content` field (handled by the shared OpenAI parser via serde
//! alias), and the request body carries a DeepSeek-specific `thinking` field
//! plus a remapped `reasoning_effort`. These request-body differences are
//! handled by `OpenAICompatProfile::deepseek()` + `RequestBodyOverride::DeepSeek`
//! in the shared `build_request_body_with_warnings`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://api.deepseek.com/v1";
const ENV_VAR: &str = "DEEPSEEK_API_KEY";
const PROVIDER_NAME: &str = "deepseek";

/// Configuration for the DeepSeek provider.
pub struct DeepSeekConfig(OpenAIConfig);

impl DeepSeekConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::deepseek()),
        )
    }

    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "DeepSeek")?;
        Ok(Self::new(key))
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// DeepSeek provider — creates [`OpenAIModel`] instances pointed at DeepSeek.
pub struct DeepSeekProvider(OpenAIProvider);

impl DeepSeekProvider {
    pub fn new(config: DeepSeekConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for DeepSeekProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
