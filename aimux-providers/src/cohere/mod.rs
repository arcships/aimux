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

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::reranking_model::RerankingModel;
use aimux_provider_utils::{load_api_key, without_trailing_slash};
use serde_json::Value;
use std::sync::Arc;

pub(crate) fn cohere_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: None,
        }
    })
}

/// Configuration for the Cohere provider.
#[derive(Debug, Clone)]
pub struct CohereConfig {
    pub api_key: String,
    pub base_url: String,
    /// api_key 来源(RFC-0023):`None` = explicit;`Some("env:VAR")` = 环境变量。
    pub api_key_source: Option<String>,
}

impl CohereConfig {
    /// Create from an API key (uses default Cohere base URL).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.cohere.com/v2".to_string(),
            api_key_source: None,
        }
    }

    /// Use a custom base URL.
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// 标注 api_key 来源(RFC-0023 回放重建用)。
    #[must_use]
    pub fn with_api_key_source(mut self, source: Option<&str>) -> Self {
        self.api_key_source = source.map(std::string::ToString::to_string);
        self
    }

    /// Create from environment variable `COHERE_API_KEY`.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when `COHERE_API_KEY` is not set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "COHERE_API_KEY", "Cohere")?;
        Ok(Self::new(api_key).with_api_key_source(Some("env:COHERE_API_KEY")))
    }
}

/// Cohere provider — creates `CohereModel` instances.
pub struct CohereProvider {
    config: CohereConfig,
}

impl CohereProvider {
    #[must_use]
    pub fn new(config: CohereConfig) -> Self {
        Self { config }
    }

    /// Create a model instance for the given model name (e.g. `"command-r-plus"`).
    #[must_use]
    pub fn model(&self, model_id: &str) -> model::CohereModel {
        model::CohereModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a reranking model instance for the given model name (e.g.
    /// `"rerank-english-v3.0"`).
    #[must_use]
    pub fn reranking_model(&self, model_id: &str) -> reranking::CohereRerankingModel {
        reranking::CohereRerankingModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create an embedding model instance for the given model name (e.g.
    /// `"embed-english-v3.0"`).
    #[must_use]
    pub fn embedding_model(&self, model_id: &str) -> embedding::CohereEmbeddingModel {
        embedding::CohereEmbeddingModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for CohereProvider {
    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(Arc::new(self.model(model_id)))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Ok(Arc::new(self.embedding_model(model_id)))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }

    fn reranking_model(
        &self,
        model_id: &str,
    ) -> Option<Result<Arc<dyn RerankingModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.reranking_model(model_id))))
    }
}

impl ProviderDiscovery for CohereProvider {
    /// List models via `GET {base_url}/models` (Cohere v2, RFC-0027).
    fn list_models(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Vec<aimux_core::model_catalogue::RuntimeModel>, AiMuxError>,
                > + Send
                + '_,
        >,
    > {
        let config = self.config.clone();
        Box::pin(async move {
            let base = config.base_url.trim_end_matches('/');
            let url = format!("{base}/models");
            let headers = vec![
                (
                    "Authorization".to_string(),
                    format!("Bearer {}", config.api_key),
                ),
                ("Content-Type".to_string(), "application/json".to_string()),
            ];
            use aimux_provider_utils::HttpRequest;
            // Retry rationale: see `openai::model::execute_list_models`.
            let resp = aimux_core::retry::prepare_retries(None, None)
                .retry(|| {
                    aimux_provider_utils::get_from_api(
                        HttpRequest {
                            url: url.clone(),
                            headers: headers.clone(),
                            abort_signal: None,
                            call_id: None,
                            recording_context: None,
                            ..Default::default()
                        },
                        aimux_provider_utils::create_json_response_handler(),
                        cohere_failed_response_handler(),
                    )
                })
                .await?;
            // Cohere v2: { models: [{ name, endpoints, ... }] }
            #[derive(serde::Deserialize)]
            struct Resp {
                #[serde(default)]
                models: Vec<Entry>,
            }
            #[derive(serde::Deserialize)]
            struct Entry {
                name: String,
                #[serde(default)]
                endpoints: Option<Vec<String>>,
            }
            let parsed: Resp = resp.value;
            let runtime: Vec<aimux_core::model_catalogue::RuntimeModel> = parsed
                .models
                .into_iter()
                // Only include chat-capable models (endpoints contain "chat").
                .filter(|e| {
                    e.endpoints
                        .as_ref()
                        .is_none_or(|eps| eps.iter().any(|e| e == "chat"))
                })
                .map(|e| aimux_core::model_catalogue::RuntimeModel {
                    id: e.name,
                    owned_by: Some("cohere".to_string()),
                    created: None,
                })
                .collect();
            Ok(runtime)
        })
    }
}
