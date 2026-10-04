//! Cohere provider.
//!
//! [`create_cohere`] is the Rust form of the AI SDK's `createCohere`: it takes
//! [`CohereProviderSettings`], validates the base URL, fixes the provider name
//! and returns a [`CohereProvider`]. The API key is not read there; it is
//! loaded in the request headers of every call, from the setting or from
//! `COHERE_API_KEY`. [`cohere()`] is the default instance.
//!
//! Implements Cohere's v2 chat API with its own message format (not
//! OpenAI-compatible). Supports text generation, streaming, tool calls,
//! and reasoning (thinking).

pub mod convert;
pub mod embedding;
mod model;
pub(crate) mod options;
pub mod reranking;
mod types;

pub use crate::shared::TransformRequestBody;
pub use embedding::CohereEmbeddingModel;
pub use model::CohereModel;
pub use reranking::CohereRerankingModel;

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;
use serde_json::Value;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::reranking_model::RerankingModel;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{Credential, EndpointConfig, provider_headers};

pub(crate) fn cohere_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: None,
        }
    })
}

const DEFAULT_BASE_URL: &str = "https://api.cohere.com/v2";
const API_KEY_ENV_VAR: &str = "COHERE_API_KEY";
const DEFAULT_NAME: &str = "cohere";

/// Settings of [`create_cohere`] (the AI SDK's `CohereProviderSettings`).
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct CohereProviderSettings {
    /// Base URL for the API calls. Default `https://api.cohere.com/v2`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `COHERE_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including `Authorization`. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings
    /// (`"{name}.chat"`, `"{name}.textEmbedding"`, `"{name}.reranking"`).
    /// Default `"cohere"`. The providerOptions key stays `cohere`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Rewrites every JSON request body once, after it is serialized and
    /// before it is sent.
    pub transform_request_body: Option<TransformRequestBody>,
}

impl std::fmt::Debug for CohereProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CohereProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .field(
                "transform_request_body",
                &self.transform_request_body.is_some(),
            )
            .finish()
    }
}

/// Create a Cohere provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_cohere(settings: CohereProviderSettings) -> Result<CohereProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(CohereProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Cohere"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
        transform_request_body: settings.transform_request_body,
    })
}

/// The default provider: `create_cohere` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn cohere() -> &'static CohereProvider {
    static DEFAULT: OnceLock<CohereProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_cohere(CohereProviderSettings::default())
            .expect("default Cohere settings are always valid")
    })
}

/// A Cohere provider (the AI SDK's `CohereProvider`). Cheap to clone the
/// models out of; it holds no HTTP client.
pub struct CohereProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
}

impl CohereProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            self.transform_request_body.clone(),
        )
    }

    /// A chat model; `provider()` is `"{name}.chat"`.
    #[must_use]
    pub fn chat(&self, model_id: &str) -> CohereModel {
        CohereModel::from_config(model_id.to_string(), self.model_config("chat"))
    }

    /// An embedding model (e.g. `"embed-english-v3.0"`); `provider()` is
    /// `"{name}.textEmbedding"`.
    #[must_use]
    pub fn embedding(&self, model_id: &str) -> CohereEmbeddingModel {
        CohereEmbeddingModel::from_config(model_id.to_string(), self.model_config("textEmbedding"))
    }

    /// A reranking model (e.g. `"rerank-english-v3.0"`); `provider()` is
    /// `"{name}.reranking"`.
    #[must_use]
    pub fn reranking(&self, model_id: &str) -> CohereRerankingModel {
        CohereRerankingModel::from_config(model_id.to_string(), self.model_config("reranking"))
    }

    /// The provider as a function: the default language model for an id. The
    /// AI SDK's callable provider; the same model as [`chat`](Self::chat) and
    /// [`language_model`](Provider::language_model).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.chat(model_id))
    }
}

impl Provider for CohereProvider {
    fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
        Some(self)
    }

    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Ok(Arc::new(self.embedding(model_id)))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }

    fn reranking_model(
        &self,
        model_id: &str,
    ) -> Option<Result<Arc<dyn RerankingModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.reranking(model_id))))
    }
}

impl ProviderDiscovery for CohereProvider {
    /// `GET {base_url}/models` (Cohere v2): one exchange, no retry. Only the
    /// chat-capable models are listed.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.model_config("models");
        Box::pin(async move {
            // Cohere v2: { models: [{ name, endpoints, ... }] }
            #[derive(serde::Deserialize)]
            struct Resp {
                #[serde(default)]
                models: Vec<Entry>,
            }
            #[derive(serde::Deserialize)]
            struct Entry {
                name: String,
                #[serde(default)]
                endpoints: Option<Vec<String>>,
            }

            let exchange = config.exchange(None).await?;
            let resp = aimux_provider_utils::get_from_api(
                exchange.with_transport(aimux_provider_utils::HttpRequest {
                    url: exchange.url("/models"),
                    headers: exchange.headers(),
                    ..Default::default()
                }),
                aimux_provider_utils::create_json_response_handler(),
                cohere_failed_response_handler(),
            )
            .await?;
            let parsed: Resp = resp.value;
            Ok(parsed
                .models
                .into_iter()
                .filter(|entry| {
                    entry
                        .endpoints
                        .as_ref()
                        .is_none_or(|endpoints| endpoints.iter().any(|e| e == "chat"))
                })
                .map(|entry| RuntimeModel {
                    id: entry.name,
                    owned_by: Some(options::NAMESPACE.to_string()),
                    created: None,
                })
                .collect())
        })
    }
}
