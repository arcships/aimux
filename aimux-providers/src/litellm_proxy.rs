//! LiteLLM Proxy provider — a thin OpenAI-compatible wrapper.
//!
//! See <litellm.ai> for API documentation. Exposes an OpenAI-compatible
//! Chat Completions API at `http://127.0.0.1:4000/v1`. The `LITELLM_PROXY_API_KEY` environment
//! variable holds a *base URL* (not an API key); when unset, the default
//! endpoint is used. A placeholder API key is sent in the `Authorization`
//! header — the shared `OpenAIConfigProvider` requires a non-empty key string.

use aimux_core::error::AiMuxError;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIConfigProvider, OpenAIModel};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:4000/v1";
const ENV_VAR: &str = "LITELLM_PROXY_API_KEY";
const PROVIDER_NAME: &str = "litellm_proxy";
const PLACEHOLDER_API_KEY: &str = "litellm_proxy";

pub struct LitellmProxyConfig(OpenAIConfig);

impl LitellmProxyConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `LITELLM_PROXY_API_KEY` environment variable (which holds
    /// a base URL), falling back to `http://127.0.0.1:4000/v1`.
    ///
    /// # Errors
    ///
    /// Never returns an error; an unset or empty variable falls back to the
    /// default local endpoint.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let config = Self::new(PLACEHOLDER_API_KEY);
        match std::env::var(ENV_VAR) {
            Ok(url) if !url.trim().is_empty() => Ok(config.with_base_url(url)),
            _ => Ok(config),
        }
    }

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

pub struct LitellmProxyProvider(OpenAIConfigProvider);

impl LitellmProxyProvider {
    #[must_use]
    pub fn new(config: LitellmProxyConfig) -> Self {
        Self(OpenAIConfigProvider::new(config.0))
    }

    #[must_use]
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

crate::impl_single_modality_provider!(LitellmProxyProvider, language_model, |p, id| p.model(id));

crate::delegate_list_models!(LitellmProxyProvider);
