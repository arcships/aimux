//! Google Programmable Search Engine (PSE) provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Google Custom Search JSON
//! API (`GET https://www.googleapis.com/customsearch/v1`).
//!
//! Google PSE is a modality-specific provider: it exposes a web search
//! protocol and does not support language models. Authentication uses an API
//! key (`GOOGLE_API_KEY`) and a search-engine ID / `cx` (`GOOGLE_CSE_ID`),
//! both passed as query parameters. The `cx` may also be supplied at call
//! time via `provider_options["google_pse"]["cx"]`.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::{HttpRequest, load_api_key, without_trailing_slash};

/// Fixed model ID for the Google PSE search model.
const MODEL_ID: &str = "google-pse-search";

/// Google PSE error response structure: `{ "error": { "code", "message" } }`.
fn google_pse_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error");
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Google PSE request failed")
                .to_string(),
            provider_code: error.and_then(|value| value.get("code")).and_then(
                |value| match value {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                },
            ),
        }
    })
}

/// Configuration for the Google PSE provider.
#[derive(Debug, Clone)]
pub struct GooglePseConfig {
    pub api_key: String,
    /// Search-engine ID (`cx`). May be `None` here and supplied per-call via
    /// `provider_options["google_pse"]["cx"]`.
    pub cx: Option<String>,
    pub base_url: String,
}

impl GooglePseConfig {
    /// Create from an API key (uses the default Google Custom Search base URL).
    /// `cx` defaults to `None`.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            cx: None,
            base_url: "https://www.googleapis.com/customsearch/v1".to_string(),
        }
    }

    /// Set the search-engine ID (`cx`).
    #[must_use]
    pub fn with_cx(mut self, cx: impl Into<String>) -> Self {
        self.cx = Some(cx.into());
        self
    }

    /// Use a custom base URL.
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from the `GOOGLE_API_KEY` and (optional) `GOOGLE_CSE_ID`
    /// environment variables. `GOOGLE_API_KEY` is required; `GOOGLE_CSE_ID`
    /// may be omitted if `cx` is supplied per-call via `provider_options`.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when `GOOGLE_API_KEY` is not set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "GOOGLE_API_KEY", "Google PSE")?;
        let cx = std::env::var("GOOGLE_CSE_ID").ok();
        Ok(Self::new(api_key).with_maybe_cx(cx))
    }

    fn with_maybe_cx(mut self, cx: Option<String>) -> Self {
        self.cx = cx;
        self
    }
}

/// Google PSE provider — creates `GooglePseSearchModel` instances.
///
/// Google PSE is a search-only provider; it does not support language models.
pub struct GooglePseProvider {
    config: GooglePseConfig,
}

impl GooglePseProvider {
    #[must_use]
    pub fn new(config: GooglePseConfig) -> Self {
        Self { config }
    }

    /// Create a search model instance.
    #[must_use]
    pub fn search_model(&self) -> GooglePseSearchModel {
        GooglePseSearchModel::new(self.config.clone())
    }
}

impl Provider for GooglePseProvider {
    fn name(&self) -> &str {
        "google_pse"
    }
}

/// Resolve the `cx` (search-engine ID): prefer the config value, then
/// `provider_options["google_pse"]["cx"]`.
fn resolve_cx(
    config_cx: Option<&str>,
    provider_options: Option<&HashMap<String, Value>>,
) -> Option<String> {
    if let Some(cx) = config_cx {
        return Some(cx.to_string());
    }
    if let Some(po) = provider_options
        && let Some(google_pse) = po.get("google_pse")
        && let Some(cx) = google_pse.get("cx").and_then(|v| v.as_str())
    {
        return Some(cx.to_string());
    }
    None
}

/// A single Google PSE result item. All fields are optional so unknown-but-
/// legal values degrade safely.
#[derive(Debug, Deserialize)]
struct GooglePseItem {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
}

/// The response from the Google Custom Search endpoint.
#[derive(Debug, Deserialize)]
struct GooglePseResponse {
    #[serde(default)]
    items: Vec<GooglePseItem>,
}

fn map_results(entries: Vec<GooglePseItem>) -> Vec<SearchResultItem> {
    entries
        .into_iter()
        .map(|r| SearchResultItem {
            title: r.title,
            url: r.link,
            content: r.snippet,
            raw_content: None,
            score: None,
            provider_metadata: None,
        })
        .collect()
}

/// A Google PSE search model.
pub struct GooglePseSearchModel {
    config: GooglePseConfig,
}

impl GooglePseSearchModel {
    #[must_use]
    pub fn new(config: GooglePseConfig) -> Self {
        Self { config }
    }

    fn endpoint(&self) -> String {
        // The base URL already includes the `/customsearch/v1` path; query
        // parameters (key, cx, q, num) are appended by the caller.
        self.config.base_url.clone()
    }
}

#[async_trait]
impl SearchModel for GooglePseSearchModel {
    fn provider(&self) -> &str {
        "google_pse"
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let cx = resolve_cx(self.config.cx.as_deref(), options.provider_options.as_ref())
            .ok_or_else(|| {
                AiMuxError::InvalidArgument(
                    "Google PSE requires a `cx` (search-engine ID). Set the `GOOGLE_CSE_ID` \
                     environment variable or pass it via `provider_options[\"google_pse\"][\"cx\"]`."
                        .to_string(),
                )
            })?;

        // Google PSE authenticates via query parameters; only forward
        // user-supplied extra headers.
        let headers: Vec<(String, String)> = options
            .headers
            .as_ref()
            .map(|extra: &HashMap<String, String>| {
                extra.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
            })
            .unwrap_or_default();

        let mut url = url::Url::parse(&self.endpoint()).map_err(|e| {
            AiMuxError::InvalidArgument(format!("invalid google_pse endpoint: {e}"))
        })?;
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair("key", &self.config.api_key)
                .append_pair("cx", &cx)
                .append_pair("q", &options.query);
            if let Some(num) = options.max_results {
                qp.append_pair("num", &num.to_string());
            }
        }

        let resp = aimux_provider_utils::get_from_api(
            HttpRequest {
                url: url.to_string(),
                headers,

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            aimux_provider_utils::create_json_response_handler::<GooglePseResponse>(),
            google_pse_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers.unwrap_or_default();

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

        Ok(SearchResult {
            results: map_results(data.items),
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
