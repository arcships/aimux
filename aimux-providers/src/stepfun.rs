//! StepFun (阶跃星辰) provider — a thin OpenAI-compatible wrapper.
//!
//! See <stepfun.com> for API documentation. Exposes an OpenAI-compatible
//! Chat Completions API at `https://api.stepfun.com/v1`. Provider-specific details are the
//! base URL and the `STEPFUN_API_KEY` environment variable; everything else is
//! delegated to the shared `OpenAIProvider`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://api.stepfun.com/v1";
const ENV_VAR: &str = "STEPFUN_API_KEY";
const PROVIDER_NAME: &str = "stepfun";

pub struct StepfunConfig(OpenAIConfig);

impl StepfunConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "StepFun (阶跃星辰)")?;
        Ok(Self::new(key))
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

pub struct StepfunProvider(OpenAIProvider);

impl StepfunProvider {
    pub fn new(config: StepfunConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for StepfunProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
