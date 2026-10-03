//! Amazon Bedrock provider.
//!
//! [`create_amazon_bedrock`] is the Rust form of the AI SDK's
//! `createAmazonBedrock`: it takes [`AmazonBedrockProviderSettings`], validates
//! an explicit base URL and returns an [`AmazonBedrockProvider`]. Nothing is
//! read from the environment and no region, key or credential is needed until
//! a request is made. [`amazon_bedrock()`] is the default instance.
//!
//! The models speak the Bedrock Converse API
//! (`bedrock-runtime.{region}.amazonaws.com/model/{model-id}/converse`), which
//! gives one interface to every Bedrock-hosted model family (Anthropic Claude,
//! Meta Llama, Mistral, Amazon Nova, ...), plus `invoke` for embeddings and
//! images and the Agent Runtime `rerank` endpoint. Claude's native Messages
//! protocol on AWS is a different package ([`crate::anthropic_aws`]) and the
//! OpenAI-compatible Mantle endpoint is a preset; neither is served from here.
//!
//! Authentication is decided for each request, the way `createAmazonBedrock`
//! decides it once at creation:
//!
//! - **API key** when one is available (the `api_key` setting, else
//!   `AWS_BEARER_TOKEN_BEDROCK`): `Authorization: Bearer <key>`, nothing is
//!   signed.
//! - **AWS SigV4** otherwise: a [`SigV4Fetch`] transport decorator signs the
//!   final method, URL, headers and body bytes. The credentials come from
//!   `credential_provider`, else `access_key_id` + `secret_access_key` (+
//!   `session_token`), else `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and
//!   `AWS_SESSION_TOKEN`. A missing region or credential fails the request
//!   with `AiMuxError::LoadSetting`.

pub mod convert;
pub mod embedding;
pub mod event_stream;
pub mod image;
mod model;
pub(crate) mod options;
pub mod reranking;
mod types;

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use futures::future::BoxFuture;
use serde_json::Value;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::reranking_model::RerankingModel;
use aimux_provider_utils::{
    AwsCredentials, Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HeaderMapOpt,
    HeadersFn, Resolvable, SigV4Fetch, combine_headers, default_fetch, load_optional_setting,
    load_setting, validate_base_url, without_trailing_slash,
};

use crate::shared::{Endpoint, EndpointConfig, TransformRequestBody};

pub use embedding::BedrockEmbeddingModel;
pub use image::BedrockImageModel;
pub use model::BedrockModel;
pub use reranking::BedrockRerankingModel;

/// The region of a request, resolved when the request is made.
pub(crate) type RegionFn = Arc<dyn Fn() -> Result<String, AiMuxError> + Send + Sync>;

const PROVIDER: &str = "amazon-bedrock";
const SIGV4_SERVICE: &str = "bedrock";

const REGION_ENV_VAR: &str = "AWS_REGION";
const BEARER_TOKEN_ENV_VAR: &str = "AWS_BEARER_TOKEN_BEDROCK";
const ACCESS_KEY_ID_ENV_VAR: &str = "AWS_ACCESS_KEY_ID";
const SECRET_ACCESS_KEY_ENV_VAR: &str = "AWS_SECRET_ACCESS_KEY";
const SESSION_TOKEN_ENV_VAR: &str = "AWS_SESSION_TOKEN";
const RUNTIME_ENDPOINT_ENV_VAR: &str = "AWS_ENDPOINT_URL_BEDROCK_RUNTIME";
const AGENT_RUNTIME_ENDPOINT_ENV_VAR: &str = "AWS_ENDPOINT_URL_BEDROCK_AGENT_RUNTIME";

pub(crate) fn bedrock_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
        // Converse errors may be a top-level AWS error object or be wrapped
        // as `{ "error": { ... } }` by compatible gateways.
        let error = data.get("error").unwrap_or(data);
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: error
                .get("type")
                .or_else(|| error.get("__type"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    })
}

/// Settings of [`create_amazon_bedrock`] (the AI SDK's
/// `AmazonBedrockProviderSettings`).
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except an explicit `base_url`; everything else is evaluated on every
/// request. An explicit value is used as given and never falls back to the
/// environment.
#[derive(Clone, Default)]
pub struct AmazonBedrockProviderSettings {
    /// The AWS region. `None` loads `AWS_REGION` when a request is made (else
    /// the region of a `credential_provider` that is a plain value) and fails
    /// that request with `AiMuxError::LoadSetting` if none is found.
    pub region: Option<String>,
    /// API key for Bearer authentication; when one is available it is used
    /// instead of SigV4. `None` loads `AWS_BEARER_TOKEN_BEDROCK` when a
    /// request is made. An empty or whitespace-only key counts as no key. A
    /// [`Resolvable::Future`] is awaited once, an [`Resolvable::AsyncFn`] on
    /// every request.
    pub api_key: Option<Resolvable<String>>,
    /// The AWS access key id of SigV4 signing. `None` loads `AWS_ACCESS_KEY_ID`.
    pub access_key_id: Option<String>,
    /// The AWS secret access key of SigV4 signing. `None` loads
    /// `AWS_SECRET_ACCESS_KEY`.
    pub secret_access_key: Option<String>,
    /// The AWS session token of temporary credentials. When `access_key_id`
    /// and `secret_access_key` are both given only this field is used; when
    /// either comes from the environment the token also falls back to
    /// `AWS_SESSION_TOKEN`.
    pub session_token: Option<String>,
    /// Base URL for the Bedrock Runtime calls (and the Agent Runtime
    /// `rerank` calls). Default `https://bedrock-runtime.{region}.amazonaws.com`,
    /// or `AWS_ENDPOINT_URL_BEDROCK_RUNTIME` when that is set; a trailing slash
    /// is removed.
    pub base_url: Option<String>,
    /// Extra headers on every request. A `None` value removes the header.
    /// Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The transport SigV4 signing (when used) wraps. `None` uses the process
    /// default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Dynamic AWS credentials, like the AWS SDK's credential providers. When
    /// set, its credentials are used instead of `access_key_id`,
    /// `secret_access_key` and `session_token`; it is resolved on every
    /// request, so an [`Resolvable::AsyncFn`] can hand out rotating STS
    /// credentials. Its `region` is used only when neither `region` nor
    /// `AWS_REGION` names one.
    pub credential_provider: Option<Resolvable<AwsCredentials>>,
    /// Rewrites every JSON request body once, after it is serialized and
    /// before it is sent (and before it is signed).
    pub transform_request_body: Option<TransformRequestBody>,
}

impl std::fmt::Debug for AmazonBedrockProviderSettings {
    /// Never prints keys, credentials or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AmazonBedrockProviderSettings")
            .field("region", &self.region)
            .field("api_key", &self.api_key)
            .field("access_key_id", &self.access_key_id.is_some())
            .field("secret_access_key", &self.secret_access_key.is_some())
            .field("session_token", &self.session_token.is_some())
            .field("base_url", &self.base_url)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("fetch", &self.fetch.is_some())
            .field("credential_provider", &self.credential_provider.is_some())
            .field(
                "transform_request_body",
                &self.transform_request_body.is_some(),
            )
            .finish()
    }
}

/// Where the region, the API key and the AWS credentials of a request come
/// from.
struct Auth {
    region: Option<String>,
    api_key: Option<Resolvable<String>>,
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    session_token: Option<String>,
    credential_provider: Option<Resolvable<AwsCredentials>>,
}

impl Auth {
    /// The Bearer API key, when there is a non-blank one (trimmed).
    async fn bearer(&self) -> Result<Option<String>, AiMuxError> {
        let raw = match &self.api_key {
            Some(key) => Some(key.resolve().await?),
            None => load_optional_setting(None, BEARER_TOKEN_ENV_VAR),
        };
        Ok(raw
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty()))
    }

    /// The region: the setting, `AWS_REGION`, or the region of a plain-value
    /// credential provider.
    fn region(&self) -> Result<String, AiMuxError> {
        if let Some(region) = load_optional_setting(self.region.as_deref(), REGION_ENV_VAR) {
            return Ok(region);
        }
        if let Some(Resolvable::Value(credentials)) = &self.credential_provider
            && !credentials.region.is_empty()
        {
            return Ok(credentials.region.clone());
        }
        load_setting(None, REGION_ENV_VAR, "region")
    }

    /// The credentials of the settings and the environment (no provider).
    fn static_credentials(&self) -> Result<AwsCredentials, AiMuxError> {
        let region = self.region()?;
        let access_key_id = load_setting(
            self.access_key_id.as_deref(),
            ACCESS_KEY_ID_ENV_VAR,
            "access_key_id",
        )?;
        let secret_access_key = load_setting(
            self.secret_access_key.as_deref(),
            SECRET_ACCESS_KEY_ENV_VAR,
            "secret_access_key",
        )?;
        let session_token = if self.access_key_id.is_some() && self.secret_access_key.is_some() {
            self.session_token.clone()
        } else {
            load_optional_setting(self.session_token.as_deref(), SESSION_TOKEN_ENV_VAR)
        };
        Ok(AwsCredentials {
            access_key_id,
            secret_access_key,
            session_token,
            region,
        })
    }

    /// The credentials to sign one request with.
    async fn credentials(&self) -> Result<AwsCredentials, AiMuxError> {
        match &self.credential_provider {
            Some(provider) => {
                let mut credentials = provider.resolve().await?;
                if let Ok(region) = self.region() {
                    credentials.region = region;
                } else if credentials.region.is_empty() {
                    return Err(AiMuxError::LoadSetting {
                        env_var: REGION_ENV_VAR.to_string(),
                        name: "region".to_string(),
                    });
                }
                Ok(credentials)
            }
            None => self.static_credentials(),
        }
    }

    /// Fail now, with the typed setting error, when signing could not find its
    /// inputs: the transport reports a credential failure as a transport
    /// error, which would hide which setting is missing. A credential provider
    /// is the caller's own and is only called when the request is signed.
    fn preflight(&self) -> Result<(), AiMuxError> {
        if self.credential_provider.is_some() {
            return Ok(());
        }
        self.static_credentials().map(|_| ())
    }

    /// The base URL of `service`: the explicit one, the service's endpoint
    /// environment variable, or the regional host.
    fn base_url(
        explicit: Option<&str>,
        service: &str,
        endpoint_env_var: &str,
        region: &dyn Fn() -> Result<String, AiMuxError>,
    ) -> Result<String, AiMuxError> {
        if let Some(url) = explicit {
            return Ok(url.to_string());
        }
        if let Some(url) = load_optional_setting(None, endpoint_env_var) {
            return validate_base_url(&url);
        }
        Ok(format!("https://{service}.{}.amazonaws.com", region()?))
    }
}

/// Signs a request with SigV4 unless it already carries an `Authorization`
/// header (the API-key path): one decision, made by the provider headers.
struct BedrockAuthFetch {
    signing: SigV4Fetch,
    inner: FetchFunction,
}

#[async_trait]
impl Fetch for BedrockAuthFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        if request.headers.contains_key(reqwest::header::AUTHORIZATION) {
            self.inner.fetch(request).await
        } else {
            self.signing.fetch(request).await
        }
    }
}

/// Create an Amazon Bedrock provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when an explicit `base_url` is not an
/// `http(s)` URL with a host. That is the only way this fails: the region, the
/// key and the credentials are loaded per request, not here.
pub fn create_amazon_bedrock(
    settings: AmazonBedrockProviderSettings,
) -> Result<AmazonBedrockProvider, AiMuxError> {
    let base_url = settings
        .base_url
        .as_deref()
        .map(validate_base_url)
        .transpose()?;
    let auth = Arc::new(Auth {
        region: settings.region,
        api_key: settings.api_key,
        access_key_id: settings.access_key_id,
        secret_access_key: settings.secret_access_key,
        session_token: settings.session_token,
        credential_provider: settings.credential_provider,
    });

    let user_headers = settings.headers;
    let header_auth = auth.clone();
    let headers: HeadersFn = Resolvable::from_async_fn(move || {
        let auth = header_auth.clone();
        let user = user_headers.clone().unwrap_or_default();
        async move {
            match auth.bearer().await? {
                Some(key) => {
                    let mut bearer = HeaderMapOpt::new();
                    bearer.insert("Authorization".to_string(), Some(format!("Bearer {key}")));
                    Ok(combine_headers(&[&user, &bearer]))
                }
                None => {
                    auth.preflight()?;
                    Ok(user)
                }
            }
        }
    });

    let credentials_auth = auth.clone();
    let inner = settings.fetch.unwrap_or_else(default_fetch);
    let fetch: FetchFunction = Arc::new(BedrockAuthFetch {
        signing: SigV4Fetch::new(
            inner.clone(),
            Resolvable::from_async_fn(move || {
                let auth = credentials_auth.clone();
                async move { auth.credentials().await }
            }),
            SIGV4_SERVICE,
        ),
        inner,
    });

    Ok(AmazonBedrockProvider {
        auth,
        base_url: base_url.map(|url| without_trailing_slash(&url)),
        headers,
        fetch,
        transform_request_body: settings.transform_request_body,
    })
}

/// The default provider: `create_amazon_bedrock` with default settings,
/// created on first use. Creating it reads nothing from the environment and
/// cannot fail; a missing region, key or credential surfaces from the first
/// request instead.
pub fn amazon_bedrock() -> &'static AmazonBedrockProvider {
    static DEFAULT: OnceLock<AmazonBedrockProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_amazon_bedrock(AmazonBedrockProviderSettings::default())
            .expect("default Bedrock settings are always valid")
    })
}

/// An Amazon Bedrock provider (the AI SDK's `AmazonBedrockProvider`). Cheap to
/// clone the models out of; it holds no HTTP client.
pub struct AmazonBedrockProvider {
    auth: Arc<Auth>,
    base_url: Option<String>,
    headers: HeadersFn,
    fetch: FetchFunction,
    transform_request_body: Option<TransformRequestBody>,
}

impl AmazonBedrockProvider {
    /// The configuration of a model that calls `service` (`bedrock-runtime`
    /// or `bedrock-agent-runtime`).
    fn model_config(
        &self,
        service: &'static str,
        endpoint_env_var: &'static str,
    ) -> EndpointConfig {
        let auth = self.auth.clone();
        let base_url = self.base_url.clone();
        let headers = self.headers.clone();
        EndpointConfig {
            provider: PROVIDER.to_string(),
            endpoint: Arc::new(move || {
                let auth = auth.clone();
                let base_url = base_url.clone();
                let headers = headers.clone();
                Box::pin(async move {
                    let region_auth = auth.clone();
                    let base_url = Auth::base_url(
                        base_url.as_deref(),
                        service,
                        endpoint_env_var,
                        &move || region_auth.region(),
                    )?;
                    Ok(Endpoint {
                        base_url,
                        headers: headers.resolve().await?,
                    })
                })
            }),
            fetch: Some(self.fetch.clone()),
            supported_urls: Arc::new(|_| SupportedUrls::default()),
            transform_request_body: self.transform_request_body.clone(),
        }
    }

    fn runtime_config(&self) -> EndpointConfig {
        self.model_config("bedrock-runtime", RUNTIME_ENDPOINT_ENV_VAR)
    }

    /// A Converse language model for a Bedrock model id (e.g.
    /// `"anthropic.claude-3-5-sonnet-20240620-v1:0"`); `provider()` is
    /// `"amazon-bedrock"`.
    #[must_use]
    pub fn chat(&self, model_id: &str) -> BedrockModel {
        BedrockModel::from_config(model_id.to_string(), self.runtime_config())
    }

    /// An embedding model (e.g. `"amazon.titan-embed-text-v2:0"`);
    /// `provider()` is `"amazon-bedrock"`.
    #[must_use]
    pub fn embedding(&self, model_id: &str) -> BedrockEmbeddingModel {
        BedrockEmbeddingModel::from_config(model_id.to_string(), self.runtime_config())
    }

    /// An image generation model (e.g. `"amazon.titan-image-generator-v1"` or
    /// `"amazon.nova-canvas-v1:0"`); `provider()` is `"amazon-bedrock"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> BedrockImageModel {
        BedrockImageModel::from_config(model_id.to_string(), self.runtime_config())
    }

    /// A reranking model (e.g. `"cohere.rerank-v3-5:0"`) on the Agent Runtime
    /// `rerank` endpoint; `provider()` is `"amazon-bedrock"`.
    #[must_use]
    pub fn reranking(&self, model_id: &str) -> BedrockRerankingModel {
        let auth = self.auth.clone();
        BedrockRerankingModel::from_config(
            model_id.to_string(),
            self.model_config("bedrock-agent-runtime", AGENT_RUNTIME_ENDPOINT_ENV_VAR),
            Arc::new(move || auth.region()),
        )
    }

    /// The provider as a function: the default language model for an id. The
    /// AI SDK's callable provider; the same model as [`chat`](Self::chat).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.chat(model_id))
    }
}

impl Provider for AmazonBedrockProvider {
    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Ok(Arc::new(self.embedding(model_id)))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Ok(Arc::new(self.image(model_id)))
    }

    fn reranking_model(
        &self,
        model_id: &str,
    ) -> Option<Result<Arc<dyn RerankingModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.reranking(model_id))))
    }
}

impl ProviderDiscovery for AmazonBedrockProvider {
    /// `GET /foundation-models` (the Bedrock `ListFoundationModels` API): one
    /// exchange, no retry, authenticated like any other request.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let auth = self.auth.clone();
        let base_url = self.base_url.clone();
        let config = self.runtime_config();
        Box::pin(async move {
            // `ListFoundationModels` is on the control plane, whose host is
            // `bedrock.{region}.amazonaws.com` (the runtime is
            // `bedrock-runtime.{region}.amazonaws.com`); an explicit base URL
            // replaces it.
            let url = match &base_url {
                Some(base) => format!("{base}/foundation-models"),
                None => format!(
                    "https://bedrock.{}.amazonaws.com/foundation-models",
                    auth.region()?
                ),
            };
            let exchange = config.exchange(None).await?;
            let mut headers = exchange.headers();
            headers.push(("Accept".to_string(), "application/json".to_string()));
            let resp = aimux_provider_utils::get_from_api(
                exchange.with_transport(aimux_provider_utils::HttpRequest {
                    url,
                    headers,
                    ..Default::default()
                }),
                aimux_provider_utils::create_json_response_handler(),
                bedrock_failed_response_handler(),
            )
            .await?;

            // AWS response: { modelSummaries: [{ modelId, modelName, ... }] }
            #[derive(serde::Deserialize)]
            struct Resp {
                #[serde(default, rename = "modelSummaries")]
                summaries: Vec<Entry>,
            }
            #[derive(serde::Deserialize)]
            struct Entry {
                #[serde(rename = "modelId")]
                id: String,
                #[serde(default, rename = "modelName")]
                name: Option<String>,
            }
            let parsed: Resp = resp.value;
            Ok(parsed
                .summaries
                .into_iter()
                .map(|entry| RuntimeModel {
                    id: entry.id,
                    owned_by: entry.name.or(Some("amazon".to_string())),
                    created: None,
                })
                .collect())
        })
    }
}
