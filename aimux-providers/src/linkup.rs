//! Linkup provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Linkup search API
//! (`POST https://api.linkup.so/v1/search`).
//!
//! Linkup is a modality-specific provider: it exposes a native web search
//! protocol (query → results) and does not support language models. The
//! `depth` and `outputType` request fields are passed through via
//! `provider_options` under the `"linkup"` key.

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

fn linkup_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
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

/// Fixed model ID for the Linkup search model.
const MODEL_ID: &str = "linkup-search";

/// Linkup provider-specific search options (passed through via
/// `provider_options["linkup"]`).
#[derive(Debug, Clone, Default)]
struct LinkupOptions {
    /// Search depth: `"standard"` (default) or `"deep"`.
    depth: Option<String>,
    /// Output type: `"searchResults"` (default) or `"sourcedAnswer"`.
    output_type: Option<String>,
}

fn parse_linkup_options(provider_options: Option<&HashMap<String, Value>>) -> LinkupOptions {
    let mut opts = LinkupOptions::default();
    if let Some(po) = provider_options
        && let Some(linkup) = po.get("linkup")
    {
        if let Some(v) = linkup.get("depth").and_then(|v| v.as_str()) {
            opts.depth = Some(v.to_string());
        }
        if let Some(v) = linkup.get("outputType").and_then(|v| v.as_str()) {
            opts.output_type = Some(v.to_string());
        }
    }
    opts
}

/// Configuration for the Linkup provider.
#[derive(Debug, Clone)]
pub struct LinkupConfig {
    pub api_key: String,
    pub base_url: String,
}

impl LinkupConfig {
    /// Create from an API key (uses the default Linkup base URL).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.linkup.so".to_string(),
        }
    }

    /// Use a custom base URL.
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from the `LINKUP_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when `LINKUP_API_KEY` is not set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "LINKUP_API_KEY", "Linkup")?;
        Ok(Self::new(api_key))
    }
}

/// Linkup provider — creates `LinkupSearchModel` instances.
///
/// Linkup is a search-only provider; it does not support language models.
pub struct LinkupProvider {
    config: LinkupConfig,
}

impl LinkupProvider {
    #[must_use]
    pub fn new(config: LinkupConfig) -> Self {
        Self { config }
    }

    /// Create a search model instance.
    #[must_use]
    pub fn search_model(&self) -> LinkupSearchModel {
        LinkupSearchModel::new(self.config.clone())
    }
}

impl Provider for LinkupProvider {
    fn name(&self) -> &str {
        "linkup"
    }
}

/// Build the Linkup `/v1/search` request body (pure function).
fn build_request_body(options: &SearchCallOptions, linkup_options: &LinkupOptions) -> Value {
    let mut body = json!({
        "q": options.query,
        "depth": linkup_options
            .depth
            .clone()
            .unwrap_or_else(|| "standard".to_string()),
        "outputType": linkup_options
            .output_type
            .clone()
            .unwrap_or_else(|| "searchResults".to_string()),
    });
    if let Some(include) = &options.include_domains {
        body["includeDomains"] = json!(include);
    }
    if let Some(exclude) = &options.exclude_domains {
        body["excludeDomains"] = json!(exclude);
    }
    body
}

/// A single Linkup result entry (`{name, url, content}`). All fields are
/// optional so unknown-but-legal values degrade safely.
#[derive(Debug, Deserialize)]
struct LinkupResult {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

/// The response from the Linkup `/v1/search` endpoint.
///
/// Covers both output types: `searchResults` (`results`) and `sourcedAnswer`
/// (`answer` + `sources`). Fields not present for a given output type are
/// `None` by default.
#[derive(Debug, Deserialize)]
struct LinkupResponse {
    #[serde(default)]
    results: Option<Vec<LinkupResult>>,
    #[serde(default)]
    answer: Option<String>,
    #[serde(default)]
    sources: Option<Vec<LinkupResult>>,
}

fn map_results(entries: Vec<LinkupResult>) -> Vec<SearchResultItem> {
    entries
        .into_iter()
        .map(|r| SearchResultItem {
            title: r.name,
            url: r.url,
            content: r.content,
            raw_content: None,
            score: None,
            provider_metadata: None,
        })
        .collect()
}

/// A Linkup search model.
pub struct LinkupSearchModel {
    config: LinkupConfig,
}

impl LinkupSearchModel {
    #[must_use]
    pub fn new(config: LinkupConfig) -> Self {
        Self { config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
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
impl SearchModel for LinkupSearchModel {
    fn provider(&self) -> &str {
        "linkup"
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let linkup_options = parse_linkup_options(options.provider_options.as_ref());
        let body = build_request_body(options, &linkup_options);

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
            aimux_provider_utils::create_json_response_handler::<LinkupResponse>(),
            linkup_failed_response_handler(),
        )
        .await?;

        // Capture response headers.
        let response_headers = resp.response_headers.unwrap_or_default();

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

        // Prefer `results` (searchResults); fall back to `sources`
        // (sourcedAnswer), which also carries an `answer`.
        let (results, answer) = if let Some(results) = data.results {
            (map_results(results), None)
        } else {
            let sources = data.sources.unwrap_or_default();
            (map_results(sources), data.answer)
        };

        Ok(SearchResult {
            results,
            answer,
            provider_metadata: None,
            warnings: Vec::new(),
            response: Some(SearchResponse {
                headers: Some(response_headers),
                body: Some(raw_body),
            }),
        })
    }
}
