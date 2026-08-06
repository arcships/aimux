//! Voyage AI provider.
//!
//! Implements the `EmbeddingModel` trait against the Voyage AI API
//! (`api.voyageai.com/v1/embeddings`).

pub mod embedding;
pub mod reranking;

pub use embedding::VoyageEmbeddingModel;
pub use reranking::VoyageRerankingModel;

use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;
use aimux_provider_utils::{load_api_key, without_trailing_slash};

/// Configuration for the Voyage AI provider.
#[derive(Debug, Clone)]
pub struct VoyageConfig {
    pub api_key: String,
    pub base_url: String,
}

impl VoyageConfig {
    /// Create from an API key (uses default Voyage AI base URL).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.voyageai.com/v1".to_string(),
        }
    }

    /// Use a custom base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from the `VOYAGE_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "VOYAGE_API_KEY", "Voyage")?;
        Ok(Self::new(api_key))
    }
}

/// Voyage AI provider — creates `VoyageEmbeddingModel` instances.
pub struct VoyageProvider {
    config: VoyageConfig,
}

impl VoyageProvider {
    pub fn new(config: VoyageConfig) -> Self {
        Self { config }
    }

    /// Create an embedding model instance for the given model name (e.g.
    /// `"voyage-3.5"`).
    pub fn embedding_model(&self, model_id: &str) -> VoyageEmbeddingModel {
        VoyageEmbeddingModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a reranking model instance for the given model name (e.g.
    /// `"rerank-2.5"`).
    pub fn reranking_model(&self, model_id: &str) -> reranking::VoyageRerankingModel {
        reranking::VoyageRerankingModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for VoyageProvider {
    fn name(&self) -> &str {
        "voyage"
    }
}
