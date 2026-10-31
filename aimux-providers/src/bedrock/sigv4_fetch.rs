//! Request authentication of the Amazon Bedrock provider (amazon-bedrock-sigv4-fetch.ts):
//! the API key or SigV4 decision, credential loading, and the regional base URL
//! (resolve-amazon-bedrock-base-url.ts).

use super::provider::{
    ACCESS_KEY_ID_ENV_VAR, BEARER_TOKEN_ENV_VAR, REGION_ENV_VAR, SECRET_ACCESS_KEY_ENV_VAR,
    SESSION_TOKEN_ENV_VAR,
};
use aimux_core::error::AiMuxError;
use aimux_provider_utils::{
    AwsCredentials, Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, Resolvable,
    SigV4Fetch, default_fetch, is_valid_hostname_part, load_optional_setting, load_setting,
    validate_base_url,
};
use async_trait::async_trait;

/// Where the region, the API key and the AWS credentials of a request come
/// from.
pub(crate) struct Auth {
    pub(super) region: Option<String>,
    pub(super) api_key: Option<Resolvable<String>>,
    pub(super) access_key_id: Option<String>,
    pub(super) secret_access_key: Option<String>,
    pub(super) session_token: Option<String>,
    pub(super) credential_provider: Option<Resolvable<AwsCredentials>>,
}

impl Auth {
    /// The Bearer API key, when there is a non-blank one (trimmed).
    pub(super) async fn bearer(&self) -> Result<Option<String>, AiMuxError> {
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
    pub(crate) fn region(&self) -> Result<String, AiMuxError> {
        let region =
            if let Some(region) = load_optional_setting(self.region.as_deref(), REGION_ENV_VAR) {
                region
            } else if let Some(Resolvable::Value(credentials)) = &self.credential_provider
                && !credentials.region.is_empty()
            {
                credentials.region.clone()
            } else {
                load_setting(None, REGION_ENV_VAR, "region")?
            };
        if !is_valid_hostname_part(&region) {
            return Err(AiMuxError::invalid_argument(
                "Invalid Amazon Bedrock region. Expected a single DNS label (letters, digits, and \
                 hyphens). Use `base_url` for custom endpoints."
                    .to_string(),
            ));
        }
        Ok(region)
    }

    /// The credentials of the settings and the environment (no provider).
    pub(super) fn static_credentials(&self) -> Result<AwsCredentials, AiMuxError> {
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
    pub(super) async fn credentials(&self) -> Result<AwsCredentials, AiMuxError> {
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
    pub(super) fn preflight(&self) -> Result<(), AiMuxError> {
        if self.credential_provider.is_some() {
            return Ok(());
        }
        self.static_credentials().map(|_| ())
    }

    /// The base URL of `service`: the explicit one, the service's endpoint
    /// environment variable, or the regional host.
    pub(super) fn base_url(
        explicit: Option<&str>,
        service: &str,
        endpoint_env_var: &str,
        auth: &Self,
    ) -> Result<String, AiMuxError> {
        if let Some(url) = explicit {
            return Ok(url.to_string());
        }
        if let Some(url) = load_optional_setting(None, endpoint_env_var)
            .or_else(|| load_optional_setting(None, "AWS_ENDPOINT_URL"))
        {
            return validate_base_url(&url);
        }
        let region = auth.region()?;
        let suffix = [
            ("cn-", "amazonaws.com.cn"),
            ("us-iso-", "c2s.ic.gov"),
            ("us-isob-", "sc2s.sgov.gov"),
            ("eu-isoe-", "cloud.adc-e.uk"),
            ("us-isof-", "csp.hci.ic.gov"),
            ("eusc-", "amazonaws.eu"),
        ]
        .iter()
        .find(|(prefix, _)| region.starts_with(prefix))
        .map_or("amazonaws.com", |(_, suffix)| *suffix);
        Ok(format!("https://{service}.{region}.{suffix}"))
    }
}

/// Signs a request with SigV4 unless it already carries an `Authorization`
/// header (the API-key path): one decision, made by the provider headers.
pub(super) struct BedrockAuthFetch {
    pub(super) signing: SigV4Fetch,
    pub(super) inner: FetchFunction,
}

#[async_trait]
impl Fetch for BedrockAuthFetch {
    async fn fetch(&self, mut request: FetchRequest) -> Result<FetchResponse, FetchError> {
        // The transport's `ai-sdk-amazon-bedrock/<v>`, after the helper's
        // `ai-sdk-provider-utils/<v>` (the 5.0.51 that amazon-bedrock pins)
        // and before signing, like amazon-bedrock-sigv4-fetch.ts:42-46
        // (SigV4) and :126-130 (API key).
        let package = crate::shared::aisdk_pins::AMAZON_BEDROCK;
        crate::shared::transport_user_agent(
            &mut request.headers,
            Some(package.provider_utils),
            Some(&format!("ai-sdk-{}/{}", package.name, package.version)),
        );
        if request.headers.contains_key(reqwest::header::AUTHORIZATION) {
            self.inner.fetch(request).await
        } else {
            self.signing.fetch(request).await
        }
    }
}

/// The process default transport, looked up on every request, as upstream
/// reads `globalThis.fetch` per call (amazon-bedrock-sigv4-fetch.ts:38, :129).
pub(super) struct ProcessDefaultFetch;

#[async_trait]
impl Fetch for ProcessDefaultFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        default_fetch().fetch(request).await
    }
}
