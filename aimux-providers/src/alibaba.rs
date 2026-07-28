//! Alibaba Cloud (DashScope / Qwen) provider — a thin OpenAI-compatible wrapper.
//!
//! Alibaba Cloud's DashScope exposes an OpenAI-compatible Chat Completions API
//! at `https://dashscope-intl.aliyuncs.com/compatible-mode/v1`. The TS SDK
//! appends `/chat/completions` to this base URL, which is exactly what the Rust
//! [`OpenAIProvider`](crate::openai::OpenAIProvider) does. Provider-specific
//! details are the base URL and the `ALIBABA_API_KEY` environment variable;
//! everything else is delegated to the shared `OpenAIProvider`.
//!
//! Note: Alibaba's reasoning models return a `reasoning_content` field on the
//! assistant message / stream delta. This is now handled by the shared OpenAI
//! response parser via a serde alias (`reasoning` / `reasoning_content`).

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://dashscope-intl.aliyuncs.com/compatible-mode/v1";
const ENV_VAR: &str = "ALIBABA_API_KEY";
const PROVIDER_NAME: &str = "alibaba";

/// Configuration for the Alibaba provider (wraps [`OpenAIConfig`]).
pub struct AlibabaConfig(OpenAIConfig);

impl AlibabaConfig {
    /// Create from an API key, using the default Alibaba base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `ALIBABA_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "Alibaba Cloud (DashScope)")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / regional endpoints such as
    /// the China endpoint `https://dashscope.aliyuncs.com/compatible-mode/v1`).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// Alibaba provider — creates [`OpenAIModel`] instances pointed at DashScope.
pub struct AlibabaProvider(OpenAIProvider);

impl AlibabaProvider {
    pub fn new(config: AlibabaConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given Alibaba model id
    /// (e.g. `"qwen-max"` or `"qwen-plus"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for AlibabaProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
