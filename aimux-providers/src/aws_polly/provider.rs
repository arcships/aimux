//! Amazon Polly provider factory, AWS credential chain, and endpoint resolution.

use std::sync::{Arc, OnceLock};

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{
    AwsCredentials, FetchFunction, HeaderMapOpt, Resolvable, SigV4Fetch, SigV4Signing,
    default_fetch, load_optional_setting, load_setting, validate_base_url,
};

use crate::shared::{Endpoint, EndpointConfig, ResolveEndpoint};

use super::speech_model::AwsPollySpeechModel;

/// AWS service name used for SigV4 signing.
const SERVICE_NAME: &str = "polly";

/// Default AWS region when `AWS_REGION` is unset.
const DEFAULT_REGION: &str = "us-east-1";

const DEFAULT_NAME: &str = "amazon-polly";
const REGION_ENV_VAR: &str = "AWS_REGION";
const ACCESS_KEY_ID_ENV_VAR: &str = "AWS_ACCESS_KEY_ID";
const SECRET_ACCESS_KEY_ENV_VAR: &str = "AWS_SECRET_ACCESS_KEY";
const SESSION_TOKEN_ENV_VAR: &str = "AWS_SESSION_TOKEN";

/// Settings of [`create_aws_polly`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; the region, the credentials and
/// `headers` are evaluated on every request.
#[derive(Clone, Default)]
pub struct AwsPollyProviderSettings {
    /// The AWS region. `None` loads `AWS_REGION` when a request is made and
    /// falls back to `us-east-1`.
    pub region: Option<String>,
    /// The AWS access key id of SigV4 signing. `None` loads
    /// `AWS_ACCESS_KEY_ID` when a request is made and fails that request with
    /// `AiMuxError::LoadSetting` if it is unset.
    pub access_key_id: Option<String>,
    /// The AWS secret access key of SigV4 signing. `None` loads
    /// `AWS_SECRET_ACCESS_KEY`.
    pub secret_access_key: Option<String>,
    /// The AWS session token of temporary credentials. When `access_key_id`
    /// and `secret_access_key` are both given only this field is used; when
    /// either comes from the environment the token also falls back to
    /// `AWS_SESSION_TOKEN`.
    pub session_token: Option<String>,
    /// Dynamic AWS credentials. When set they are used instead of
    /// `access_key_id`, `secret_access_key` and `session_token`; it is
    /// resolved on every request, so an [`Resolvable::AsyncFn`] can hand out
    /// rotating STS credentials. Its `region` is used only when neither
    /// `region` nor `AWS_REGION` names one.
    pub credential_provider: Option<Resolvable<AwsCredentials>>,
    /// Base URL for the API calls. Default `https://polly.{region}.amazonaws.com`;
    /// a trailing slash is removed.
    pub base_url: Option<String>,
    /// Extra headers on every request. A `None` value removes the header.
    /// Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` string
    /// (`"{name}.speech"`). Default `"amazon-polly"`. The providerOptions key
    /// is `aws_polly`.
    pub name: Option<String>,
    /// The transport SigV4 signing wraps. `None` uses the process default,
    /// resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for AwsPollyProviderSettings {
    /// Never prints keys, credentials or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsPollyProviderSettings")
            .field("region", &self.region)
            .field("access_key_id", &self.access_key_id.is_some())
            .field("secret_access_key", &self.secret_access_key.is_some())
            .field("session_token", &self.session_token.is_some())
            .field("credential_provider", &self.credential_provider.is_some())
            .field("base_url", &self.base_url)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Where the region and the AWS credentials of a request come from.
#[derive(Clone)]
struct Auth {
    region: Option<String>,
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    session_token: Option<String>,
    credential_provider: Option<Resolvable<AwsCredentials>>,
}

impl Auth {
    /// The region: the setting, `AWS_REGION`, the region of a plain-value
    /// credential provider, then `us-east-1`.
    fn region(&self) -> String {
        if let Some(region) = load_optional_setting(self.region.as_deref(), REGION_ENV_VAR) {
            return region;
        }
        if let Some(Resolvable::Value(credentials)) = &self.credential_provider
            && !credentials.region.is_empty()
        {
            return credentials.region.clone();
        }
        DEFAULT_REGION.to_string()
    }

    /// The credentials of the settings and the environment (no provider).
    fn static_credentials(&self) -> Result<AwsCredentials, AiMuxError> {
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
                .filter(|token| !token.trim().is_empty())
        };
        Ok(AwsCredentials {
            access_key_id,
            secret_access_key,
            session_token,
            region: self.region(),
        })
    }

    /// The credentials to sign one request with.
    async fn credentials(&self) -> Result<AwsCredentials, AiMuxError> {
        if let Some(provider) = &self.credential_provider {
            let mut credentials = provider.resolve().await?;
            if self.region.is_some() || std::env::var(REGION_ENV_VAR).is_ok() {
                credentials.region = self.region();
            } else if credentials.region.is_empty() {
                credentials.region = DEFAULT_REGION.to_string();
            }
            return Ok(credentials);
        }
        self.static_credentials()
    }

    /// Fail now, with the typed setting error, when signing could not find its
    /// inputs: the transport reports a credential failure as a transport
    /// error, which would hide which setting is missing. A credential provider
    /// is left to the signer.
    fn preflight(&self) -> Result<(), AiMuxError> {
        if self.credential_provider.is_none() {
            self.static_credentials()?;
        }
        Ok(())
    }
}

/// The endpoint of a Polly request: the explicit base URL or the regional
/// host, and only the caller's headers, after checking that signing has its
/// credentials (the signature itself is made by the transport).
#[derive(Clone)]
struct PollyEndpoint {
    auth: Auth,
    base_url: Option<String>,
    user: Option<HeaderMapOpt>,
}

#[async_trait::async_trait]
impl ResolveEndpoint for PollyEndpoint {
    /// The base URL and provider headers of one request.
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        let base_url = self
            .base_url
            .clone()
            .unwrap_or_else(|| format!("https://polly.{}.amazonaws.com", self.auth.region()));
        self.auth.preflight()?;
        Ok(Endpoint {
            base_url,
            headers: self.user.clone().unwrap_or_default(),
        })
    }
}

/// Create an Amazon Polly provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the region and the
/// credentials are loaded per request, not here.
pub fn create_aws_polly(
    settings: AwsPollyProviderSettings,
) -> Result<AwsPollyProvider, AiMuxError> {
    let base_url = settings
        .base_url
        .as_deref()
        .map(validate_base_url)
        .transpose()?;
    let auth = Auth {
        region: settings.region,
        access_key_id: settings.access_key_id,
        secret_access_key: settings.secret_access_key,
        session_token: settings.session_token,
        credential_provider: settings.credential_provider,
    };
    let signing_auth = auth.clone();
    let fetch: FetchFunction = Arc::new(SigV4Fetch::new(
        settings.fetch.unwrap_or_else(default_fetch),
        Resolvable::from_async_fn(move || {
            let auth = signing_auth.clone();
            async move { auth.credentials().await }
        }),
        SERVICE_NAME,
        // No AI SDK counterpart: botocore-style, locked by
        // tests/sigv4_signed_headers_test.rs.
        SigV4Signing::AllHeaders,
    ));
    Ok(AwsPollyProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        // SigV4 signs in the transport; the provider headers carry no credential.
        endpoint: PollyEndpoint {
            auth,
            base_url,
            user: settings.headers,
        },
        fetch,
    })
}

/// The default provider: `create_aws_polly` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// missing credentials surface from the first request instead.
pub fn aws_polly() -> &'static AwsPollyProvider {
    static DEFAULT: OnceLock<AwsPollyProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_aws_polly(AwsPollyProviderSettings::default())
            .expect("default Amazon Polly settings are always valid")
    })
}

/// An Amazon Polly provider. Speech only; it holds no HTTP client.
pub struct AwsPollyProvider {
    name: String,
    endpoint: PollyEndpoint,
    fetch: FetchFunction,
}

impl AwsPollyProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::new(
            format!("{}.{method}", self.name),
            self.endpoint.clone(),
            Some(self.fetch.clone()),
        )
    }

    /// A speech (TTS) model (an engine id such as `"neural"`); `provider()` is
    /// `"{name}.speech"`.
    #[must_use]
    pub fn speech(&self, model_id: &str) -> AwsPollySpeechModel {
        AwsPollySpeechModel::from_config(model_id.to_string(), self.model_config("speech"))
    }
}

crate::impl_single_modality_provider!(AwsPollyProvider, speech_model, |p, id| p.speech(id));
