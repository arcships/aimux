//! Llamafile provider — a thin OpenAI-compatible wrapper for the local
//! [llamafile](https://github.com/Mozilla-Ocho/llamafile) inference server.
//!
//! llamafile exposes an OpenAI-compatible Chat Completions API at
//! `http://127.0.0.1:8080/v1` by default. The Rust
//! [`OpenAIProvider`](crate::openai::OpenAIProvider) appends `/chat/completions`
//! to this base URL, yielding `http://127.0.0.1:8080/v1/chat/completions`.
//!
//! Unlike hosted providers, llamafile runs locally and does not require
//! authentication. Accordingly the `LLAMAFILE_BASE_URL` environment variable
//! holds a *base URL* (not an API key); [`LlamafileConfig::from_env`] reads it
//! and falls back to the default local endpoint when it is unset. A placeholder
//! API key is sent in the `Authorization` header — llamafile ignores it, but the
//! shared `OpenAIProvider` requires a non-empty key string.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8080/v1";
/// Environment variable holding the llamafile base URL (not an API key).
const ENV_VAR: &str = "LLAMAFILE_BASE_URL";
const PROVIDER_NAME: &str = "llamafile";
/// Placeholder API key — llamafile does not authenticate, but the shared
/// `OpenAIConfig` requires a non-empty key string.
const PLACEHOLDER_API_KEY: &str = "llamafile";

/// Configuration for the llamafile provider (wraps [`OpenAIConfig`]).
pub struct LlamafileConfig(OpenAIConfig);

impl LlamafileConfig {
    /// Create from an API key, using the default local llamafile base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full())
                .with_api_key_source(Some("none")),
        )
    }

    /// Create from the `LLAMAFILE_BASE_URL` environment variable.
    ///
    /// `LLAMAFILE_BASE_URL` holds a *base URL* (e.g.
    /// `http://127.0.0.1:8080/v1`), not an API key — llamafile is a local
    /// inference server that does not require authentication. When the variable
    /// is unset (or empty), the default local endpoint
    /// (`http://127.0.0.1:8080/v1`) is used.
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

/// llamafile provider — creates [`OpenAIModel`] instances pointed at llamafile.
pub struct LlamafileProvider(OpenAIProvider);

impl LlamafileProvider {
    pub fn new(config: LlamafileConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given llamafile model id
    /// (e.g. `"llama3.2:latest"`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for LlamafileProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }

    crate::delegate_list_models!();
}
