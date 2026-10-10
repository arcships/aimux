//! Anthropic-AWS (Claude Platform on AWS) provider.
//!
//! The Claude Platform on AWS API (`aws-external-anthropic.{region}.api.aws/v1`)
//! is the Anthropic Messages API hosted in AWS, so [`create_anthropic_aws`]
//! returns a provider whose models are the shared
//! [`AnthropicMessagesModel`]. Only
//! the endpoint and the authentication differ: an AWS-provisioned API key, or
//! AWS SigV4 applied by a [`SigV4Fetch`](aimux_provider_utils::SigV4Fetch)
//! transport decorator over the final request.
//!
//! Reference: <https://docs.anthropic.com/en/api/messages>

mod model;

pub use model::AnthropicAwsModel;

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_provider_utils::{
    AwsCredentials, FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url,
};

use crate::anthropic::AnthropicMessagesModel;
use crate::anthropic::config::{AnthropicModelConfig, AnthropicModelHooks};
use crate::anthropic::options::{CANONICAL, options_name_of};

/// The region used when the settings name none and SigV4 credentials do not
/// carry one.
const DEFAULT_REGION: &str = "us-east-1";

/// The provider name when the settings give none.
const DEFAULT_NAME: &str = "anthropic-aws";

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
/// created except `region`, `base_url` and `name`; credentials and `headers`
/// are evaluated on every request.
#[derive(Clone, Default)]
pub struct AnthropicAwsProviderSettings {
    /// Base URL for the API calls, version segment included. Default
    /// `https://aws-external-anthropic.{region}.api.aws/v1`; a trailing slash
    /// is removed.
    pub base_url: Option<String>,
    /// The AWS region of the default base URL. When unset, the region of
    /// `Resolvable::Value` SigV4 credentials, else `us-east-1`. Never read
    /// from the environment.
    pub region: Option<String>,
    /// How requests are authenticated. `None` sends `x-api-key` from
    /// `ANTHROPIC_AWS_API_KEY`, loaded when a request is made (a missing
    /// variable fails that request with `AiMuxError::LoadApiKey`).
    pub auth: Option<AnthropicAwsAuth>,
    /// Sent as `anthropic-workspace-id`.
    pub workspace_id: Option<String>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including one of the fixed ones. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the `provider()` string of the messages model.
    /// Default `"anthropic-aws"`. The providerOptions key is always
    /// `anthropic`, plus the first dot-separated segment of a name that was
    /// given here.
    pub name: Option<String>,
    /// The transport SigV4 signing (when used) wraps. `None` uses the process
    /// default, resolved per request.
    pub fetch: Option<FetchFunction>,
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
            .finish()
    }
}

/// A region is a hostname label: refuse anything that could change the host
/// of the default base URL.
fn validate_region(region: &str) -> Result<(), AiMuxError> {
    let valid = !region.is_empty()
        && region
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if valid {
        Ok(())
    } else {
        Err(AiMuxError::InvalidArgument(format!(
            "invalid AWS region {region:?}: expected letters, digits and hyphens"
        )))
    }
}

/// Create an Anthropic-AWS provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host or `region` is not a valid hostname label. Those are the
/// only ways this fails: credentials are loaded per request, not here.
pub fn create_anthropic_aws(
    settings: AnthropicAwsProviderSettings,
) -> Result<AnthropicAwsProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => {
            let from_credentials = match &settings.auth {
                Some(AnthropicAwsAuth::SigV4(Resolvable::Value(credentials))) => {
                    Some(credentials.region.clone())
                }
                _ => None,
            };
            let region = settings
                .region
                .clone()
                .or(from_credentials)
                .unwrap_or_else(|| DEFAULT_REGION.to_string());
            validate_region(&region)?;
            format!("https://aws-external-anthropic.{region}.api.aws/v1")
        }
    };
    // The providerOptions key stays `anthropic` unless the caller named the
    // provider.
    let provider_options_name = settings
        .name
        .as_deref()
        .map_or_else(|| CANONICAL.to_string(), options_name_of);
    let name = settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string());
    let (headers, fetch) = model::authenticated(
        settings.auth,
        settings.workspace_id,
        settings.headers,
        settings.fetch,
    );
    Ok(AnthropicAwsProvider {
        name,
        provider_options_name,
        base_url,
        headers,
        fetch,
    })
}

/// The default provider: `create_anthropic_aws` with default settings, created
/// on first use. Creating it reads nothing from the environment and cannot
/// fail; a missing key surfaces from the first request instead.
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
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl AnthropicAwsProvider {
    fn model_config(&self) -> AnthropicModelConfig {
        let base = self.base_url.clone();
        AnthropicModelConfig {
            provider: self.name.clone(),
            url: Arc::new(move |path| format!("{base}{path}")),
            headers: self.headers.clone(),
            fetch: self.fetch.clone(),
            supported_urls: SupportedUrls::default(),
            base_url: self.base_url.clone(),
            provider_options_name: self.provider_options_name.clone(),
            hooks: AnthropicModelHooks::default(),
            resolve: None,
        }
    }

    /// A Messages model; `provider()` is the provider name
    /// (`"anthropic-aws"` by default).
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
