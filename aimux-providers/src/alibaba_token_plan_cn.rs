//! Alibaba Token Plan (China) provider — a thin OpenAI-compatible wrapper.
//!
//! Exposes an OpenAI-compatible Chat Completions API at
//! `https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`. Provider-specific details are the base URL
//! and the `ALIBABA_TOKEN_PLAN_API_KEY` environment variable; everything else is delegated
//! to the shared [`OpenAIProvider`](crate::openai::OpenAIProvider).

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1";
const ENV_VAR: &str = "ALIBABA_TOKEN_PLAN_API_KEY";
const PROVIDER_NAME: &str = "alibaba_token_plan_cn";

/// Configuration for the Alibaba Token Plan (China) provider (wraps [`OpenAIConfig`]).
pub struct AlibabaTokenPlanCnConfig(OpenAIConfig);

impl AlibabaTokenPlanCnConfig {
    /// Create from an API key, using the default base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `ALIBABA_TOKEN_PLAN_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "Alibaba Token Plan (China)")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// Alibaba Token Plan (China) provider — creates [`OpenAIModel`] instances.
pub struct AlibabaTokenPlanCnProvider(OpenAIProvider);

impl AlibabaTokenPlanCnProvider {
    pub fn new(config: AlibabaTokenPlanCnConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance (e.g. `MiniMax-M2.5`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for AlibabaTokenPlanCnProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
