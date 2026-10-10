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

use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{AuthScheme, Credential, EndpointConfig, credential_headers};

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

const DEFAULT_BASE_URL: &str = "https://api.exa.ai";
const API_KEY_ENV_VAR: &str = "EXA_API_KEY";
const DEFAULT_NAME: &str = "exa_ai";

/// Settings of [`create_exa_ai`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct ExaAiProviderSettings {
    /// Base URL for the API calls. Default `https://api.exa.ai`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `EXA_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.search"`).
    /// Default `"exa_ai"`. The providerOptions key stays `exa_ai`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for ExaAiProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExaAiProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Create a Exa AI provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_exa_ai(settings: ExaAiProviderSettings) -> Result<ExaAiProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(ExaAiProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: credential_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Exa AI"),
            AuthScheme::Header("x-api-key"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_exa_ai` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn exa_ai() -> &'static ExaAiProvider {
    static DEFAULT: OnceLock<ExaAiProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_exa_ai(ExaAiProviderSettings::default())
            .expect("default Exa AI settings are always valid")
    })
}

/// A Exa AI provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct ExaAiProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl ExaAiProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
        )
    }

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
        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/search"), options),
            body,
            aimux_provider_utils::create_json_response_handler(),
            exa_failed_response_handler(),
        )
        .await?;
        let response_headers = resp.response_headers;
        let response_body = resp.raw_value;
        let parsed: ExaResponse = resp.value;

        Ok(SearchResult {
            results: map_results(parsed.results),
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
