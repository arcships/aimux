//! Anthropic-AWS model wiring.
//!
//! Claude Platform on AWS serves the Anthropic Messages API unchanged, so its
//! model *is* the shared [`AnthropicMessagesModel`]. What is specific to the
//! host is where a request goes and how it is authenticated, and that is data
//! on the model's configuration: an `x-api-key` header, or a [`SigV4Fetch`]
//! transport decorator that signs the final method, URL, headers and body
//! bytes just before they are sent.
//!
//! The settings and the environment are read when a request is made, by the
//! rules of `@ai-sdk/anthropic-aws` 2.0.63 (src/anthropic-aws-provider.ts):
//! the region from the setting or `AWS_REGION` with no default, a required
//! workspace id from the setting or `ANTHROPIC_AWS_WORKSPACE_ID`, and, with no
//! `auth`, `ANTHROPIC_AWS_API_KEY` when set and SigV4 with the `AWS_*`
//! credentials otherwise.

use async_trait::async_trait;

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{
    AwsCredentials, Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HeaderMapOpt,
    Resolvable, SigV4Fetch, SigV4Signing, combine_headers, load_optional_setting, load_setting,
};

use crate::anthropic::AnthropicMessagesModel;
use crate::shared::is_valid_hostname_part;

use super::AnthropicAwsAuth;

/// An Anthropic-AWS language model: the shared Messages model configured for
/// the AWS host.
pub type AnthropicAwsModel = AnthropicMessagesModel;

/// The SigV4 service name of Claude Platform on AWS.
const SIGV4_SERVICE: &str = "aws-external-anthropic";

const API_VERSION: &str = "2023-06-01";
const API_KEY_ENV_VAR: &str = "ANTHROPIC_AWS_API_KEY";
const WORKSPACE_ID_ENV_VAR: &str = "ANTHROPIC_AWS_WORKSPACE_ID";
const REGION_ENV_VAR: &str = "AWS_REGION";
const ACCESS_KEY_ID_ENV_VAR: &str = "AWS_ACCESS_KEY_ID";
const SECRET_ACCESS_KEY_ENV_VAR: &str = "AWS_SECRET_ACCESS_KEY";
const SESSION_TOKEN_ENV_VAR: &str = "AWS_SESSION_TOKEN";

/// Where the region, the workspace id and the credentials of a request come
/// from.
pub(crate) struct Auth {
    pub(super) region: Option<String>,
    pub(super) workspace_id: Option<String>,
    pub(super) auth: Option<AnthropicAwsAuth>,
}

impl Auth {
    /// The setting, `AWS_REGION`, or the region of plain-value SigV4
    /// credentials (aimux's credentials carry one). There is no default:
    /// upstream loads it with `loadSetting` (anthropic-aws-provider.ts:137-143,
    /// :212-218).
    fn region(&self) -> Result<String, AiMuxError> {
        if let Some(region) = load_optional_setting(self.region.as_deref(), REGION_ENV_VAR) {
            return Ok(region);
        }
        if let Some(AnthropicAwsAuth::SigV4(Resolvable::Value(credentials))) = &self.auth
            && !credentials.region.is_empty()
        {
            return Ok(credentials.region.clone());
        }
        load_setting(None, REGION_ENV_VAR, "region")
    }

    /// The region of the default base URL: a single DNS label
    /// (anthropic-aws-provider.ts:212-227).
    pub(crate) fn host_region(&self) -> Result<String, AiMuxError> {
        let region = self.region()?;
        if !is_valid_hostname_part(&region) {
            return Err(AiMuxError::InvalidArgument(
                "Invalid AWS region. Expected a single DNS label (letters, digits, and hyphens). \
                 Use `base_url` for custom endpoints."
                    .to_string(),
            ));
        }
        Ok(region)
    }

    /// Required on every request (`getHeaders`, anthropic-aws-provider.ts:233-242).
    fn workspace_id(&self) -> Result<String, AiMuxError> {
        load_setting(
            self.workspace_id.as_deref(),
            WORKSPACE_ID_ENV_VAR,
            "workspace_id",
        )
    }

    /// The `x-api-key` of the request, when it uses one: the explicit key, or
    /// with no `auth` a non-empty `ANTHROPIC_AWS_API_KEY`
    /// (anthropic-aws-provider.ts:130-136).
    async fn api_key(&self) -> Result<Option<String>, AiMuxError> {
        match &self.auth {
            Some(AnthropicAwsAuth::ApiKey(key)) => key.resolve().await.map(Some),
            Some(AnthropicAwsAuth::SigV4(_)) => Ok(None),
            None => Ok(load_optional_setting(None, API_KEY_ENV_VAR).filter(|key| !key.is_empty())),
        }
    }

    /// The `AWS_*` credentials, for SigV4 with no `auth`
    /// (anthropic-aws-provider.ts:162-209). A missing key fails with
    /// upstream's guidance and the setting error as the original error; a
    /// missing region stays the setting error, as upstream loads it first.
    fn environment_credentials(&self) -> Result<AwsCredentials, AiMuxError> {
        let region = self.region()?;
        let access_key_id = load_setting(None, ACCESS_KEY_ID_ENV_VAR, "access_key_id")
            .map_err(|error| {
                AiMuxError::Other(format!(
                    "AWS SigV4 authentication requires AWS credentials. Please provide either:\n\
                     1. Set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY environment variables\n\
                     2. Provide the credentials as `AnthropicAwsAuth::SigV4(Resolvable::Value(..))`\n\
                     3. Use a credential producer in `AnthropicAwsAuth::SigV4`\n\
                     4. Use API key authentication with ANTHROPIC_AWS_API_KEY or `AnthropicAwsAuth::ApiKey`\n\
                     Original error: {error}"
                ))
            })?;
        let secret_access_key = load_setting(None, SECRET_ACCESS_KEY_ENV_VAR, "secret_access_key")
            .map_err(|error| {
                AiMuxError::Other(format!(
                    "AWS SigV4 authentication requires both AWS_ACCESS_KEY_ID and \
                     AWS_SECRET_ACCESS_KEY. Please ensure both credentials are provided.\n\
                     Original error: {error}"
                ))
            })?;
        Ok(AwsCredentials {
            region,
            access_key_id,
            secret_access_key,
            session_token: load_optional_setting(None, SESSION_TOKEN_ENV_VAR),
        })
    }

    /// The credentials to sign one request with. The region of the setting
    /// or `AWS_REGION` wins over the producer's, as upstream spreads the
    /// loaded region over the credential provider's result (:145-150); a
    /// producer failure carries upstream's guidance (:151-160).
    async fn credentials(&self) -> Result<AwsCredentials, AiMuxError> {
        match &self.auth {
            Some(AnthropicAwsAuth::SigV4(producer)) => {
                let mut credentials = producer.resolve().await.map_err(|error| {
                    AiMuxError::Other(format!(
                        "AWS credential provider failed: {error}. Please ensure your credential \
                         provider returns valid AWS credentials with access_key_id and \
                         secret_access_key."
                    ))
                })?;
                if let Some(region) = load_optional_setting(self.region.as_deref(), REGION_ENV_VAR)
                {
                    credentials.region = region;
                } else if credentials.region.is_empty() {
                    return Err(AiMuxError::LoadSetting {
                        env_var: REGION_ENV_VAR.to_string(),
                        name: "region".to_string(),
                    });
                }
                Ok(credentials)
            }
            _ => self.environment_credentials(),
        }
    }

    /// Fail before an environment-credential request is signed: the
    /// transport reports a credential failure as a transport error, which
    /// would hide which setting is missing.
    fn preflight(&self) -> Result<(), AiMuxError> {
        match &self.auth {
            Some(AnthropicAwsAuth::SigV4(_)) => Ok(()),
            _ => self.environment_credentials().map(|_| ()),
        }
    }
}

/// The provider headers and the transport that authenticate requests.
///
/// The headers carry `anthropic-version`, the workspace id, the caller's
/// headers and, on the API-key path, `x-api-key` last, as
/// anthropic-aws-fetch.ts:132-134 combines it. The transport signs every
/// request that carries no `x-api-key`: one decision, made by the headers.
pub(super) fn authenticated(
    auth: std::sync::Arc<Auth>,
    user_headers: Option<HeaderMapOpt>,
    inner: FetchFunction,
) -> (AwsHeaders, FetchFunction) {
    let headers = AwsHeaders {
        auth: auth.clone(),
        user: user_headers,
    };
    let credentials_auth = auth;
    let fetch: FetchFunction = std::sync::Arc::new(AnthropicAwsFetch {
        // aws4fetch, like anthropic-aws-fetch.ts:73-83.
        signing: SigV4Fetch::new(
            inner.clone(),
            Resolvable::from_async_fn(move || {
                let auth = credentials_auth.clone();
                async move { auth.credentials().await }
            }),
            SIGV4_SERVICE,
            SigV4Signing::Aws4Fetch,
        ),
        inner,
    });
    (headers, fetch)
}

/// The provider headers of Claude Platform on AWS, resolved on every
/// request: `anthropic-version`, the workspace id, the caller's headers and,
/// on the API-key path, `x-api-key` last, as anthropic-aws-fetch.ts:132-134
/// combines it.
#[derive(Clone)]
pub(crate) struct AwsHeaders {
    auth: std::sync::Arc<Auth>,
    user: Option<HeaderMapOpt>,
}

impl AwsHeaders {
    pub(crate) async fn resolve(&self) -> Result<HeaderMapOpt, AiMuxError> {
        let auth = &self.auth;
        let user = self.user.clone().unwrap_or_default();
        let mut fixed = HeaderMapOpt::new();
        fixed.insert("anthropic-version".into(), Some(API_VERSION.into()));
        fixed.insert("anthropic-workspace-id".into(), Some(auth.workspace_id()?));
        let mut key = HeaderMapOpt::new();
        match auth.api_key().await? {
            Some(api_key) => {
                key.insert("x-api-key".into(), Some(api_key));
            }
            None => auth.preflight()?,
        }
        Ok(combine_headers(&[&fixed, &user, &key]))
    }
}

/// Adds the transport's user-agent part, then signs the request unless it
/// carries an `x-api-key`.
struct AnthropicAwsFetch {
    signing: SigV4Fetch,
    inner: FetchFunction,
}

#[async_trait]
impl Fetch for AnthropicAwsFetch {
    async fn fetch(&self, mut request: FetchRequest) -> Result<FetchResponse, FetchError> {
        // `ai-sdk-anthropic-aws/<v>` after the helper's
        // `ai-sdk-provider-utils/<v>` and before any signing, on the API-key
        // and SigV4 paths alike (anthropic-aws-fetch.ts:42-46, :126-130).
        let package = crate::shared::aisdk_pins::ANTHROPIC_AWS;
        crate::shared::transport_user_agent(
            &mut request.headers,
            Some(package.provider_utils),
            Some(&format!("ai-sdk-{}/{}", package.name, package.version)),
        );
        if request.headers.contains_key("x-api-key") {
            self.inner.fetch(request).await
        } else {
            self.signing.fetch(request).await
        }
    }
}
