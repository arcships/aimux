//! DataForSEO search provider — implements the `SearchModel` trait.
//!
//! Implements the DataForSEO SERP Google Organic Live Advanced API
//! (`POST https://api.dataforseo.com/v3/serp/google/organic/live/advanced`).
//!
//! Authentication uses HTTP Basic auth with `DATAFORSEO_LOGIN` and
//! `DATAFORSEO_PASSWORD` credentials. The request body is a JSON array of
//! task objects and the response is deeply nested. DataForSEO is a
//! search-only provider that does not support language models.

use std::collections::HashMap;

use async_trait::async_trait;
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_core::shared::SharedHeaders;

use aimux_provider_utils::response::ErrorStructure;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, send, without_trailing_slash,
};

/// Provider canonical name.
const PROVIDER_NAME: &str = "dataforseo";

/// Fixed model id for the DataForSEO search model.
const MODEL_ID: &str = "dataforseo-search";

/// Default result depth when `max_results` is unset.
const DEFAULT_DEPTH: u32 = 10;

/// Fixed `max_credits` budget per request.
const MAX_CREDITS: u32 = 1;

/// DataForSEO error response structure: `{ status_code, status_message }`.
const DATAFORSEO_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["status_message"],
    type_path: &["status_code"],
};

// ── Config ───────────────────────────────────────────────────────────────────

/// Configuration for the DataForSEO provider.
///
/// Holds HTTP Basic credentials (`login` + `password`). The `Debug`
/// implementation redacts both so credentials never appear in logs or error
/// messages.
#[derive(Clone)]
pub struct DataforseoConfig {
    login: String,
    password: String,
    base_url: String,
}

impl DataforseoConfig {
    /// Create from explicit login + password (uses the default DataForSEO base URL).
    pub fn new(login: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            login: login.into(),
            password: password.into(),
            base_url: "https://api.dataforseo.com".to_string(),
        }
    }

    /// Use a custom base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from the `DATAFORSEO_LOGIN` + `DATAFORSEO_PASSWORD` environment variables.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let login = std::env::var("DATAFORSEO_LOGIN").map_err(|_| {
            AiMuxError::Auth(
                "No DataForSEO login found. Please set the `DATAFORSEO_LOGIN` environment variable."
                    .to_string(),
            )
        })?;
        let password = std::env::var("DATAFORSEO_PASSWORD").map_err(|_| {
            AiMuxError::Auth(
                "No DataForSEO password found. Please set the `DATAFORSEO_PASSWORD` environment variable."
                    .to_string(),
            )
        })?;
        Ok(Self::new(login, password))
    }
}

impl std::fmt::Debug for DataforseoConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataforseoConfig")
            .field("login", &"<redacted>")
            .field("password", &"<redacted>")
            .field("base_url", &self.base_url)
            .finish()
    }
}

// ── Provider ─────────────────────────────────────────────────────────────────

/// DataForSEO provider — creates [`DataforseoSearchModel`] instances.
///
/// DataForSEO is a search-only provider; it does not support language models.
pub struct DataforseoProvider {
    config: DataforseoConfig,
}

impl DataforseoProvider {
    pub fn new(config: DataforseoConfig) -> Self {
        Self { config }
    }

    /// Create a search model instance.
    pub fn search_model(&self) -> DataforseoSearchModel {
        DataforseoSearchModel::new(self.config.clone())
    }
}

impl Provider for DataforseoProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }
}

// ── Request builder ──────────────────────────────────────────────────────────

/// Resolve the `depth` request field from the call options.
///
/// Pure function: maps `max_results` to the DataForSEO `depth` field (the
/// number of organic results to return), defaulting to [`DEFAULT_DEPTH`] when
/// unset.
fn resolve_depth(max_results: Option<u32>) -> u32 {
    max_results.unwrap_or(DEFAULT_DEPTH)
}

/// Build the DataForSEO request body (a JSON array of one task object).
///
/// Pure function. The body shape is:
/// `[{"keyword": <query>, "max_credits": 1, "depth": <depth>}]`.
fn build_request_body(query: &str, depth: u32) -> Value {
    json!([{ "keyword": query, "max_credits": MAX_CREDITS, "depth": depth }])
}

// ── Response types ───────────────────────────────────────────────────────────

/// The response from the DataForSEO live/advanced endpoint.
///
/// The result list is nested as `tasks[].result[].organic[]`; only the fields
/// used by the trait are deserialized, and extra fields are ignored so
/// unknown-but-legal values degrade safely.
#[derive(Debug, Deserialize)]
struct DataforseoResponse {
    #[serde(default)]
    tasks: Vec<DataforseoTask>,
}

#[derive(Debug, Deserialize)]
struct DataforseoTask {
    #[serde(default)]
    result: Vec<DataforseoTaskResult>,
}

#[derive(Debug, Deserialize)]
struct DataforseoTaskResult {
    #[serde(default)]
    organic: Vec<DataforseoOrganic>,
}

#[derive(Debug, Deserialize)]
struct DataforseoOrganic {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// Map a DataForSEO organic result into a [`SearchResultItem`].
///
/// Field mapping: `title` → `title`, `url` → `url`, `description` → `content`.
fn map_result(r: DataforseoOrganic) -> SearchResultItem {
    SearchResultItem {
        title: r.title,
        url: r.url,
        content: r.description,
        raw_content: None,
        score: None,
        provider_metadata: None,
    }
}

// ── Search model ─────────────────────────────────────────────────────────────

/// A DataForSEO search model.
pub struct DataforseoSearchModel {
    config: DataforseoConfig,
}

impl DataforseoSearchModel {
    pub fn new(config: DataforseoConfig) -> Self {
        Self { config }
    }

    /// Build the HTTP Basic `Authorization` header value.
    fn basic_auth(&self) -> String {
        let credentials = format!("{}:{}", self.config.login, self.config.password);
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes());
        format!("Basic {}", encoded)
    }

    fn build_headers(&self, extra: Option<&SharedHeaders>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), self.basic_auth());
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/v3/serp/google/organic/live/advanced",
            self.config.base_url
        )
    }
}

#[async_trait]
impl SearchModel for DataforseoSearchModel {
    fn provider(&self) -> &str {
        PROVIDER_NAME
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let depth = resolve_depth(options.max_results);
        let body = build_request_body(&options.query, depth);
        let headers: Vec<(String, String)> = self
            .build_headers(options.headers.as_ref())
            .into_iter()
            .collect();

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers,
                body: HttpBody::Json(body),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &DATAFORSEO_ERROR_STRUCTURE,
        )
        .await?;

        // Capture response headers.
        let response_headers = resp.headers;

        let raw_body: Value =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

        let data: DataforseoResponse = serde_json::from_value(raw_body.clone())
            .map_err(|e| AiMuxError::Provider(format!("failed to parse search response: {e}")))?;

        // Flatten tasks[].result[].organic[] preserving provider order.
        let results: Vec<SearchResultItem> = data
            .tasks
            .into_iter()
            .flat_map(|t| t.result)
            .flat_map(|r| r.organic)
            .map(map_result)
            .collect();

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
