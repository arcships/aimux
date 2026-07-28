//! xAI (Grok) provider — a thin wrapper with xAI-specific behaviour.
//!
//! xAI exposes an OpenAI-compatible Chat Completions API at
//! `https://api.x.ai/v1`. While the wire format is OpenAI-compatible, xAI has
//! enough provider-specific behaviour (reasoning content extraction, citations,
//! search parameters, xai-keyed provider options, non-inclusive cached tokens,
//! reasoning-effort model gating, 200-status errors) to warrant its own model
//! implementation ([`XaiModel`]) rather than reusing `OpenAIModel`.

pub mod convert;
mod model;
pub mod responses;
mod types;

pub use model::XaiModel;
pub use responses::XaiResponsesModel;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::OpenAIConfig;

const DEFAULT_BASE_URL: &str = "https://api.x.ai/v1";
const ENV_VAR: &str = "XAI_API_KEY";
const PROVIDER_NAME: &str = "xai";

/// Configuration for the xAI provider (wraps [`OpenAIConfig`]).
pub struct XAIConfig(OpenAIConfig);

impl XAIConfig {
    /// Create from an API key, using the default xAI base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(OpenAIConfig::new(api_key).with_base_url(DEFAULT_BASE_URL))
    }

    /// Create from the `XAI_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "xAI")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }

    /// Get the API key.
    pub(crate) fn api_key(&self) -> &str {
        &self.0.api_key
    }

    /// Get the base URL.
    pub(crate) fn base_url(&self) -> &str {
        &self.0.base_url
    }
}

/// xAI provider — creates [`XaiModel`] instances pointed at xAI.
pub struct XAIProvider {
    config: XAIConfig,
    client: reqwest::Client,
}

impl XAIProvider {
    pub fn new(config: XAIConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Create a model instance for the given xAI model id (e.g. `"grok-2"`).
    pub fn model(&self, model_id: &str) -> XaiModel {
        XaiModel::new(
            model_id.to_string(),
            XAIConfig::new(self.config.api_key()).with_base_url(self.config.base_url()),
            self.client.clone(),
        )
    }

    /// Create a Responses API model instance for the given xAI model id.
    ///
    /// Uses the xAI `/responses` endpoint with the Responses API wire format
    /// (input items, reasoning objects, provider-executed tools, etc.).
    pub fn responses_model(&self, model_id: &str) -> XaiResponsesModel {
        XaiResponsesModel::new(
            model_id.to_string(),
            XAIConfig::new(self.config.api_key()).with_base_url(self.config.base_url()),
            self.client.clone(),
        )
    }
}

impl Provider for XAIProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
