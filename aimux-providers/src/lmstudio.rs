//! LM Studio provider — a thin OpenAI-compatible wrapper for the local
//! [LM Studio](https://lmstudio.ai) inference server.
//!
//! LM Studio exposes an OpenAI-compatible Chat Completions API at
//! `http://127.0.0.1:1234/v1` by default. The Rust
//! [`OpenAIProvider`](crate::openai::OpenAIProvider) appends `/chat/completions`
//! to this base URL, yielding `http://127.0.0.1:1234/v1/chat/completions`.
//!
//! Unlike hosted providers, LM Studio runs locally and does not require
//! authentication. Accordingly the `LMSTUDIO_BASE_URL` environment variable
//! holds a *base URL* (not an API key); [`LmStudioConfig::from_env`] reads it
//! and falls back to the default local endpoint when it is unset. A placeholder
//! API key is sent in the `Authorization` header — LM Studio ignores it, but
//! the shared `OpenAIProvider` requires a non-empty key string.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:1234/v1";
/// Environment variable holding the LM Studio base URL (not an API key).
const ENV_VAR: &str = "LMSTUDIO_BASE_URL";
const PROVIDER_NAME: &str = "lmstudio";
/// Placeholder API key — LM Studio does not authenticate, but the shared
/// `OpenAIConfig` requires a non-empty key string.
const PLACEHOLDER_API_KEY: &str = "lmstudio";

/// Configuration for the LM Studio provider (wraps [`OpenAIConfig`]).
pub struct LmStudioConfig(OpenAIConfig);

impl LmStudioConfig {
    /// Create from an API key, using the default local LM Studio base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `LMSTUDIO_BASE_URL` environment variable.
    ///
    /// `LMSTUDIO_BASE_URL` holds a *base URL* (e.g.
    /// `http://127.0.0.1:1234/v1`), not an API key — LM Studio is a local
    /// inference server that does not require authentication. When the variable
    /// is unset (or empty), the default local endpoint
    /// (`http://127.0.0.1:1234/v1`) is used.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let config = Self::new(PLACEHOLDER_API_KEY);
        match std::env::var(ENV_VAR) {
            Ok(url) if !url.trim().is_empty() => Ok(config.with_base_url(url)),
            _ => Ok(config),
        }
    }

    /// Override the base URL (useful for tests / non-default ports).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// LM Studio provider — creates [`OpenAIModel`] instances pointed at LM Studio.
pub struct LmStudioProvider(OpenAIProvider);

impl LmStudioProvider {
    pub fn new(config: LmStudioConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given LM Studio model id
    /// (e.g. `"llama-3.2-3b-instruct"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for LmStudioProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
