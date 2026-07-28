//! Alibaba Coding Plan (China) provider — a thin OpenAI-compatible wrapper.
//!
//! Exposes an OpenAI-compatible Chat Completions API at
//! `https://coding.dashscope.aliyuncs.com/v1`. Provider-specific details are the base URL
//! and the `ALIBABA_CODING_PLAN_API_KEY` environment variable; everything else is delegated
//! to the shared [`OpenAIProvider`](crate::openai::OpenAIProvider).

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://coding.dashscope.aliyuncs.com/v1";
const ENV_VAR: &str = "ALIBABA_CODING_PLAN_API_KEY";
const PROVIDER_NAME: &str = "alibaba_coding_plan_cn";

/// Configuration for the Alibaba Coding Plan (China) provider (wraps [`OpenAIConfig`]).
pub struct AlibabaCodingPlanCnConfig(OpenAIConfig);

impl AlibabaCodingPlanCnConfig {
    /// Create from an API key, using the default base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `ALIBABA_CODING_PLAN_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "Alibaba Coding Plan (China)")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// Alibaba Coding Plan (China) provider — creates [`OpenAIModel`] instances.
pub struct AlibabaCodingPlanCnProvider(OpenAIProvider);

impl AlibabaCodingPlanCnProvider {
    pub fn new(config: AlibabaCodingPlanCnConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance (e.g. `sk-sp-xxxxx`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for AlibabaCodingPlanCnProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
