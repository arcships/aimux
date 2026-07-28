//! Parallel AI provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Parallel AI search API
//! (`POST https://api.parallel.ai/v1/search`).
//!
//! Parallel AI is a modality-specific provider: it exposes a native web
//! search protocol (objective + search queries → results) and does not
//! support language models. The single `query` is mapped to the
//! `search_queries` array (and used as the `objective`).

use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::response::{DEFAULT_ERROR_STRUCTURE, parse_provider_error};
use aimux_provider_utils::{load_api_key, without_trailing_slash};

/// Fixed model ID for the Parallel AI search model.
const MODEL_ID: &str = "parallel-search";

/// Configuration for the Parallel AI provider.
#[derive(Debug, Clone)]
pub struct ParallelAiConfig {
    pub api_key: String,
    pub base_url: String,
}

impl ParallelAiConfig {
    /// Create from an API key (uses the default Parallel AI base URL).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.parallel.ai".to_string(),
        }
    }

    /// Use a custom base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from the `PARALLEL_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "PARALLEL_API_KEY", "Parallel AI")?;
        Ok(Self::new(api_key))
    }
}

/// Parallel AI provider — creates `ParallelAiSearchModel` instances.
///
/// Parallel AI is a search-only provider; it does not support language models.
pub struct ParallelAiProvider {
    config: ParallelAiConfig,
    client: Client,
}

impl ParallelAiProvider {
    pub fn new(config: ParallelAiConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    /// Create a search model instance.
    pub fn search_model(&self) -> ParallelAiSearchModel {
        ParallelAiSearchModel::new(self.config.clone(), self.client.clone())
    }
}

impl Provider for ParallelAiProvider {
    fn name(&self) -> &str {
        "parallel_ai"
    }

    fn language_model(&self, _model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Err(AiMuxError::Unsupported(
            "parallel_ai does not support language models. Use search_model() instead.".to_string(),
        ))
    }
}

/// Build the Parallel AI `/v1/search` request body (pure function).
///
/// The single `query` is used both as the `objective` and as the sole entry
/// in the `search_queries` array.
fn build_request_body(options: &SearchCallOptions) -> Value {
    json!({
        "objective": options.query,
        "search_queries": [options.query],
        "mode": "advanced",
    })
}

/// A single Parallel AI result entry. `excerpts` is a list of snippet
/// strings; all fields are optional so unknown-but-legal values degrade
/// safely.
#[derive(Debug, Deserialize)]
struct ParallelAiResult {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    excerpts: Vec<String>,
}

/// The response from the Parallel AI `/v1/search` endpoint.
#[derive(Debug, Deserialize)]
struct ParallelAiResponse {
    #[serde(default)]
    results: Vec<ParallelAiResult>,
}

fn map_results(entries: Vec<ParallelAiResult>) -> Vec<SearchResultItem> {
    entries
        .into_iter()
        .map(|r| SearchResultItem {
            title: r.title,
            url: r.url,
            // Join excerpt snippets into a single content string.
            content: if r.excerpts.is_empty() {
                None
            } else {
                Some(r.excerpts.join("\n"))
            },
            raw_content: None,
            score: None,
            provider_metadata: None,
        })
        .collect()
}

/// A Parallel AI search model.
pub struct ParallelAiSearchModel {
    config: ParallelAiConfig,
    client: Client,
}

impl ParallelAiSearchModel {
    pub fn new(config: ParallelAiConfig, client: Client) -> Self {
        Self { config, client }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("x-api-key".to_string(), self.config.api_key.clone());
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/search", self.config.base_url)
    }
}

#[async_trait]
impl SearchModel for ParallelAiSearchModel {
    fn provider(&self) -> &str {
        "parallel_ai"
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let body = build_request_body(options);

        let headers = self.build_headers(options.headers.as_ref());
        let header_map: reqwest::header::HeaderMap = headers
            .iter()
            .filter_map(|(k, v)| {
                reqwest::header::HeaderName::try_from(k)
                    .ok()
                    .zip(reqwest::header::HeaderValue::try_from(v).ok())
            })
            .collect();

        let resp = self
            .client
            .post(self.endpoint())
            .header("Content-Type", "application/json")
            .headers(header_map)
            .json(&body)
            .send()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(parse_provider_error(
                status.as_u16(),
                &text,
                &DEFAULT_ERROR_STRUCTURE,
            ));
        }

        let response_headers: HashMap<String, String> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        let raw_body: Value = resp
            .json()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        let data: ParallelAiResponse = serde_json::from_value(raw_body.clone()).map_err(|e| {
            AiMuxError::Provider(format!("failed to parse parallel_ai search response: {e}"))
        })?;

        Ok(SearchResult {
            results: map_results(data.results),
            answer: None,
            provider_metadata: None,
            warnings: Vec::new(),
            response: Some(SearchResponse {
                headers: Some(response_headers),
                body: Some(raw_body),
            }),
        })
    }
}
