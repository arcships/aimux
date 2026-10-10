//! Google Programmable Search Engine (PSE) provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Google Custom Search JSON
//! API (`GET https://www.googleapis.com/customsearch/v1`).
//!
//! Google PSE is a modality-specific provider: it exposes a web search
//! protocol and does not support language models. Authentication uses an API
//! key (`GOOGLE_API_KEY`) and a search-engine ID / `cx` (`GOOGLE_CSE_ID`),
//! both passed as query parameters. The `cx` may also be supplied at call
//! time via `provider_options["google_pse"]["cx"]`.
//!
//! [`create_google_pse`] takes [`GooglePseProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`GooglePseProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `GOOGLE_API_KEY`.
//! [`google_pse()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::shared::{AuthScheme, Credential, EndpointConfig, ProviderHeaders};

/// Fixed model ID for the Google PSE search model.
const MODEL_ID: &str = "google-pse-search";

/// Google PSE error response structure: `{ "error": { "code", "message" } }`.
fn google_pse_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error");
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Google PSE request failed")
                .to_string(),
            provider_code: error.and_then(|value| value.get("code")).and_then(
                |value| match value {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                },
            ),
        }
    })
}

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://www.googleapis.com/customsearch/v1";
const API_KEY_ENV_VAR: &str = "GOOGLE_API_KEY";
const CX_ENV_VAR: &str = "GOOGLE_CSE_ID";
const DEFAULT_NAME: &str = "google_pse";

/// Settings of [`create_google_pse`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct GooglePseProviderSettings {
    /// The endpoint URL, query parameters excluded. Default
    /// `https://www.googleapis.com/customsearch/v1`; a trailing slash is
    /// removed.
    pub base_url: Option<String>,
    /// The API key, sent as the `key` query parameter. `None` loads
    /// `GOOGLE_API_KEY` when a request is made and fails that request with
    /// `AiMuxError::LoadApiKey` if it is unset. An explicit value is used as
    /// given, `""` included: it never falls back to the environment.
    pub api_key: Option<Resolvable<String>>,
    /// The search-engine ID (`cx`). `None` uses
    /// `providerOptions.google_pse.cx` of the call, then `GOOGLE_CSE_ID`;
    /// a call with none of them fails with `AiMuxError::InvalidArgument`.
    pub cx: Option<String>,
    /// Extra headers on every request. A `None` value removes the header.
    /// Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` string
    /// (`"{name}.search"`). Default `"google_pse"`. The providerOptions key
    /// stays `google_pse`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for GooglePseProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GooglePseProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field("cx", &self.cx)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Create a Google PSE provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_google_pse(
    settings: GooglePseProviderSettings,
) -> Result<GooglePseProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(GooglePseProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        credential: Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Google PSE"),
        cx: settings.cx,
        // The key travels in the query string, so the headers carry none.
        headers: ProviderHeaders::new(
            Credential::None,
            AuthScheme::Bearer,
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_google_pse` with default settings, created
/// on first use. Creating it reads nothing from the environment and cannot
/// fail; a missing key surfaces from the first request instead.
pub fn google_pse() -> &'static GooglePseProvider {
    static DEFAULT: OnceLock<GooglePseProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_google_pse(GooglePseProviderSettings::default())
            .expect("default Google PSE settings are always valid")
    })
}

/// A Google PSE provider. Search only; it holds no HTTP client.
pub struct GooglePseProvider {
    name: String,
    base_url: String,
    credential: Credential,
    cx: Option<String>,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
}

impl GooglePseProvider {
    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> GooglePseSearchModel {
        GooglePseSearchModel::from_config(
            EndpointConfig::fixed(
                format!("{}.search", self.name),
                self.base_url.clone(),
                self.headers.clone(),
                self.fetch.clone(),
            ),
            self.credential.clone(),
            self.cx.clone(),
        )
    }
}

crate::impl_single_modality_provider!(GooglePseProvider, search_model, |p, _id| p.search_model());

/// Resolve the `cx` (search-engine ID): the `cx` setting, then
/// `provider_options["google_pse"]["cx"]`, then `GOOGLE_CSE_ID`.
fn resolve_cx(
    setting_cx: Option<&str>,
    provider_options: Option<&SharedProviderOptions>,
) -> Option<String> {
    if let Some(cx) = setting_cx {
        return Some(cx.to_string());
    }
    if let Some(cx) = options::google_pse_options(provider_options)
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
                AiMuxError::InvalidArgument(
                    "Google PSE requires a `cx` (search-engine ID). Set the `cx` setting, the `GOOGLE_CSE_ID` \
                     environment variable or pass it via `provider_options[\"google_pse\"][\"cx\"]`."
                        .to_string(),
                )
            })?;

        // Google PSE authenticates via query parameters, so the provider headers
        // carry no credential.
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let api_key = self.credential.secret().await?.unwrap_or_default();

        let mut url = url::Url::parse(exchange.base_url()).map_err(|e| {
            AiMuxError::InvalidArgument(format!("invalid google_pse endpoint: {e}"))
        })?;
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair("key", &api_key)
                .append_pair("cx", &cx)
                .append_pair("q", &options.query);
            if let Some(num) = options.max_results {
                qp.append_pair("num", &num.to_string());
            }
        }

        let resp = aimux_provider_utils::get_from_api(
            exchange.request(url.to_string(), options),
            aimux_provider_utils::create_json_response_handler::<GooglePseResponse>(),
            google_pse_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

        Ok(SearchResult {
            results: map_results(data.items),
            answer: None,
            provider_metadata: None,
            warnings: Vec::new(),
            response: Some(SearchResponse {
                headers: Some(response_headers),
                body: Some(raw_body),
            }),
        })
    }
}
