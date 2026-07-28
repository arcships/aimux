//! AIGC2D provider — a thin OpenAI-compatible wrapper.
//!
//! See <aigc2d.com> for API documentation. Exposes an OpenAI-compatible
//! Chat Completions API at `https://api.aigc2d.com/v1`. Provider-specific details are the
//! base URL and the `AIGC2D_API_KEY` environment variable; everything else is
//! delegated to the shared `OpenAIProvider`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://api.aigc2d.com/v1";
const ENV_VAR: &str = "AIGC2D_API_KEY";
const PROVIDER_NAME: &str = "aigc2d";

pub struct Aigc2dConfig(OpenAIConfig);

impl Aigc2dConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "AIGC2D")?;
        Ok(Self::new(key))
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

pub struct Aigc2dProvider(OpenAIProvider);

impl Aigc2dProvider {
    pub fn new(config: Aigc2dConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for Aigc2dProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
