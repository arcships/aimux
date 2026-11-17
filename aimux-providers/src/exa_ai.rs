//! Exa AI provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Exa search API
//! (`POST https://api.exa.ai/search`). Uses `x-api-key` header auth
//! via `EXA_API_KEY`.
//!
//! [`create_exa_ai`] takes [`ExaAiProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`ExaAiProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `EXA_API_KEY`.
//! [`exa_ai()`] is the default instance; it reads nothing and cannot fail.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel, SearchResult, SearchResultItem};
use aimux_provider_utils::AuthScheme;

use crate::shared::{EndpointConfig, define_vendor_provider, post_json_search};

fn exa_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error");
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Exa request failed")
                .to_string(),
            provider_code: error
                .and_then(|value| value.get("code").or_else(|| value.get("type")))
                .and_then(Value::as_str)
                .map(str::to_string),
        }
    })
}

const MODEL_ID: &str = "exa-search";

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.exa.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "EXA_API_KEY";
const DEFAULT_NAME: &str = "exa_ai";

define_vendor_provider! {
    settings: ExaAiProviderSettings,
    provider: ExaAiProvider,
    create: create_exa_ai,
    singleton: exa_ai,
    display: "Exa AI",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Header("x-api-key"),
    extra_headers: Vec::new(),
    package: None,
}

impl ExaAiProvider {
    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> ExaAiSearchModel {
        ExaAiSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(ExaAiProvider, search_model, |p, _id| p.search_model());

fn build_request_body(options: &SearchCallOptions) -> Value {
    let mut body = json!({
        "query": options.query,
    });
    if let Some(max) = options.max_results {
        body["numResults"] = json!(max);
    }
    if let Some(include) = &options.include_domains {
        body["includeDomains"] = json!(include);
    }
    if let Some(exclude) = &options.exclude_domains {
        body["excludeDomains"] = json!(exclude);
    }
    body
}

#[derive(Debug, Deserialize)]
struct ExaResult {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExaResponse {
    #[serde(default)]
    results: Vec<ExaResult>,
}

fn map_results(entries: Vec<ExaResult>) -> Vec<SearchResultItem> {
    entries
        .into_iter()
        .map(|r| SearchResultItem {
            title: r.title,
            url: r.url,
            content: r.text,
            raw_content: None,
            score: None,
            provider_metadata: None,
        })
        .collect()
}

/// A Exa AI search model.
pub struct ExaAiSearchModel {
    config: EndpointConfig,
}

impl ExaAiSearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for ExaAiSearchModel {
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
            exa_failed_response_handler(),
            |parsed: ExaResponse| (map_results(parsed.results), None),
        )
        .await
    }
}
