//! You.com search provider — implements the `SearchModel` trait.
//!
//! Implements the You.com (YDC Index) search API
//! (`GET https://ydc-index.io/v1/search?query=...&count=...`).
//!
//! Authentication uses the `X-API-Key` header (env `YDC_API_KEY`). Note the
//! base URL is `ydc-index.io`, not `you.com`. You.com is a search-only
//! provider: it exposes a web search protocol (query → results) and does not
//! support language models.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
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
const PROVIDER_NAME: &str = "you_com";

/// Fixed model id for the You.com search model.
const MODEL_ID: &str = "youcom-search";

/// Default number of results when `max_results` is unset.
const DEFAULT_COUNT: u32 = 10;

// ── Config ───────────────────────────────────────────────────────────────────

/// Configuration for the You.com provider.
///
/// The `Debug` implementation redacts the API key so credentials never appear
/// in logs or error messages.
#[derive(Clone)]
pub struct YouComConfig {
    api_key: String,
    base_url: String,
}

impl YouComConfig {
    /// Create from an API key (uses the default You.com / YDC base URL).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://ydc-index.io".to_string(),
        }
    }

    /// Use a custom base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from the `YDC_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "YDC_API_KEY", "You.com")?;
        Ok(Self::new(api_key))
    }
}

impl std::fmt::Debug for YouComConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("YouComConfig")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .finish()
    }
}

// ── Provider ─────────────────────────────────────────────────────────────────

/// You.com provider — creates [`YouComSearchModel`] instances.
///
/// You.com is a search-only provider; it does not support language models.
pub struct YouComProvider {
    config: YouComConfig,
}

impl YouComProvider {
    pub fn new(config: YouComConfig) -> Self {
        Self { config }
    }

    /// Create a search model instance.
    pub fn search_model(&self) -> YouComSearchModel {
        YouComSearchModel::new(self.config.clone())
    }
}

impl Provider for YouComProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, _model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Err(AiMuxError::Unsupported(
            "you_com does not support language models. Use search_model() instead.".to_string(),
        ))
    }
}

// ── Request builder ──────────────────────────────────────────────────────────

/// Resolve the `count` query parameter from the call options.
///
/// Pure function: maps `max_results` to the You.com `count` parameter,
/// defaulting to [`DEFAULT_COUNT`] when unset.
fn resolve_count(max_results: Option<u32>) -> u32 {
    max_results.unwrap_or(DEFAULT_COUNT)
}

// ── Response types ───────────────────────────────────────────────────────────

/// The response from the You.com `/v1/search` endpoint.
///
/// Only the fields used by the trait are deserialized; extra fields returned
/// by the API are ignored so unknown-but-legal values degrade safely.
#[derive(Debug, Deserialize)]
struct YoucomSearchResponse {
    #[serde(default)]
    results: Vec<YoucomResult>,
}

#[derive(Debug, Deserialize)]
struct YoucomResult {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
}

/// Map a You.com result into a [`SearchResultItem`].
///
/// Field mapping: `title` → `title`, `url` → `url`, `description` → `content`
/// (falling back to `snippet` when `description` is absent).
fn map_result(r: YoucomResult) -> SearchResultItem {
    SearchResultItem {
        title: r.title,
        url: r.url,
        content: r.description.or(r.snippet),
        raw_content: None,
        score: None,
        provider_metadata: None,
    }
}

// ── Search model ─────────────────────────────────────────────────────────────

/// A You.com search model.
pub struct YouComSearchModel {
    config: YouComConfig,
}

impl YouComSearchModel {
    pub fn new(config: YouComConfig) -> Self {
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

    fn endpoint(&self) -> String {
        format!("{}/v1/search", self.config.base_url)
    }
}

#[async_trait]
impl SearchModel for YouComSearchModel {
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
            .map_err(|e| AiMuxError::Provider(format!("invalid you_com endpoint: {e}")))?;
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

        let data: YoucomSearchResponse = serde_json::from_value(raw_body.clone())
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
