//! Snowflake provider — a thin OpenAI-compatible wrapper.
//!
//! Exposes an OpenAI-compatible Chat Completions API at
//! `https://<account-identifier>.snowflakecomputing.com/api/v2/cortex/v1`. Provider-specific details are the base URL
//! and the `SNOWFLAKE_PAT` environment variable; everything else is delegated
//! to the shared [`OpenAIProvider`](crate::openai::OpenAIProvider).

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const DEFAULT_BASE_URL: &str =
    "https://<account-identifier>.snowflakecomputing.com/api/v2/cortex/v1";
const ENV_VAR: &str = "SNOWFLAKE_PAT";
const PROVIDER_NAME: &str = "snowflake";

/// Configuration for the Snowflake provider (wraps [`OpenAIConfig`]).
pub struct SnowflakeConfig(OpenAIConfig);

impl SnowflakeConfig {
    /// Create from an API key, using the default base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME)
                .with_profile(OpenAICompatProfile::full()),
        )
    }

    /// Create from the `SNOWFLAKE_PAT` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "Snowflake")?;
        Ok(Self::new(key))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// Snowflake provider — creates [`OpenAIModel`] instances.
pub struct SnowflakeProvider(OpenAIProvider);

impl SnowflakeProvider {
    pub fn new(config: SnowflakeConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance (e.g. `claude-sonnet-4-5`).
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for SnowflakeProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
