//! LocalAI provider — a thin OpenAI-compatible wrapper.
//!
//! See <localai.io> for API documentation. Exposes an OpenAI-compatible
//! Chat Completions API at `http://127.0.0.1:8080/v1`. The `LOCALAI_BASE_URL` environment
//! variable holds a *base URL* (not an API key); when unset, the default
//! endpoint is used. A placeholder API key is sent in the `Authorization`
//! header — the shared `OpenAIProvider` requires a non-empty key string.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8080/v1";
const ENV_VAR: &str = "LOCALAI_BASE_URL";
const PROVIDER_NAME: &str = "localai";
const PLACEHOLDER_API_KEY: &str = "localai";

pub struct LocalaiConfig(OpenAIConfig);

impl LocalaiConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full())
                .with_api_key_source(Some("none")),
        )
    }

    pub fn from_env() -> Result<Self, AiMuxError> {
        let config = Self::new(PLACEHOLDER_API_KEY);
        match std::env::var(ENV_VAR) {
            Ok(url) if !url.trim().is_empty() => Ok(config.with_base_url(url)),
            _ => Ok(config),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

pub struct LocalaiProvider(OpenAIProvider);

impl LocalaiProvider {
    pub fn new(config: LocalaiConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for LocalaiProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
