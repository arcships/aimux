//! The Anthropic-AWS provider factory and default instance
//! (`anthropic-aws-provider.ts`).

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_provider_utils::{
    AwsCredentials, FetchFunction, HeaderMapOpt, Resolvable, validate_base_url,
};

use crate::anthropic::AnthropicMessagesModel;
use crate::anthropic::config::{
    AnthropicEndpoint, AnthropicHeaders, AnthropicModelConfig, AnthropicModelHooks,
};
use crate::anthropic::options::{CANONICAL, options_name_of};
use crate::shared::TransformRequestBody;

use super::fetch::{self, AwsHeaders};

/// An Anthropic-AWS language model: the shared Messages model configured for
/// the AWS host.
pub type AnthropicAwsModel = AnthropicMessagesModel;

/// The provider name when the settings give none, the `provider` of
/// upstream's chat model (anthropic-aws-provider.ts:246).
const DEFAULT_NAME: &str = "anthropic-aws.messages";

/// Authentication method for the Anthropic-AWS provider.
#[derive(Clone, Debug)]
pub enum AnthropicAwsAuth {
    /// `x-api-key` header authentication with an AWS-provisioned key. A
    /// [`Resolvable::Future`] is awaited once, an [`Resolvable::AsyncFn`] on
    /// every request.
    ApiKey(Resolvable<String>),
    /// AWS SigV4 signing (service `aws-external-anthropic`). The credentials
    /// are resolved on every request, so a refreshing producer can hand out
    /// rotating STS credentials.
    SigV4(Resolvable<AwsCredentials>),
}

/// Settings of [`create_anthropic_aws`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url`, an explicit `region` and `name`; the region,
/// the workspace id, credentials and `headers` are resolved on every request.
#[derive(Clone, Default)]
pub struct AnthropicAwsProviderSettings {
    /// Base URL for the API calls, version segment included. Default
    /// `https://aws-external-anthropic.{region}.api.aws/v1`; a trailing slash
    /// is removed.
    pub base_url: Option<String>,
    /// The AWS region: the host of the default base URL and the SigV4
    /// signing region. `None` loads `AWS_REGION` when a request is made, else
    /// the region of `Resolvable::Value` SigV4 credentials; with neither the
    /// request fails with `AiMuxError::LoadSetting`. There is no default
    /// region. The setting and `AWS_REGION` win over the region SigV4
    /// credentials carry.
    pub region: Option<String>,
    /// How requests are authenticated. `None` decides per request, as the AI
    /// SDK does: `x-api-key` with `ANTHROPIC_AWS_API_KEY` when it is set and
    /// not empty, else SigV4 with `AWS_ACCESS_KEY_ID`,
    /// `AWS_SECRET_ACCESS_KEY` and the optional `AWS_SESSION_TOKEN` (a
    /// missing key fails the request with `AiMuxError::Other`, upstream's
    /// guidance on providing credentials).
    pub auth: Option<AnthropicAwsAuth>,
    /// Sent as `anthropic-workspace-id`. `None` loads
    /// `ANTHROPIC_AWS_WORKSPACE_ID` when a request is made. Required: with
    /// neither, the request fails with `AiMuxError::LoadSetting` and nothing
    /// is sent.
    pub workspace_id: Option<String>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including one of the fixed ones. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the `provider()` string of the messages model.
    /// Default `"anthropic-aws.messages"`. The providerOptions key is always
    /// `anthropic`, plus the first dot-separated segment of a name that was
    /// given here.
    pub name: Option<String>,
    /// The transport SigV4 signing (when used) wraps. `None` uses the process
    /// default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Rewrites every JSON request body once, after it is serialized and
    /// before it is sent (and before it is signed).
    pub transform_request_body: Option<TransformRequestBody>,
}

impl std::fmt::Debug for AnthropicAwsProviderSettings {
    /// Never prints credentials or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicAwsProviderSettings")
            .field("base_url", &self.base_url)
            .field("region", &self.region)
            .field("auth", &self.auth)
            .field("workspace_id", &self.workspace_id)
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

/// Create an Anthropic-AWS provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
/// URL with a host, or, with no `base_url`, an explicit `region` is not a
/// single DNS label. Those are the only ways this fails: the region, the
/// workspace id and the credentials are loaded per request, not here.
pub fn create_anthropic_aws(
    settings: AnthropicAwsProviderSettings,
) -> Result<AnthropicAwsProvider, AiMuxError> {
    let auth = Arc::new(fetch::Auth {
        region: settings.region,
        workspace_id: settings.workspace_id,
        auth: settings.auth,
    });
    let base_url = match settings.base_url.as_deref() {
        Some(url) => Some(validate_base_url(url)?),
        None => {
            if auth.region.is_some() {
                auth.host_region()?;
            }
            None
        }
    };
    // The providerOptions key stays `anthropic` unless the caller named the
    // provider.
    let provider_options_name = settings
        .name
        .as_deref()
        .map_or_else(|| CANONICAL.to_string(), options_name_of);
    let name = settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string());
    let (headers, fetch) = fetch::authenticated(auth.clone(), settings.headers, settings.fetch);
    Ok(AnthropicAwsProvider {
        name,
        provider_options_name,
        auth,
        base_url,
        headers,
        fetch,
        transform_request_body: settings.transform_request_body,
    })
}

/// The default provider: `create_anthropic_aws` with default settings, created
/// on first use. Creating it reads nothing from the environment and cannot
/// fail; a missing region, workspace id or credential surfaces from the first
/// request instead.
pub fn anthropic_aws() -> &'static AnthropicAwsProvider {
    static DEFAULT: OnceLock<AnthropicAwsProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_anthropic_aws(AnthropicAwsProviderSettings::default())
            .expect("default Anthropic-AWS settings are always valid")
    })
}

/// An Anthropic-AWS provider. Cheap to clone the models out of; it holds no
/// HTTP client.
pub struct AnthropicAwsProvider {
    name: String,
    provider_options_name: String,
    auth: Arc<fetch::Auth>,
    /// The explicit base URL; `None` builds the regional one per request.
    base_url: Option<String>,
    headers: AwsHeaders,
    fetch: FetchFunction,
    transform_request_body: Option<TransformRequestBody>,
}

impl AnthropicAwsProvider {
    /// The configuration of requests against `base_url`.
    fn config_for(&self, base_url: String) -> AnthropicModelConfig {
        AnthropicModelConfig {
            provider: self.name.clone(),
            headers: AnthropicHeaders::Aws(self.headers.clone()),
            fetch: Some(self.fetch.clone()),
            // The Messages API fetches http(s) images and PDFs itself
            // (anthropic-aws-provider.ts:251-254).
            supported_urls: crate::anthropic::supported_urls(),
            transform_request_body: self.transform_request_body.clone(),
            base_url,
            provider_options_name: self.provider_options_name.clone(),
            hooks: AnthropicModelHooks::default(),
            endpoint: AnthropicEndpoint::Fixed,
        }
    }

    /// The model configuration. With no explicit base URL every request is
    /// built against `https://aws-external-anthropic.{region}.api.aws/v1` for
    /// the region resolved then (`getBaseURL`, anthropic-aws-provider.ts:229-231).
    fn model_config(&self) -> AnthropicModelConfig {
        if let Some(base_url) = &self.base_url {
            return self.config_for(base_url.clone());
        }
        AnthropicModelConfig {
            endpoint: AnthropicEndpoint::AwsRegional(self.auth.clone()),
            ..self.config_for(String::new())
        }
    }

    /// A Messages model; `provider()` is the provider name
    /// (`"anthropic-aws.messages"` by default).
    #[must_use]
    pub fn messages(&self, model_id: &str) -> AnthropicAwsModel {
        AnthropicMessagesModel::with_config(model_id.to_string(), self.model_config())
    }

    /// The provider as a function: the default language model for an id.
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.messages(model_id))
    }
}

impl Provider for AnthropicAwsProvider {
    fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
        Some(self)
    }

    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }
}

impl ProviderDiscovery for AnthropicAwsProvider {
    /// `GET {base_url}/models`: one exchange, no retry. Signed like any other
    /// request when the provider uses SigV4.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.model_config();
        Box::pin(async move { crate::anthropic::list_models_once(&config).await })
    }
}
