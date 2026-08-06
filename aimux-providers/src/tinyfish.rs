//! TinyFish search provider — implements the `SearchModel` trait.
//!
//! Implements the TinyFish search API
//! (`GET https://api.search.tinyfish.ai?query=...&count=...`).
//!
//! Authentication uses the `X-API-Key` header (env `TINYFISH_API_KEY`).
//! TinyFish is a search-only provider: it exposes a web search protocol
//! (query → results) and does not support language models.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_core::shared::SharedHeaders;

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, load_api_key, send, without_trailing_slash,
};

/// Provider canonical name.
const PROVIDER_NAME: &str = "tinyfish";

/// Fixed model id for the TinyFish search model.
const MODEL_ID: &str = "tinyfish-search";

/// Default number of results when `max_results` is unset.
const DEFAULT_COUNT: u32 = 10;

// ── Config ───────────────────────────────────────────────────────────────────

/// Configuration for the TinyFish provider.
///
/// The `Debug` implementation redacts the API key so credentials never appear
/// in logs or error messages.
#[derive(Clone)]
pub struct TinyfishConfig {
    api_key: String,
    base_url: String,
}

impl TinyfishConfig {
    /// Create from an API key (uses the default TinyFish base URL).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.search.tinyfish.ai".to_string(),
        }
    }

    /// Use a custom base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from the `TINYFISH_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "TINYFISH_API_KEY", "TinyFish")?;
        Ok(Self::new(api_key))
    }
}

impl std::fmt::Debug for TinyfishConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TinyfishConfig")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .finish()
    }
}

// ── Provider ─────────────────────────────────────────────────────────────────

/// TinyFish provider — creates [`TinyfishSearchModel`] instances.
///
/// TinyFish is a search-only provider; it does not support language models.
pub struct TinyfishProvider {
    config: TinyfishConfig,
}

impl TinyfishProvider {
    pub fn new(config: TinyfishConfig) -> Self {
        Self { config }
    }

    /// Create a search model instance.
    pub fn search_model(&self) -> TinyfishSearchModel {
        TinyfishSearchModel::new(self.config.clone())
    }
}

impl Provider for TinyfishProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }
}

// ── Request builder ──────────────────────────────────────────────────────────

/// Resolve the `count` query parameter from the call options.
///
/// Pure function: maps `max_results` to the TinyFish `count` parameter,
/// defaulting to [`DEFAULT_COUNT`] when unset.
fn resolve_count(max_results: Option<u32>) -> u32 {
    max_results.unwrap_or(DEFAULT_COUNT)
}

// ── Response types ───────────────────────────────────────────────────────────

/// The response from the TinyFish search endpoint.
///
/// Only the fields used by the trait are deserialized; extra fields returned
/// by the API (e.g. `query`, `total_results`, per-result `position` /
/// `site_name`) are ignored so unknown-but-legal values degrade safely.
#[derive(Debug, Deserialize)]
struct TinyfishSearchResponse {
    #[serde(default)]
    results: Vec<TinyfishResult>,
}

#[derive(Debug, Deserialize)]
struct TinyfishResult {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

/// Map a TinyFish result into a [`SearchResultItem`].
///
/// Field mapping: `title` → `title`, `url` → `url`, `snippet` → `content`.
fn map_result(r: TinyfishResult) -> SearchResultItem {
    SearchResultItem {
        title: r.title,
        url: r.url,
        content: r.snippet,
        raw_content: None,
        score: None,
        provider_metadata: None,
    }
}

// ── Search model ─────────────────────────────────────────────────────────────

/// A TinyFish search model.
pub struct TinyfishSearchModel {
    config: TinyfishConfig,
}

impl TinyfishSearchModel {
    pub fn new(config: TinyfishConfig) -> Self {
        Self { config }
    }

    fn build_headers(&self, extra: Option<&SharedHeaders>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("X-API-Key".to_string(), self.config.api_key.clone());
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    /// The search endpoint (root path; query parameters are appended at call time).
    fn endpoint(&self) -> String {
        self.config.base_url.clone()
    }
}

#[async_trait]
impl SearchModel for TinyfishSearchModel {
    fn provider(&self) -> &str {
        PROVIDER_NAME
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let count = resolve_count(options.max_results);
        let headers: Vec<(String, String)> = self
            .build_headers(options.headers.as_ref())
            .into_iter()
            .collect();

        let mut url = url::Url::parse(&self.endpoint())
            .map_err(|e| AiMuxError::Provider(format!("invalid tinyfish endpoint: {e}")))?;
        url.query_pairs_mut()
            .append_pair("query", &options.query)
            .append_pair("count", &count.to_string());

        let resp = send(
            HttpRequest {
                method: HttpMethod::Get,
                url: url.to_string(),
                headers,
                body: HttpBody::Empty,

                abort_signal: options.abort_signal.clone(),
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        // Capture response headers.
        let response_headers = resp.headers;

        let raw_body: Value =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

        let data: TinyfishSearchResponse = serde_json::from_value(raw_body.clone())
            .map_err(|e| AiMuxError::Provider(format!("failed to parse search response: {e}")))?;

        let results: Vec<SearchResultItem> = data.results.into_iter().map(map_result).collect();

        Ok(SearchResult {
            results,
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
