//! Voyage AI provider.
//!
//! [`create_voyage`] takes [`VoyageProviderSettings`], validates the base URL,
//! fixes the provider name and returns a [`VoyageProvider`]. The API key is not
//! read there; it is loaded in the request headers of every call, from the
//! setting or from `VOYAGE_API_KEY`. [`voyage()`] is the default instance. The
//! AI SDK has no Voyage package; the factory follows the shape of its
//! embedding and reranking vendors.
//!
//! Implements the `EmbeddingModel` trait against the Voyage AI API
//! (`api.voyageai.com/v1/embeddings`).

pub mod embedding;
pub(crate) mod options;
pub mod reranking;

pub use embedding::VoyageEmbeddingModel;
pub use reranking::VoyageRerankingModel;

use std::sync::{Arc, OnceLock};

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_core::reranking_model::RerankingModel;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{Credential, EndpointConfig, provider_headers};

pub(crate) fn voyage_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("detail")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Voyage request failed")
                .to_owned(),
            provider_code: None,
        }
    })
}

const DEFAULT_BASE_URL: &str = "https://api.voyageai.com/v1";
const API_KEY_ENV_VAR: &str = "VOYAGE_API_KEY";
const DEFAULT_NAME: &str = "voyage";

/// Settings of [`create_voyage`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct VoyageProviderSettings {
    /// Base URL for the API calls. Default `https://api.voyageai.com/v1`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `VOYAGE_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including `Authorization`. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings
    /// (`"{name}.embedding"`, `"{name}.reranking"`). Default `"voyage"`. The
    /// providerOptions key stays `voyage`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for VoyageProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoyageProviderSettings")
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

/// Create a Voyage AI provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_voyage(settings: VoyageProviderSettings) -> Result<VoyageProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(VoyageProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Voyage"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_voyage` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn voyage() -> &'static VoyageProvider {
    static DEFAULT: OnceLock<VoyageProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_voyage(VoyageProviderSettings::default())
            .expect("default Voyage settings are always valid")
    })
}

/// A Voyage AI provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct VoyageProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl VoyageProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
        )
    }

    /// An embedding model (e.g. `"voyage-3.5"`); `provider()` is
    /// `"{name}.embedding"`.
    #[must_use]
    pub fn embedding(&self, model_id: &str) -> VoyageEmbeddingModel {
        VoyageEmbeddingModel::from_config(model_id.to_string(), self.model_config("embedding"))
    }

    /// A reranking model (e.g. `"rerank-2.5"`); `provider()` is
    /// `"{name}.reranking"`.
    #[must_use]
    pub fn reranking(&self, model_id: &str) -> VoyageRerankingModel {
        VoyageRerankingModel::from_config(model_id.to_string(), self.model_config("reranking"))
    }
}

impl Provider for VoyageProvider {
    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "languageModel"))
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
