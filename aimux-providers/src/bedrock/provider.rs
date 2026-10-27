//! The Amazon Bedrock provider factory (amazon-bedrock-provider.ts).

use super::chat_language_model::BedrockModel;
use super::chat_language_model_options::AmazonBedrockChatModelSettings;
use super::embedding_model::BedrockEmbeddingModel;
use super::embedding_model_options::AmazonBedrockEmbeddingModelSettings;
use super::error::bedrock_failed_response_handler;
use super::image_model::BedrockImageModel;
use super::reranking::BedrockRerankingModel;
use super::sigv4_fetch::{Auth, BedrockAuthFetch, ProcessDefaultFetch};
use crate::shared::{Endpoint, EndpointConfig, ResolveEndpoint, TransformRequestBody};
use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::reranking_model::RerankingModel;
use aimux_provider_utils::{
    AwsCredentials, FetchFunction, HeaderMapOpt, Resolvable, SigV4Fetch, SigV4Signing,
    combine_headers, validate_base_url, without_trailing_slash,
};
use futures::future::BoxFuture;
use std::sync::{Arc, OnceLock};

pub(super) const PROVIDER: &str = "amazon-bedrock";

pub(super) const SIGV4_SERVICE: &str = "bedrock";

pub(super) const REGION_ENV_VAR: &str = "AWS_REGION";

pub(crate) const BEARER_TOKEN_ENV_VAR: &str = "AWS_BEARER_TOKEN_BEDROCK";

pub(super) const ACCESS_KEY_ID_ENV_VAR: &str = "AWS_ACCESS_KEY_ID";

pub(super) const SECRET_ACCESS_KEY_ENV_VAR: &str = "AWS_SECRET_ACCESS_KEY";

pub(super) const SESSION_TOKEN_ENV_VAR: &str = "AWS_SESSION_TOKEN";

pub(super) const RUNTIME_ENDPOINT_ENV_VAR: &str = "AWS_ENDPOINT_URL_BEDROCK_RUNTIME";

pub(super) const AGENT_RUNTIME_ENDPOINT_ENV_VAR: &str = "AWS_ENDPOINT_URL_BEDROCK_AGENT_RUNTIME";

pub(crate) fn encode_model_id(model_id: &str) -> String {
    const SAFE: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'!')
        .remove(b'~')
        .remove(b'*')
        .remove(b'\'')
        .remove(b'(')
        .remove(b')');
    percent_encoding::utf8_percent_encode(model_id, SAFE).to_string()
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

/// Create an Amazon Bedrock provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when an explicit `base_url` is not an
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

    let credentials_auth = auth.clone();
    let inner = settings
        .fetch
        .unwrap_or_else(|| Arc::new(ProcessDefaultFetch));
    let fetch: FetchFunction = Arc::new(BedrockAuthFetch {
        signing: SigV4Fetch::new(
            inner.clone(),
            Resolvable::from_async_fn(move || {
                let auth = credentials_auth.clone();
                async move { auth.credentials().await }
            }),
            SIGV4_SERVICE,
            // aws4fetch (amazon-bedrock-sigv4-fetch.ts:73-83) signs
            // `host;x-amz-date` (+ `x-amz-security-token`).
            SigV4Signing::Aws4Fetch,
        ),
        inner,
    });

    Ok(AmazonBedrockProvider {
        auth,
        base_url: base_url.map(|url| without_trailing_slash(&url)),
        user_headers: settings.headers,
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
    user_headers: Option<HeaderMapOpt>,
    fetch: FetchFunction,
    transform_request_body: Option<TransformRequestBody>,
}

/// The endpoint of a Bedrock request: the explicit base URL, the service's
/// endpoint environment variable or the regional host; the caller's headers
/// with the package user-agent part, and an API key as `Authorization: Bearer`
/// when there is one (otherwise the transport signs with SigV4, after
/// checking here that signing has its inputs).
#[derive(Clone)]
struct BedrockEndpoint {
    auth: Arc<Auth>,
    base_url: Option<String>,
    /// `bedrock-runtime` or `bedrock-agent-runtime`.
    service: &'static str,
    endpoint_env_var: &'static str,
    user: Option<HeaderMapOpt>,
}

#[async_trait::async_trait]
impl ResolveEndpoint for BedrockEndpoint {
    /// The base URL and provider headers of one request.
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        let base_url = Auth::base_url(
            self.base_url.as_deref(),
            self.service,
            self.endpoint_env_var,
            &self.auth,
        )?;
        Ok(Endpoint {
            base_url,
            headers: self.headers().await?,
        })
    }
}

impl BedrockEndpoint {
    async fn headers(&self) -> Result<HeaderMapOpt, AiMuxError> {
        let mut user = combine_headers(&[&self.user.clone().unwrap_or_default()]);
        // amazon-bedrock-provider.ts:292-295 (getHeaders) appends the pinned
        // package part. The helper then adds `ai-sdk-provider-utils/<v>` and
        // the transport (`BedrockAuthFetch`) the second occurrence.
        let package = crate::shared::aisdk_pins::AMAZON_BEDROCK;
        let suffix = format!("ai-sdk-{}/{}", package.name, package.version);
        let agent = user
            .get("user-agent")
            .and_then(|value| value.as_deref())
            .map_or_else(|| suffix.clone(), |value| format!("{value} {suffix}"));
        user.insert("user-agent".into(), Some(agent));
        match self.auth.bearer().await? {
            Some(key) => {
                let mut bearer = HeaderMapOpt::new();
                bearer.insert("Authorization".to_string(), Some(format!("Bearer {key}")));
                Ok(combine_headers(&[&user, &bearer]))
            }
            None => {
                self.auth.preflight()?;
                Ok(user)
            }
        }
    }
}

impl AmazonBedrockProvider {
    /// The configuration of a model that calls `service` (`bedrock-runtime`
    /// or `bedrock-agent-runtime`).
    fn model_config(
        &self,
        service: &'static str,
        endpoint_env_var: &'static str,
    ) -> EndpointConfig {
        EndpointConfig::new(
            PROVIDER.to_string(),
            BedrockEndpoint {
                auth: self.auth.clone(),
                base_url: self.base_url.clone(),
                service,
                endpoint_env_var,
                user: self.user_headers.clone(),
            },
            Some(self.fetch.clone()),
        )
        .with_transform_request_body(self.transform_request_body.clone())
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

    #[must_use]
    pub fn chat_with_settings(
        &self,
        model_id: &str,
        settings: &AmazonBedrockChatModelSettings,
    ) -> BedrockModel {
        self.chat(model_id).with_model_family(
            settings
                .model_family
                .map(|family| family.as_str().to_owned()),
        )
    }

    /// An embedding model (e.g. `"amazon.titan-embed-text-v2:0"`);
    /// `provider()` is `"amazon-bedrock"`.
    #[must_use]
    pub fn embedding(&self, model_id: &str) -> BedrockEmbeddingModel {
        BedrockEmbeddingModel::from_config(model_id.to_string(), self.runtime_config())
    }

    #[must_use]
    pub fn embedding_with_settings(
        &self,
        model_id: &str,
        settings: &AmazonBedrockEmbeddingModelSettings,
    ) -> BedrockEmbeddingModel {
        self.embedding(model_id)
            .with_model_family(settings.model_family)
    }

    #[must_use]
    pub fn call_with_settings(
        &self,
        model_id: &str,
        settings: &AmazonBedrockChatModelSettings,
    ) -> Arc<dyn LanguageModel> {
        Arc::new(self.chat_with_settings(model_id, settings))
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
        BedrockRerankingModel::from_config(
            model_id.to_string(),
            self.model_config("bedrock-agent-runtime", AGENT_RUNTIME_ENDPOINT_ENV_VAR),
            self.auth.clone(),
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
            let mut request = exchange.with_transport(aimux_provider_utils::HttpRequest {
                url,
                headers,
                ..Default::default()
            });
            request.credentialed_origin = Some(request.url.clone());
            let resp = aimux_provider_utils::get_from_api(
                request,
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
