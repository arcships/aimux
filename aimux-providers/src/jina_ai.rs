//! Jina AI provider — rerank modality only.
//!
//! Implements the `RerankingModel` trait against the Jina AI rerank API
//! (`POST https://api.jina.ai/v1/rerank`).
//!
//! Jina AI is a modality-specific provider: it exposes a native rerank
//! protocol (query + documents → relevance scores) and does not support
//! language models.
//!
//! [`create_jina_ai`] takes [`JinaAiProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`JinaAiProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `JINA_AI_API_KEY`.
//! [`jina_ai()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::reranking_model::{
    RerankingCallOptions, RerankingDocuments, RerankingModel, RerankingRank, RerankingResponse,
    RerankingResult,
};
use aimux_core::shared::SharedProviderOptions;
use aimux_core::types::Warning;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{Credential, EndpointConfig, provider_headers};

/// Jina AI error response structure: `{ "detail": "...", "code": "..." }`.
///
/// `detail` carries the human-readable message; `code` carries the
/// machine-readable error code (e.g. `AUTH_INVALID_API_KEY`). Both the
/// `ErrorResponse` (`detail` + optional `code`) and the FastAPI
/// `HTTPValidationError` (`detail`) shapes are covered, since both surface
/// the message under `detail`.
fn jina_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or("Jina AI request failed")
                .to_string(),
            provider_code: data.get("code").and_then(Value::as_str).map(str::to_string),
        }
    })
}

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://api.jina.ai";
const API_KEY_ENV_VAR: &str = "JINA_AI_API_KEY";
const DEFAULT_NAME: &str = "jina_ai";

/// Settings of [`create_jina_ai`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct JinaAiProviderSettings {
    /// Base URL for the API calls. Default `https://api.jina.ai`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `JINA_AI_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.reranking"`).
    /// Default `"jina_ai"`. The providerOptions key stays `jina`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for JinaAiProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JinaAiProviderSettings")
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

/// Create a Jina AI provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_jina_ai(settings: JinaAiProviderSettings) -> Result<JinaAiProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(JinaAiProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Jina AI"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_jina_ai` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn jina_ai() -> &'static JinaAiProvider {
    static DEFAULT: OnceLock<JinaAiProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_jina_ai(JinaAiProviderSettings::default())
            .expect("default Jina AI settings are always valid")
    })
}

/// A Jina AI provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct JinaAiProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl JinaAiProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
        )
    }

    /// A reranking model (e.g. `"jina-reranker-v2-base-multilingual"`); `provider()` is `"{name}.reranking"`.
    #[must_use]
    pub fn reranking_model(&self, model_id: &str) -> JinaAiRerankingModel {
        JinaAiRerankingModel::from_config(model_id.to_string(), self.model_config("reranking"))
    }
}

crate::impl_single_modality_provider!(JinaAiProvider, reranking_model, |p, id| p
    .reranking_model(id));

/// Jina AI provider-specific reranking options.
#[derive(Debug, Clone, Default)]
struct JinaRerankingOptions {
    return_documents: Option<bool>,
}

fn parse_jina_reranking_options(
    provider_options: Option<&SharedProviderOptions>,
) -> JinaRerankingOptions {
    let mut opts = JinaRerankingOptions::default();
    if let Some(jina) = options::jina_options(provider_options)
        && let Some(v) = jina
            .get("returnDocuments")
            .and_then(serde_json::Value::as_bool)
    {
        opts.return_documents = Some(v);
    }
    opts
}

/// Convert call options into the Jina AI rerank request body (pure function).
///
/// Object documents are stringified (with a compatibility warning) since the
/// Jina rerank endpoint accepts a list of strings.
fn build_request_body(
    model_id: &str,
    options: &RerankingCallOptions,
    jina_options: &JinaRerankingOptions,
    warnings: &mut Vec<Warning>,
) -> Value {
    let documents: Vec<String> = match &options.documents {
        RerankingDocuments::Text { values } => values.clone(),
        RerankingDocuments::Object { values } => {
            warnings.push(Warning::Compatibility {
                feature: "object documents".to_string(),
                details: Some("Object documents are converted to strings.".to_string()),
            });
            values
                .iter()
                .map(std::string::ToString::to_string)
                .collect()
        }
    };

    let mut body = json!({
        "model": model_id,
        "query": options.query,
        "documents": documents,
        "top_n": options.top_n,
    });

    if let Some(return_documents) = jina_options.return_documents {
        body["return_documents"] = json!(return_documents);
    }

    body
}

/// The response from the Jina AI `/v1/rerank` endpoint.
///
/// Only the fields used by the trait are deserialized; extra fields returned
/// by the API (e.g. `object`, per-result `document`/`embedding`) are ignored
/// so unknown-but-legal values degrade safely.
#[derive(Debug, Deserialize)]
struct JinaRerankingResponse {
    #[serde(default)]
    model: Option<String>,
    results: Vec<JinaRerankingResult>,
}

#[derive(Debug, Deserialize)]
struct JinaRerankingResult {
    index: u32,
    relevance_score: f64,
}

/// A Jina AI reranking model.
pub struct JinaAiRerankingModel {
    model_id: String,
    config: EndpointConfig,
}

impl JinaAiRerankingModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl RerankingModel for JinaAiRerankingModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_rerank(
        &self,
        options: &RerankingCallOptions,
    ) -> Result<RerankingResult, AiMuxError> {
        let jina_options = parse_jina_reranking_options(options.provider_options.as_ref());

        let mut warnings = Vec::new();
        let body = build_request_body(&self.model_id, options, &jina_options, &mut warnings);

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/v1/rerank"), options),
            body,
            aimux_provider_utils::create_json_response_handler::<JinaRerankingResponse>(),
            jina_failed_response_handler(),
        )
        .await?;

        // Capture response headers.
        let response_headers = resp.response_headers;

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

        let model = data.model;
        let ranking: Vec<RerankingRank> = data
            .results
            .into_iter()
            .map(|r| RerankingRank {
                index: r.index,
                relevance_score: r.relevance_score,
            })
            .collect();

        Ok(RerankingResult {
            ranking,
            provider_metadata: None,
            warnings: Some(warnings),
            response: Some(RerankingResponse {
                id: None,
                timestamp: None,
                model_id: model,
                headers: Some(response_headers),
                body: Some(raw_body),
            }),
        })
    }
}
