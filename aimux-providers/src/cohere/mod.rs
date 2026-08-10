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
use aimux_provider_utils::{RetryConfig, load_api_key, without_trailing_slash};

/// Configuration for the Cohere provider.
#[derive(Debug, Clone)]
pub struct CohereConfig {
    pub api_key: String,
    pub base_url: String,
    /// api_key 来源(RFC-0023):`None` = explicit;`Some("env:VAR")` = 环境变量。
    pub api_key_source: Option<String>,
    /// 重试配置(M1b)。默认 `RetryConfig::default()`。
    pub retry_config: RetryConfig,
}

impl CohereConfig {
    /// Create from an API key (uses default Cohere base URL).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.cohere.com/v2".to_string(),
            api_key_source: None,
            retry_config: RetryConfig::default(),
        }
    }

    /// Use a custom base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// 标注 api_key 来源(RFC-0023 回放重建用)。
    pub fn with_api_key_source(mut self, source: Option<&str>) -> Self {
        self.api_key_source = source.map(|s| s.to_string());
        self
    }

    /// Set the retry configuration. Pass `max_retries: 0` to disable retries.
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Create from environment variable `COHERE_API_KEY`.
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
        reranking::CohereRerankingModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create an embedding model instance for the given model name (e.g.
    /// `"embed-english-v3.0"`).
    pub fn embedding_model(&self, model_id: &str) -> embedding::CohereEmbeddingModel {
        embedding::CohereEmbeddingModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for CohereProvider {
    fn name(&self) -> &str {
        "cohere"
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }

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
            use aimux_provider_utils::{
                DEFAULT_ERROR_STRUCTURE, HttpBody, HttpMethod, HttpRequest, send_timed,
            };
            let resp = send_timed(
                HttpRequest {
                    method: HttpMethod::Get,
                    url,
                    headers,
                    body: HttpBody::Empty,
                    abort_signal: None,
                    call_id: None,
                    recording_context: None,
                },
                config.retry_config,
                &DEFAULT_ERROR_STRUCTURE,
                None,
            )
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
            let parsed: Resp = serde_json::from_slice(&resp.body)
                .map_err(|e| AiMuxError::Json(format!("cohere list_models: parse: {e}")))?;
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
