//! Mistral AI provider.
//!
//! OpenAI-compatible chat completions API with Mistral-specific differences:
//! - Tool choice uses `"any"` instead of `"required"`
//! - Content can be a string or an array of typed parts (text, thinking, image_url)
//! - Usage supports `num_cached_tokens`
//! - Finish reasons include `model_length`

pub mod convert;
pub mod embedding;
mod model;
mod types;

pub use embedding::MistralEmbeddingModel;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::{load_api_key, without_trailing_slash};

/// Configuration for the Mistral provider.
#[derive(Debug, Clone)]
pub struct MistralConfig {
    pub api_key: String,
    pub base_url: String,
}

impl MistralConfig {
    /// Create from an API key (uses default Mistral base URL).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.mistral.ai/v1".to_string(),
        }
    }

    /// Use a custom base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from environment variable `MISTRAL_API_KEY`.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "MISTRAL_API_KEY", "Mistral")?;
        Ok(Self::new(api_key))
    }
}

/// Mistral provider — creates `MistralModel` instances.
pub struct MistralProvider {
    config: MistralConfig,
}

impl MistralProvider {
    pub fn new(config: MistralConfig) -> Self {
        Self { config }
    }

    /// Create a model instance for the given model name (e.g. `"mistral-small-latest"`).
    pub fn model(&self, model_id: &str) -> model::MistralModel {
        model::MistralModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create an embedding model instance for the given model name (e.g.
    /// `"mistral-embed"`).
    pub fn embedding_model(&self, model_id: &str) -> embedding::MistralEmbeddingModel {
        embedding::MistralEmbeddingModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for MistralProvider {
    fn name(&self) -> &str {
        "mistral"
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
