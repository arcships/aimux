//! Jev provider factory and configuration.

use std::collections::HashMap;
use std::sync::Arc;

use aimux_core::decision_model::DecisionModel;
use aimux_core::{AiMuxError, Provider};
use aimux_provider_utils::load_api_key;

use crate::decision::DecisionProbabilitySource;

use super::decision_model::JevDecisionModel;

#[derive(Clone)]
pub struct JevConfig {
    pub api_key: String,
    /// Full POST URL for the official API, or an explicitly configured proxy.
    pub endpoint: String,
    pub headers: HashMap<String, String>,
    pub probability_source: DecisionProbabilitySource,
}

impl JevConfig {
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            endpoint: "https://api.typesafe.ai/v1/systemone".into(),
            headers: HashMap::new(),
            probability_source: DecisionProbabilitySource::Native,
        }
    }

    /// # Errors
    /// Returns `InvalidArgument` if `TYPESAFE_API_KEY` is unavailable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        Ok(Self::new(load_api_key(None, "TYPESAFE_API_KEY", "Jev")?))
    }

    /// Override the complete endpoint (for example a trusted API proxy).
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }
}

pub struct JevProvider {
    config: JevConfig,
}

impl JevProvider {
    #[must_use]
    pub fn new(config: JevConfig) -> Self {
        Self { config }
    }
}

impl Provider for JevProvider {
    crate::__unsupported_required_model!(language_model, LanguageModel, "languageModel");
    crate::__unsupported_required_model!(embedding_model, EmbeddingModel, "embeddingModel");
    crate::__unsupported_required_model!(image_model, ImageModel, "imageModel");

    fn decision_model(&self, model_id: &str) -> Option<Result<Arc<dyn DecisionModel>, AiMuxError>> {
        if model_id.trim().is_empty() {
            return Some(Err(AiMuxError::invalid_argument(
                "decision model ID cannot be empty",
            )));
        }
        Some(Ok(Arc::new(JevDecisionModel::from_config(
            self.config.clone(),
            model_id.into(),
        ))))
    }
}
