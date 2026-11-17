//! You.com search provider — implements the `SearchModel` trait.
//!
//! Implements the You.com (YDC Index) search API
//! (`GET https://ydc-index.io/v1/search?query=...&count=...`).
//!
//! Authentication uses the `X-API-Key` header (env `YDC_API_KEY`). Note the
//! base URL is `ydc-index.io`, not `you.com`. You.com is a search-only
//! provider: it exposes a web search protocol (query → results) and does not
//! support language models.
//!
//! [`create_you_com`] takes [`YouComProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`YouComProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `YDC_API_KEY`.
//! [`you_com()`] is the default instance; it reads nothing and cannot fail.

use async_trait::async_trait;
use serde::Deserialize;

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel, SearchResult, SearchResultItem};
use aimux_provider_utils::AuthScheme;

use crate::shared::{EndpointConfig, define_vendor_provider, get_query_search};

/// Fixed model id for the You.com search model.
/// Failed-response handler of the You.com endpoint: `message`, and `type`
/// else `code` as the provider code, from the body's `error` object, or from
/// the top level when there is none. This is the mapping the endpoint used
/// with the removed shared handler; it is not an AI SDK schema.
fn you_com_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    crate::shared::message_type_error_response_handler()
}

const MODEL_ID: &str = "youcom-search";

/// Default number of results when `max_results` is unset.
const DEFAULT_COUNT: u32 = 10;

// ── Config ───────────────────────────────────────────────────────────────────

pub(crate) const DEFAULT_BASE_URL: &str = "https://ydc-index.io";
pub(crate) const API_KEY_ENV_VAR: &str = "YDC_API_KEY";
const DEFAULT_NAME: &str = "you_com";

define_vendor_provider! {
    settings: YouComProviderSettings,
    provider: YouComProvider,
    create: create_you_com,
    singleton: you_com,
    display: "You.com",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Header("X-API-Key"),
    extra_headers: Vec::new(),
    package: None,
}

impl YouComProvider {
    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> YouComSearchModel {
        YouComSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(YouComProvider, search_model, |p, _id| p.search_model());

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
