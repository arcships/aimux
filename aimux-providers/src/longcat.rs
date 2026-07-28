//! LongCat provider — a thin OpenAI-compatible wrapper.
//!
//! See <longcat.chat> for API documentation. Exposes an OpenAI-compatible
//! Chat Completions API at `https://api.longcat.chat/v1`. Provider-specific details are the
//! base URL and the `LONGCAT_API_KEY` environment variable; everything else is
//! delegated to the shared `OpenAIProvider`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://api.longcat.chat/v1";
const ENV_VAR: &str = "LONGCAT_API_KEY";
const PROVIDER_NAME: &str = "longcat";

pub struct LongcatConfig(OpenAIConfig);

impl LongcatConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "LongCat")?;
        Ok(Self::new(key))
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

pub struct LongcatProvider(OpenAIProvider);

impl LongcatProvider {
    pub fn new(config: LongcatConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for LongcatProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
