//! Cohere provider.
//!
//! Implements Cohere's v2 chat API with its own message format (not
//! OpenAI-compatible). Supports text generation, streaming, tool calls,
//! and reasoning (thinking).

pub mod convert;
pub mod embedding;
mod model;
pub mod reranking;
mod types;

pub use embedding::CohereEmbeddingModel;
pub use reranking::CohereRerankingModel;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::{load_api_key, without_trailing_slash};

/// Configuration for the Cohere provider.
#[derive(Debug, Clone)]
pub struct CohereConfig {
    pub api_key: String,
    pub base_url: String,
}

impl CohereConfig {
    /// Create from an API key (uses default Cohere base URL).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.cohere.com/v2".to_string(),
        }
    }

    /// Use a custom base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from environment variable `COHERE_API_KEY`.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "COHERE_API_KEY", "Cohere")?;
        Ok(Self::new(api_key))
    }
}

/// Cohere provider — creates `CohereModel` instances.
pub struct CohereProvider {
    config: CohereConfig,
}

impl CohereProvider {
    pub fn new(config: CohereConfig) -> Self {
        Self { config }
    }

    /// Create a model instance for the given model name (e.g. `"command-r-plus"`).
    pub fn model(&self, model_id: &str) -> model::CohereModel {
        model::CohereModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a reranking model instance for the given model name (e.g.
    /// `"rerank-english-v3.0"`).
    pub fn reranking_model(&self, model_id: &str) -> reranking::CohereRerankingModel {
        reranking::CohereRerankingModel::new(
            model_id.to_string(),
            self.config.clone(),
        )
    }

    /// Create an embedding model instance for the given model name (e.g.
    /// `"embed-english-v3.0"`).
    pub fn embedding_model(&self, model_id: &str) -> embedding::CohereEmbeddingModel {
        embedding::CohereEmbeddingModel::new(
            model_id.to_string(),
            self.config.clone(),
        )
    }
}

impl Provider for CohereProvider {
    fn name(&self) -> &str {
        "cohere"
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}
