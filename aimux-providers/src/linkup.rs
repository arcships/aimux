//! Linkup provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Linkup search API
//! (`POST https://api.linkup.so/v1/search`).
//!
//! Linkup is a modality-specific provider: it exposes a native web search
//! protocol (query → results) and does not support language models. The
//! `depth` and `outputType` request fields are passed through via
//! `provider_options` under the `"linkup"` key.
//!
//! [`create_linkup`] takes [`LinkupProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`LinkupProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `LINKUP_API_KEY`.
//! [`linkup()`] is the default instance; it reads nothing and cannot fail.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel, SearchResult, SearchResultItem};
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::AuthScheme;

use crate::shared::{EndpointConfig, define_vendor_provider, namespaced_options, post_json_search};

/// Fixed model ID for the Linkup search model.
/// Failed-response handler of the Linkup endpoint: `message`, and `type`
/// else `code` as the provider code, from the body's `error` object, or from
/// the top level when there is none. This is the mapping the endpoint used
/// with the removed shared handler; it is not an AI SDK schema.
fn linkup_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    crate::shared::message_type_error_response_handler()
}

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

fn parse_linkup_options(provider_options: Option<&SharedProviderOptions>) -> LinkupOptions {
    let mut opts = LinkupOptions::default();
    if let Some(linkup) = namespaced_options(provider_options, NAMESPACE) {
        if let Some(v) = linkup.get("depth").and_then(|v| v.as_str()) {
            opts.depth = Some(v.to_string());
        }
        if let Some(v) = linkup.get("outputType").and_then(|v| v.as_str()) {
            opts.output_type = Some(v.to_string());
        }
    }
    opts
}

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "linkup";

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.linkup.so";
pub(crate) const API_KEY_ENV_VAR: &str = "LINKUP_API_KEY";
const DEFAULT_NAME: &str = "linkup";

define_vendor_provider! {
    settings: LinkupProviderSettings,
    provider: LinkupProvider,
    create: create_linkup,
    singleton: linkup,
    display: "Linkup",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: None,
}

impl LinkupProvider {
    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> LinkupSearchModel {
        LinkupSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(LinkupProvider, search_model, |p, _id| p.search_model());

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
    config: EndpointConfig,
}

impl LinkupSearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for LinkupSearchModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let linkup_options = parse_linkup_options(options.provider_options.as_ref());
        let body = build_request_body(options, &linkup_options);

        post_json_search(
            &self.config,
            "/v1/search",
            options,
            body,
            linkup_failed_response_handler(),
            |data: LinkupResponse| {
                // Prefer `results` (searchResults); fall back to `sources`
                // (sourcedAnswer), which also carries an `answer`.
                if let Some(results) = data.results {
                    (map_results(results), None)
                } else {
                    let sources = data.sources.unwrap_or_default();
                    (map_results(sources), data.answer)
                }
            },
        )
        .await
    }
}
