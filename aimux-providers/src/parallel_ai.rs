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
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::{HttpRequest, load_api_key, without_trailing_slash};

fn parallel_ai_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error").unwrap_or(data);
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: error
                .get("type")
                .or_else(|| error.get("code"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    })
}

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
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from the `PARALLEL_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when `PARALLEL_API_KEY` is not set.
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
}

impl ParallelAiProvider {
    #[must_use]
    pub fn new(config: ParallelAiConfig) -> Self {
        Self { config }
    }

    /// Create a search model instance.
    #[must_use]
    pub fn search_model(&self) -> ParallelAiSearchModel {
        ParallelAiSearchModel::new(self.config.clone())
    }
}

impl Provider for ParallelAiProvider {
    fn name(&self) -> &str {
        "parallel_ai"
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
}

impl ParallelAiSearchModel {
    #[must_use]
    pub fn new(config: ParallelAiConfig) -> Self {
        Self { config }
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

        let headers: Vec<(String, String)> = self
            .build_headers(options.headers.as_ref())
            .into_iter()
            .collect();

        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: self.endpoint(),
                headers,

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
                response_timeout: None,
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
            body,
            aimux_provider_utils::create_json_response_handler::<ParallelAiResponse>(),
            parallel_ai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers.unwrap_or_default();

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

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
