//! Ollama provider — a thin OpenAI-compatible wrapper for the local
//! [Ollama](https://ollama.com) inference server.
//!
//! Ollama exposes an OpenAI-compatible Chat Completions API at
//! `http://127.0.0.1:11434/v1` by default. The Rust
//! [`OpenAIProvider`] appends `/chat/completions`
//! to this base URL, yielding `http://127.0.0.1:11434/v1/chat/completions`.
//!
//! Unlike hosted providers, Ollama runs locally and does not require
//! authentication. Accordingly the `OLLAMA_BASE_URL` environment variable
//! holds a *base URL* (not an API key); [`OllamaConfig::from_env`] reads it
//! and falls back to the default local endpoint when it is unset. A placeholder
//! API key is sent in the `Authorization` header — Ollama ignores it, but the
//! shared `OpenAIProvider` requires a non-empty key string.
//!
//! Note: Ollama also has a native API at `/api/chat` (NDJSON, not SSE). This
//! provider uses the OpenAI-compatible endpoint only.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434/v1";
/// Environment variable holding the Ollama base URL (not an API key).
const ENV_VAR: &str = "OLLAMA_BASE_URL";
const PROVIDER_NAME: &str = "ollama";
/// Placeholder API key — Ollama does not authenticate, but the shared
/// `OpenAIConfig` requires a non-empty key string.
const PLACEHOLDER_API_KEY: &str = "ollama";

/// Configuration for the Ollama provider (wraps [`OpenAIConfig`]).
pub struct OllamaConfig(OpenAIConfig);

impl OllamaConfig {
    /// Create from an API key, using the default local Ollama base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full())
                .with_api_key_source(Some("none")),
        )
    }

    /// Create from the `OLLAMA_BASE_URL` environment variable.
    ///
    /// `OLLAMA_BASE_URL` holds a *base URL* (e.g.
    /// `http://127.0.0.1:11434/v1`), not an API key — Ollama is a local
    /// inference server that does not require authentication. When the variable
    /// is unset (or empty), the default local endpoint
    /// (`http://127.0.0.1:11434/v1`) is used.
    ///
    /// # Errors
    ///
    /// Never returns an error; an unset `OLLAMA_BASE_URL` falls back to the
    /// default local endpoint.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let config = Self::new(PLACEHOLDER_API_KEY);
        match std::env::var(ENV_VAR) {
            Ok(url) if !url.trim().is_empty() => Ok(config.with_base_url(url)),
            _ => Ok(config),
        }
    }

    /// Override the base URL (useful for tests / non-default ports).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// Ollama provider — creates [`OpenAIModel`] instances pointed at Ollama.
pub struct OllamaProvider(OpenAIProvider);

impl OllamaProvider {
    #[must_use]
    pub fn new(config: OllamaConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given Ollama model id
    /// (e.g. `"llama3.2"` or `"qwen3:4b"`).
    #[must_use]
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }

    crate::delegate_list_models!();
}
