//! SiliconFlow (硅基流动) provider — a thin OpenAI-compatible wrapper.
//!
//! SiliconFlow exposes an OpenAI-compatible Chat Completions API at
//! `https://api.siliconflow.cn/v1`. The Rust
//! [`OpenAIProvider`](crate::openai::OpenAIProvider) appends `/chat/completions`
//! to this base URL. Provider-specific details are the base URL and the
//! `SILICONFLOW_API_KEY` environment variable; everything else is delegated to
//! the shared `OpenAIProvider`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://api.siliconflow.cn/v1";
const ENV_VAR: &str = "SILICONFLOW_API_KEY";
const PROVIDER_NAME: &str = "siliconflow";

/// Configuration for the SiliconFlow provider (wraps [`OpenAIConfig`]).
pub struct SiliconFlowConfig(OpenAIConfig);

impl SiliconFlowConfig {
    /// Create from an API key, using the default SiliconFlow base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `SILICONFLOW_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "SiliconFlow")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// SiliconFlow provider — creates [`OpenAIModel`] instances.
pub struct SiliconFlowProvider(OpenAIProvider);

impl SiliconFlowProvider {
    pub fn new(config: SiliconFlowConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given SiliconFlow model id
    /// (e.g. `"Qwen/Qwen2.5-7B-Instruct"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for SiliconFlowProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
