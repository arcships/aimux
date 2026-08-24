//! Serper provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Serper search API
//! (`POST https://google.serper.dev/search`). Uses `X-API-KEY` header auth
//! via `SERPER_API_KEY`.

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

fn serper_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
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

const MODEL_ID: &str = "serper-search";

/// Configuration for the Serper provider.
#[derive(Debug, Clone)]
pub struct SerperConfig {
    pub api_key: String,
    pub base_url: String,
}

impl SerperConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://google.serper.dev".to_string(),
        }
    }

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from the `SERPER_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when the environment variable is not
    /// set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "SERPER_API_KEY", "Serper")?;
        Ok(Self::new(api_key))
    }
}

/// Serper provider — search-only.
pub struct SerperProvider {
    config: SerperConfig,
}

impl SerperProvider {
    #[must_use]
    pub fn new(config: SerperConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn search_model(&self) -> SerperSearchModel {
        SerperSearchModel::new(self.config.clone())
    }
}

impl Provider for SerperProvider {
    fn name(&self) -> &str {
        "serper"
    }
}

fn build_request_body(options: &SearchCallOptions) -> Value {
    let mut body = json!({
        "q": options.query,
    });
    if let Some(max) = options.max_results {
        body["num"] = json!(max);
    }
    body
}

#[derive(Debug, Deserialize)]
struct SerperOrganicResult {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SerperResponse {
    #[serde(default)]
    organic: Vec<SerperOrganicResult>,
}

fn map_results(entries: Vec<SerperOrganicResult>) -> Vec<SearchResultItem> {
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

/// Serper search model — implements `SearchModel`.
pub struct SerperSearchModel {
    config: SerperConfig,
}

impl SerperSearchModel {
    #[must_use]
    pub fn new(config: SerperConfig) -> Self {
        Self { config }
    }

    fn endpoint(&self) -> String {
        format!("{}/search", self.config.base_url)
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> Vec<(String, String)> {
        let mut headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-API-KEY".to_string(), self.config.api_key.clone()),
        ];
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.push((k.clone(), v.clone()));
            }
        }
        headers
    }
}

#[async_trait]
impl SearchModel for SerperSearchModel {
    fn provider(&self) -> &str {
        "serper"
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let body = build_request_body(options);
        let headers = self.build_headers(options.headers.as_ref());

        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: self.endpoint(),
                headers,

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            body,
            aimux_provider_utils::create_json_response_handler(),
            serper_failed_response_handler(),
        )
        .await?;
        let response_headers = resp.response_headers.unwrap_or_default();
        let response_body = resp.raw_value;
        let parsed: SerperResponse = resp.value;

        Ok(SearchResult {
            results: map_results(parsed.organic),
            answer: None,
            provider_metadata: None,
            warnings: Vec::new(),
            response: Some(SearchResponse {
                headers: Some(response_headers),
                body: response_body,
            }),
        })
    }
}
