use async_trait::async_trait;
use serde::Deserialize;

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel, SearchResult, SearchResultItem};

use crate::shared::{EndpointConfig, get_query_search};

/// Fixed model ID for the SearXNG search model.
const MODEL_ID: &str = "searxng-search";

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
    config: EndpointConfig,
}

impl SearxngSearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for SearxngSearchModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        get_query_search(
            &self.config,
            Some("/search"),
            options,
            |url| {
                url.query_pairs_mut()
                    .append_pair("q", &options.query)
                    .append_pair("format", "json");
            },
            aimux_provider_utils::create_status_code_error_response_handler(),
            |data: SearxngResponse| (map_results(data.results), None),
        )
        .await
    }
}
