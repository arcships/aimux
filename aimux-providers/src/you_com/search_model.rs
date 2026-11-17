use async_trait::async_trait;
use serde::Deserialize;

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel, SearchResult, SearchResultItem};

use crate::shared::{EndpointConfig, get_query_search};

/// Fixed model id for the You.com search model.
const MODEL_ID: &str = "youcom-search";

/// Default number of results when `max_results` is unset.
const DEFAULT_COUNT: u32 = 10;

/// Failed-response handler of the You.com endpoint: `message`, and `type`
/// else `code` as the provider code, from the body's `error` object, or from
/// the top level when there is none. This is the mapping the endpoint used
/// with the removed shared handler; it is not an AI SDK schema.
fn you_com_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    crate::shared::message_type_error_response_handler()
}

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

/// A You.com search model.
pub struct YouComSearchModel {
    config: EndpointConfig,
}

impl YouComSearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for YouComSearchModel {
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
            Some("/v1/search"),
            options,
            |url| {
                url.query_pairs_mut()
                    .append_pair("query", &options.query)
                    .append_pair("count", &count.to_string());
            },
            you_com_failed_response_handler(),
            |data: YoucomSearchResponse| (data.results.into_iter().map(map_result).collect(), None),
        )
        .await
    }
}
