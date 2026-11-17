//! Serper provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Serper search API
//! (`POST https://google.serper.dev/search`). Uses `X-API-KEY` header auth
//! via `SERPER_API_KEY`.
//!
//! [`create_serper`] takes [`SerperProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`SerperProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `SERPER_API_KEY`.
//! [`serper()`] is the default instance; it reads nothing and cannot fail.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel, SearchResult, SearchResultItem};
use aimux_provider_utils::AuthScheme;

use crate::shared::{EndpointConfig, define_vendor_provider, post_json_search};

/// Failed-response handler of the Serper endpoint: `message`, and `type`
/// else `code` as the provider code, from the body's `error` object, or from
/// the top level when there is none. This is the mapping the endpoint used
/// with the removed shared handler; it is not an AI SDK schema.
fn serper_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    crate::shared::message_type_error_response_handler()
}

const MODEL_ID: &str = "serper-search";

pub(crate) const DEFAULT_BASE_URL: &str = "https://google.serper.dev";
pub(crate) const API_KEY_ENV_VAR: &str = "SERPER_API_KEY";
const DEFAULT_NAME: &str = "serper";

define_vendor_provider! {
    settings: SerperProviderSettings,
    provider: SerperProvider,
    create: create_serper,
    singleton: serper,
    display: "Serper",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Header("X-API-KEY"),
    extra_headers: Vec::new(),
    package: None,
}

impl SerperProvider {
    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> SerperSearchModel {
        SerperSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(SerperProvider, search_model, |p, _id| p.search_model());

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

/// A Serper search model.
pub struct SerperSearchModel {
    config: EndpointConfig,
}

impl SerperSearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for SerperSearchModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let body = build_request_body(options);
        post_json_search(
            &self.config,
            "/search",
            options,
            body,
            serper_failed_response_handler(),
            |parsed: SerperResponse| (map_results(parsed.organic), None),
        )
        .await
    }
}
