//! SearXNG provider — search modality only.
//!
//! Implements the `SearchModel` trait against a self-hosted SearXNG instance
//! (`GET {SEARXNG_URL}/search?q=...&format=json`).
//!
//! SearXNG is a modality-specific, unauthenticated provider: there is no API
//! key. The instance URL is supplied via the `SEARXNG_URL` environment
//! variable (required — there is no default). A `403` response typically
//! indicates that the `json` output format is not enabled on the instance.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::HttpRequest;
use aimux_provider_utils::without_trailing_slash;

/// Fixed model ID for the SearXNG search model.
const MODEL_ID: &str = "searxng-search";

/// Configuration for the SearXNG provider.
///
/// SearXNG is unauthenticated, so there is no API key — only the instance
/// base URL.
#[derive(Debug, Clone)]
pub struct SearxngConfig {
    pub base_url: String,
}

impl SearxngConfig {
    /// Create from an explicit instance base URL.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: without_trailing_slash(&base_url.into()),
        }
    }

    /// Use a custom base URL.
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from the `SEARXNG_URL` environment variable (required).
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when `SEARXNG_URL` is not set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let base_url = std::env::var("SEARXNG_URL").map_err(|_| {
            AiMuxError::InvalidArgument(
                "SEARXNG_URL environment variable is required for SearXNG".to_string(),
            )
        })?;
        Ok(Self::new(base_url))
    }
}

/// SearXNG provider — creates `SearxngSearchModel` instances.
///
/// SearXNG is a search-only provider; it does not support language models.
pub struct SearxngProvider {
    config: SearxngConfig,
}

impl SearxngProvider {
    #[must_use]
    pub fn new(config: SearxngConfig) -> Self {
        Self { config }
    }

    /// Create a search model instance.
    #[must_use]
    pub fn search_model(&self) -> SearxngSearchModel {
        SearxngSearchModel::new(self.config.clone())
    }
}

impl Provider for SearxngProvider {
    fn name(&self) -> &str {
        "searxng"
    }
}

/// A single SearXNG result entry. All fields are optional so unknown-but-legal
/// values degrade safely.
#[derive(Debug, Deserialize)]
struct SearxngResult {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    content: Option<String>,
    // `engine` and other extra fields returned by SearXNG are ignored.
    #[serde(default)]
    score: Option<f64>,
}

/// The response from the SearXNG `/search` endpoint.
#[derive(Debug, Deserialize)]
struct SearxngResponse {
    #[serde(default)]
    results: Vec<SearxngResult>,
}

fn map_results(entries: Vec<SearxngResult>) -> Vec<SearchResultItem> {
    entries
        .into_iter()
        .map(|r| SearchResultItem {
            title: r.title,
            url: r.url,
            content: r.content,
            raw_content: None,
            score: r.score,
            provider_metadata: None,
        })
        .collect()
}

/// A SearXNG search model.
pub struct SearxngSearchModel {
    config: SearxngConfig,
}

impl SearxngSearchModel {
    #[must_use]
    pub fn new(config: SearxngConfig) -> Self {
        Self { config }
    }

    fn endpoint(&self) -> String {
        format!("{}/search", self.config.base_url)
    }
}

#[async_trait]
impl SearchModel for SearxngSearchModel {
    fn provider(&self) -> &str {
        "searxng"
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        // SearXNG is unauthenticated; only forward user-supplied extra headers.
        let headers: Vec<(String, String)> = options
            .headers
            .as_ref()
            .map(|extra: &HashMap<String, String>| {
                extra.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
            })
            .unwrap_or_default();

        let mut url = url::Url::parse(&self.endpoint())
            .map_err(|e| AiMuxError::InvalidArgument(format!("invalid searxng endpoint: {e}")))?;
        url.query_pairs_mut()
            .append_pair("q", &options.query)
            .append_pair("format", "json");

        let resp = aimux_provider_utils::get_from_api(
            HttpRequest {
                url: url.to_string(),
                headers,

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            aimux_provider_utils::create_json_response_handler::<SearxngResponse>(),
            aimux_provider_utils::create_status_code_error_response_handler(),
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
