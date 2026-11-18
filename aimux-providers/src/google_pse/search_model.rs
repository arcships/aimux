use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel, SearchResult, SearchResultItem};
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::Credential;

use crate::shared::{EndpointConfig, get_query_search_with_exchange, namespaced_options};

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "google_pse";

/// Fixed model ID for the Google PSE search model.
const MODEL_ID: &str = "google-pse-search";

const CX_ENV_VAR: &str = "GOOGLE_CSE_ID";

/// Google PSE error response structure: `{ "error": { "code", "message" } }`.
fn google_pse_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    crate::shared::nested_error_response_handler("Google PSE request failed")
}

/// Resolve the `cx` (search-engine ID): the `cx` setting, then
/// `provider_options["google_pse"]["cx"]`, then `GOOGLE_CSE_ID`.
fn resolve_cx(
    setting_cx: Option<&str>,
    provider_options: Option<&SharedProviderOptions>,
) -> Option<String> {
    if let Some(cx) = setting_cx {
        return Some(cx.to_string());
    }
    if let Some(cx) = namespaced_options(provider_options, NAMESPACE)
        .and_then(|options| options.get("cx"))
        .and_then(Value::as_str)
    {
        return Some(cx.to_string());
    }
    std::env::var(CX_ENV_VAR).ok()
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
    config: EndpointConfig,
    credential: Credential,
    cx: Option<String>,
}

impl GooglePseSearchModel {
    pub(crate) fn from_config(
        config: EndpointConfig,
        credential: Credential,
        cx: Option<String>,
    ) -> Self {
        Self {
            config,
            credential,
            cx,
        }
    }
}

#[async_trait]
impl SearchModel for GooglePseSearchModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let cx = resolve_cx(self.cx.as_deref(), options.provider_options.as_ref())
            .ok_or_else(|| {
                AiMuxError::invalid_argument(
                    "Google PSE requires a `cx` (search-engine ID). Set the `cx` setting, the `GOOGLE_CSE_ID` \
                     environment variable or pass it via `provider_options[\"google_pse\"][\"cx\"]`."
                        .to_string(),
                )
            })?;

        // Google PSE authenticates via query parameters, so the provider headers
        // carry no credential.
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let api_key = self.credential.secret().await?.unwrap_or_default();

        get_query_search_with_exchange(
            &exchange,
            None,
            &self.config.provider,
            options,
            |url| {
                let mut qp = url.query_pairs_mut();
                qp.append_pair("key", &api_key)
                    .append_pair("cx", &cx)
                    .append_pair("q", &options.query);
                if let Some(num) = options.max_results {
                    qp.append_pair("num", &num.to_string());
                }
            },
            google_pse_failed_response_handler(),
            |data: GooglePseResponse| (map_results(data.items), None),
        )
        .await
    }
}
