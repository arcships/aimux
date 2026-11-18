use async_trait::async_trait;
use serde::Deserialize;

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel, SearchResult, SearchResultItem};

use crate::shared::{EndpointConfig, get_query_search};

/// Fixed model id for the TinyFish search model.
const MODEL_ID: &str = "tinyfish-search";

/// Default number of results when `max_results` is unset.
const DEFAULT_COUNT: u32 = 10;

/// Failed-response handler of the TinyFish endpoint: `message`, and `type`
/// else `code` as the provider code, from the body's `error` object, or from
/// the top level when there is none. This is the mapping the endpoint used
/// with the removed shared handler; it is not an AI SDK schema.
fn tinyfish_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    crate::shared::message_type_error_response_handler()
}

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

/// A TinyFish search model.
pub struct TinyfishSearchModel {
    config: EndpointConfig,
}

impl TinyfishSearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for TinyfishSearchModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let count = options.max_results.unwrap_or(DEFAULT_COUNT);
        get_query_search(
            &self.config,
            None,
            options,
            |url| {
                url.query_pairs_mut()
                    .append_pair("query", &options.query)
                    .append_pair("count", &count.to_string());
            },
            tinyfish_failed_response_handler(),
            |data: TinyfishSearchResponse| {
                (data.results.into_iter().map(map_result).collect(), None)
            },
        )
        .await
    }
}
