//! DataForSEO search provider — implements the `SearchModel` trait.
//!
//! Implements the DataForSEO SERP Google Organic Live Advanced API
//! (`POST https://api.dataforseo.com/v3/serp/google/organic/live/advanced`).
//!
//! Authentication uses HTTP Basic auth with `DATAFORSEO_LOGIN` and
//! `DATAFORSEO_PASSWORD` credentials. The request body is a JSON array of
//! task objects and the response is deeply nested. DataForSEO is a
//! search-only provider that does not support language models.
//!
//! [`create_dataforseo`] takes [`DataforseoProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`DataforseoProvider`]. The credential is not read
//! there: it is loaded for every request, from the settings or from `DATAFORSEO_LOGIN` and `DATAFORSEO_PASSWORD`.
//! [`dataforseo()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;

use async_trait::async_trait;
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::search_model::{
    SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
};
use aimux_provider_utils::{
    FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, combine_headers, validate_base_url,
};

use crate::shared::{Credential, EndpointConfig};

/// Fixed model id for the DataForSEO search model.
const MODEL_ID: &str = "dataforseo-search";

/// Default result depth when `max_results` is unset.
const DEFAULT_DEPTH: u32 = 10;

/// Fixed `max_credits` budget per request.
const MAX_CREDITS: u32 = 1;

/// DataForSEO error response structure: `{ status_code, status_message }`.
fn dataforseo_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("status_message")
                .and_then(Value::as_str)
                .unwrap_or("DataForSEO request failed")
                .to_string(),
            provider_code: data.get("status_code").and_then(|value| match value {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            }),
        }
    })
}

// ── Config ───────────────────────────────────────────────────────────────────

const DEFAULT_BASE_URL: &str = "https://api.dataforseo.com";
const LOGIN_ENV_VAR: &str = "DATAFORSEO_LOGIN";
const PASSWORD_ENV_VAR: &str = "DATAFORSEO_PASSWORD";
const DEFAULT_NAME: &str = "dataforseo";

/// Settings of [`create_dataforseo`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; the credentials and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct DataforseoProviderSettings {
    /// Base URL for the API calls. Default `https://api.dataforseo.com`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The HTTP Basic login. `None` loads `DATAFORSEO_LOGIN` when a request is
    /// made and fails that request with `AiMuxError::LoadApiKey` if it is
    /// unset. An explicit value is used as given, `""` included.
    pub login: Option<Resolvable<String>>,
    /// The HTTP Basic password. `None` loads `DATAFORSEO_PASSWORD` when a
    /// request is made and fails that request with `AiMuxError::LoadApiKey`
    /// if it is unset. An explicit value is used as given, `""` included.
    pub password: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including `Authorization`. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` string
    /// (`"{name}.search"`). Default `"dataforseo"`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for DataforseoProviderSettings {
    /// Never prints the credentials or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataforseoProviderSettings")
            .field("base_url", &self.base_url)
            .field("login", &self.login)
            .field("password", &self.password)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// The provider headers: `Authorization: Basic base64(login:password)` with
/// both parts resolved for every request, then the caller's headers.
fn basic_auth_headers(
    login: Credential,
    password: Credential,
    user: Option<HeaderMapOpt>,
) -> HeadersFn {
    Resolvable::from_async_fn(move || {
        let login = login.clone();
        let password = password.clone();
        let user = user.clone();
        async move {
            let login = login.secret().await?.unwrap_or_default();
            let password = password.secret().await?.unwrap_or_default();
            let encoded = base64::engine::general_purpose::STANDARD
                .encode(format!("{login}:{password}").as_bytes());
            let mut layer = HeaderMapOpt::new();
            layer.insert(
                "Authorization".to_string(),
                Some(format!("Basic {encoded}")),
            );
            Ok(match &user {
                Some(user) => combine_headers(&[&layer, user]),
                None => layer,
            })
        }
    })
}

/// Create a DataForSEO provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the credentials are
/// loaded per request, not here.
pub fn create_dataforseo(
    settings: DataforseoProviderSettings,
) -> Result<DataforseoProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(DataforseoProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: basic_auth_headers(
            Credential::explicit_or_env(settings.login, LOGIN_ENV_VAR, "DataForSEO login"),
            Credential::explicit_or_env(settings.password, PASSWORD_ENV_VAR, "DataForSEO password"),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_dataforseo` with default settings, created
/// on first use. Creating it reads nothing from the environment and cannot
/// fail; a missing login or password surfaces from the first request instead.
pub fn dataforseo() -> &'static DataforseoProvider {
    static DEFAULT: OnceLock<DataforseoProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_dataforseo(DataforseoProviderSettings::default())
            .expect("default DataForSEO settings are always valid")
    })
}

/// A DataForSEO provider. Search only; it holds no HTTP client.
pub struct DataforseoProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl DataforseoProvider {
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
    pub fn search_model(&self) -> DataforseoSearchModel {
        DataforseoSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(DataforseoProvider, search_model, |p, _id| p.search_model());

// ── Request builder ──────────────────────────────────────────────────────────

/// Resolve the `depth` request field from the call options.
///
/// Pure function: maps `max_results` to the DataForSEO `depth` field (the
/// number of organic results to return), defaulting to [`DEFAULT_DEPTH`] when
/// unset.
fn resolve_depth(max_results: Option<u32>) -> u32 {
    max_results.unwrap_or(DEFAULT_DEPTH)
}

/// Build the DataForSEO request body (a JSON array of one task object).
///
/// Pure function. The body shape is:
/// `[{"keyword": <query>, "max_credits": 1, "depth": <depth>}]`.
fn build_request_body(query: &str, depth: u32) -> Value {
    json!([{ "keyword": query, "max_credits": MAX_CREDITS, "depth": depth }])
}

// ── Response types ───────────────────────────────────────────────────────────

/// The response from the DataForSEO live/advanced endpoint.
///
/// The result list is nested as `tasks[].result[].organic[]`; only the fields
/// used by the trait are deserialized, and extra fields are ignored so
/// unknown-but-legal values degrade safely.
#[derive(Debug, Deserialize)]
struct DataforseoResponse {
    #[serde(default)]
    tasks: Vec<DataforseoTask>,
}

#[derive(Debug, Deserialize)]
struct DataforseoTask {
    #[serde(default)]
    result: Vec<DataforseoTaskResult>,
}

#[derive(Debug, Deserialize)]
struct DataforseoTaskResult {
    #[serde(default)]
    organic: Vec<DataforseoOrganic>,
}

#[derive(Debug, Deserialize)]
struct DataforseoOrganic {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// Map a DataForSEO organic result into a [`SearchResultItem`].
///
/// Field mapping: `title` → `title`, `url` → `url`, `description` → `content`.
fn map_result(r: DataforseoOrganic) -> SearchResultItem {
    SearchResultItem {
        title: r.title,
        url: r.url,
        content: r.description,
        raw_content: None,
        score: None,
        provider_metadata: None,
    }
}

// ── Search model ─────────────────────────────────────────────────────────────

/// A DataForSEO search model.
pub struct DataforseoSearchModel {
    config: EndpointConfig,
}

impl DataforseoSearchModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SearchModel for DataforseoSearchModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        MODEL_ID
    }

    async fn do_search(&self, options: &SearchCallOptions) -> Result<SearchResult, AiMuxError> {
        let depth = resolve_depth(options.max_results);
        let body = build_request_body(&options.query, depth);
        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(
                exchange.url("/v3/serp/google/organic/live/advanced"),
                options,
            ),
            body,
            aimux_provider_utils::create_json_response_handler::<DataforseoResponse>(),
            dataforseo_failed_response_handler(),
        )
        .await?;

        // Capture response headers.
        let response_headers = resp.response_headers;

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

        // Flatten tasks[].result[].organic[] preserving provider order.
        let results: Vec<SearchResultItem> = data
            .tasks
            .into_iter()
            .flat_map(|t| t.result)
            .flat_map(|r| r.organic)
            .map(map_result)
            .collect();

        Ok(SearchResult {
            results,
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
