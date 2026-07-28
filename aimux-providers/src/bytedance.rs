//! ByteDance (Volcengine Ark) provider — a thin OpenAI-compatible wrapper.
//!
//! Volcengine Ark (火山方舟) exposes an OpenAI-compatible Chat Completions API
//! at `https://ark.cn-beijing.volces.com/api/v3`. The Rust
//! [`OpenAIProvider`](crate::openai::OpenAIProvider) appends `/chat/completions`
//! to the configured base URL, yielding
//! `https://ark.cn-beijing.volces.com/api/v3/chat/completions`. Provider-specific
//! details are the base URL and the `ARK_API_KEY` environment variable (with a
//! fallback to `BYTE_DANCE_API_KEY`); everything else is delegated to the shared
//! `OpenAIProvider`.
//!
//! Note: the TS SDK's ByteDance package only models image/video generation and
//! uses the South-East-Asia endpoint `https://ark.ap-southeast.bytepluses.com/api/v3`.
//! This Rust wrapper targets the chat-completions API on the China endpoint; use
//! [`ByteDanceConfig::with_base_url`] to point at a different region.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://ark.cn-beijing.volces.com/api/v3";
const ENV_VAR: &str = "ARK_API_KEY";
const FALLBACK_ENV_VAR: &str = "BYTE_DANCE_API_KEY";
const PROVIDER_NAME: &str = "bytedance";

/// Configuration for the ByteDance provider (wraps [`OpenAIConfig`]).
pub struct ByteDanceConfig(OpenAIConfig);

impl ByteDanceConfig {
    /// Create from an API key, using the default ByteDance Ark base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `ARK_API_KEY` environment variable, falling back to
    /// `BYTE_DANCE_API_KEY` if `ARK_API_KEY` is not set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "ByteDance")
            .or_else(|_| load_api_key(None, FALLBACK_ENV_VAR, "ByteDance"))?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / regional endpoints such as the
    /// South-East-Asia endpoint `https://ark.ap-southeast.bytepluses.com/api/v3`).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// ByteDance provider — creates [`OpenAIModel`] instances pointed at Ark.
pub struct ByteDanceProvider(OpenAIProvider);

impl ByteDanceProvider {
    pub fn new(config: ByteDanceConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given ByteDance Ark model id
    /// (e.g. `"doubao-pro-32k"` or an endpoint id like `"ep-2024xxx"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for ByteDanceProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
