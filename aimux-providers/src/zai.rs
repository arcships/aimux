//! Zai (智谱 / Z.AI / GLM) provider — a thin OpenAI-compatible wrapper.
//!
//! Zai exposes an OpenAI-compatible Chat Completions API at
//! `https://api.z.ai/api/paas/v4`. The Rust
//! [`OpenAIProvider`](crate::openai::OpenAIProvider) appends `/chat/completions`
//! to this base URL. Provider-specific details are the base URL and the
//! `ZAI_API_KEY` environment variable; everything else is delegated to the
//! shared `OpenAIProvider`.
//!
//! Note: Zai's reasoning models return a `reasoning_content` field on the
//! assistant message / stream delta (same as DeepSeek). This field is currently
//! ignored by the shared OpenAI response parser (serde skips unknown fields).
//! Text content, tool calls, usage, and finish-reason handling all work
//! through the shared `OpenAIProvider`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "https://api.z.ai/api/paas/v4";
const ENV_VAR: &str = "ZAI_API_KEY";
const PROVIDER_NAME: &str = "zai";

/// Configuration for the Zai provider (wraps [`OpenAIConfig`]).
pub struct ZaiConfig(OpenAIConfig);

impl ZaiConfig {
    /// Create from an API key, using the default Zai base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `ZAI_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "Zai")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / regional endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// Zai provider — creates [`OpenAIModel`] instances pointed at Z.AI.
pub struct ZaiProvider(OpenAIProvider);

impl ZaiProvider {
    pub fn new(config: ZaiConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given Zai model id
    /// (e.g. `"glm-4.7"` or `"glm-5.2"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for ZaiProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
