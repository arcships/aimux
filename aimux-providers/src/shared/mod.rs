//! Helpers the native packages and the presets share.
//!
//! Only transport plumbing lives here: where the base URL and the
//! provider-level request headers of a request come from, as data. The few
//! vendors whose endpoint is decided by more than a URL and a credential
//! (Vertex, Bedrock, Polly) keep that logic in their own modules.

pub(crate) mod aisdk_pins;
mod discovery;
mod exchange;
mod poll;

pub(crate) use discovery::list_data_models;
pub(crate) use exchange::{
    BaseUrl, Endpoint, EndpointConfig, EndpointHeaders, EndpointSource, SupportedUrlsSource,
};
pub(crate) use poll::{
    POLL_INTERVAL_MILLIS_KEY, POLL_INTERVAL_MS_KEY, PollStep, is_poll_control_key,
    poll_interval_ms, poll_until, retry_download,
};

use std::sync::Arc;

use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_provider_utils::{
    FetchFunction, HeaderMapOpt, Resolvable, combine_headers, load_api_key, normalize_headers,
    with_user_agent_suffix,
};

/// A host part (a Vertex location, an Azure resource name) is one DNS label: letters, digits and hyphens, not starting or
/// ending with a hyphen (`isValidHostnamePart`).
pub(crate) fn is_valid_hostname_part(part: &str) -> bool {
    let bytes = part.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 63
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        && bytes[0] != b'-'
        && bytes[bytes.len() - 1] != b'-'
}

/// The base URL of a native package: the explicit setting, else the
/// environment variable `var` read when the provider is created (the AI SDK's
/// `loadOptionalSetting`), else `default`.
///
/// An invalid explicit value fails the factory (`Err` from the outer
/// result). An invalid environment value does not: the AI SDK does not
/// validate it when the provider is created and the first request fails, so
/// the inner `Err` is kept and reported by every request of that provider.
/// Creating a default provider therefore never fails on the environment.
pub(crate) fn base_url_setting(
    explicit: Option<&str>,
    var: &str,
    default: &str,
    normalize: impl Fn(&str) -> Result<String, AiMuxError>,
) -> Result<Result<String, AiMuxError>, AiMuxError> {
    if let Some(url) = explicit {
        return normalize(url).map(Ok);
    }
    Ok(
        match aimux_provider_utils::load_optional_setting(None, var) {
            Some(url) => normalize(&url)
                .map_err(|error| AiMuxError::InvalidArgument(format!("{var}: {error}"))),
            None => Ok(default.to_string()),
        },
    )
}

/// A provider-level rewrite of every JSON request body, called once after the
/// body is serialized and before it is sent.
pub type TransformRequestBody = Arc<dyn Fn(Value) -> Value + Send + Sync>;

/// Where the credential of a provider comes from.
#[derive(Clone, Debug)]
pub(crate) enum Credential {
    /// No credential: no `Authorization` header is produced and no key is
    /// resolved (a local server, or `auth: none` in a preset).
    None,
    /// The caller's key. It is used as given, `""` included, and never falls
    /// back to the environment. A [`Resolvable::Future`] is awaited once, an
    /// `AsyncFn` on every request.
    Explicit(Resolvable<String>),
    /// Read the environment variable on every request; unset fails that
    /// request with `AiMuxError::LoadApiKey`.
    Env { var: String, description: String },
    /// Read the environment variable on every request; unset or empty sends
    /// no credential (a local runtime started with or without a key).
    OptionalEnv(String),
}

impl Credential {
    /// `Explicit` when the caller gave a key, otherwise `Env`.
    pub(crate) fn explicit_or_env(
        api_key: Option<Resolvable<String>>,
        var: &str,
        description: &str,
    ) -> Self {
        match api_key {
            Some(key) => Self::Explicit(key),
            None => Self::Env {
                var: var.to_string(),
                description: description.to_string(),
            },
        }
    }

    /// The secret of this request, `None` for [`Credential::None`]. A
    /// package that puts the key somewhere other than a header (a query
    /// parameter, a JWT claim) asks for it here, per request.
    pub(crate) async fn secret(&self) -> Result<Option<String>, AiMuxError> {
        match self {
            Self::None => Ok(None),
            Self::Explicit(key) => Ok(Some(key.resolve().await?)),
            Self::Env { var, description } => Ok(Some(load_api_key(None, var, description)?)),
            Self::OptionalEnv(var) => Ok(std::env::var(var).ok().filter(|key| !key.is_empty())),
        }
    }
}

/// How the credential is put on the wire.
#[derive(Clone, Copy, Debug)]
pub(crate) enum AuthScheme {
    /// `Authorization: Bearer <secret>`.
    Bearer,
    /// `<name>: <secret>` (`x-api-key`).
    Header(&'static str),
    /// `Authorization: <scheme> <secret>` (`Token <key>`, `Key <key>`).
    Scheme(&'static str),
}

/// The provider-level request headers, as data. They are produced on every
/// request: the credential first, then the fixed headers (organization,
/// project, version, ...), then the user's headers, which may override or
/// remove any of them (case-insensitively, `None` removes), and finally the
/// `ai-sdk-{package}/{version}` user-agent suffix when one is set.
#[derive(Clone, Debug)]
pub(crate) struct ProviderHeaders {
    pub credential: Credential,
    pub scheme: AuthScheme,
    pub fixed: Vec<(String, String)>,
    pub user: Option<HeaderMapOpt>,
    /// `(package, version)` of the user-agent suffix.
    pub user_agent: Option<(&'static str, &'static str)>,
}

impl ProviderHeaders {
    /// Headers with `credential` put on the wire by `scheme`, no user-agent
    /// suffix.
    pub(crate) fn new(
        credential: Credential,
        scheme: AuthScheme,
        fixed: Vec<(String, String)>,
        user: Option<HeaderMapOpt>,
    ) -> Self {
        Self {
            credential,
            scheme,
            fixed,
            user,
            user_agent: None,
        }
    }

    /// Headers with a bearer credential, no user-agent suffix.
    pub(crate) fn bearer(
        credential: Credential,
        fixed: Vec<(String, String)>,
        user: Option<HeaderMapOpt>,
    ) -> Self {
        Self::new(credential, AuthScheme::Bearer, fixed, user)
    }

    /// The same headers with the `ai-sdk-{package}/{version}` user-agent
    /// suffix.
    #[must_use]
    pub(crate) fn with_user_agent(mut self, package: &'static str, version: &'static str) -> Self {
        self.user_agent = Some((package, version));
        self
    }

    pub(crate) async fn resolve(&self) -> Result<HeaderMapOpt, AiMuxError> {
        let mut layer = HeaderMapOpt::new();
        if let Some(secret) = self.credential.secret().await? {
            let (name, value) = match self.scheme {
                AuthScheme::Bearer => ("Authorization", format!("Bearer {secret}")),
                AuthScheme::Header(name) => (name, secret),
                AuthScheme::Scheme(scheme) => ("Authorization", format!("{scheme} {secret}")),
            };
            layer.insert(name.to_string(), Some(value));
        }
        for (name, value) in &self.fixed {
            layer.insert(name.clone(), Some(value.clone()));
        }
        let headers = match &self.user {
            Some(user) => combine_headers(&[&layer, user]),
            None => layer,
        };
        let Some((package, version)) = self.user_agent else {
            return Ok(headers);
        };
        let mut headers = normalize_headers(headers).into_iter().collect();
        with_user_agent_suffix(&mut headers, &format!("ai-sdk-{package}/{version}"));
        Ok(headers
            .into_iter()
            .map(|(name, value)| (name, Some(value)))
            .collect())
    }
}

/// The transport of a pinned package: `fetch` (the process default, resolved
/// per request, when `None`) behind a [`UserAgentFetch`] that names the
/// provider-utils `package` depends on in the `ai-sdk-provider-utils/<v>` part
/// the helper layer adds. Upstream that part comes from the provider-utils the
/// package itself imports (provider-utils post-to-api.ts:100-104,
/// get-from-api.ts:77-81), so it follows the package's `package.json`, not
/// the workspace pin.
pub(crate) fn package_fetch(
    fetch: Option<FetchFunction>,
    package: aisdk_pins::Package,
) -> FetchFunction {
    Arc::new(UserAgentFetch {
        inner: fetch,
        provider_utils: Some(package.provider_utils),
        suffix: None,
    })
}

/// A transport decorator that edits the request's `user-agent` just before
/// forwarding it, after the helper layer has added
/// `ai-sdk-provider-utils/<v>`: it names `provider_utils` in that part (when
/// set) and appends `suffix` (when set). The suffix is the AI SDK AWS
/// transports' own `withUserAgentSuffix` (amazon-bedrock-sigv4-fetch.ts:42-46
/// and :126-130, anthropic-aws-fetch.ts:42-46 and :126-130); the runtime
/// fragment they append is left out, as everywhere in aimux.
pub(crate) struct UserAgentFetch {
    /// `None` is the process default transport, resolved per request.
    pub inner: Option<FetchFunction>,
    pub provider_utils: Option<&'static str>,
    pub suffix: Option<String>,
}

#[async_trait::async_trait]
impl aimux_provider_utils::Fetch for UserAgentFetch {
    async fn fetch(
        &self,
        mut request: aimux_provider_utils::FetchRequest,
    ) -> Result<aimux_provider_utils::FetchResponse, aimux_provider_utils::FetchError> {
        transport_user_agent(
            &mut request.headers,
            self.provider_utils,
            self.suffix.as_deref(),
        );
        let inner = self
            .inner
            .clone()
            .unwrap_or_else(aimux_provider_utils::default_fetch);
        inner.fetch(request).await
    }
}

/// Rewrite the last `ai-sdk-provider-utils/<v>` part of the `user-agent` to
/// `provider_utils` (when set), then append `suffix` (when set, creating the
/// header if absent). A value that is not a valid header is left as it was.
pub(crate) fn transport_user_agent(
    headers: &mut reqwest::header::HeaderMap,
    provider_utils: Option<&str>,
    suffix: Option<&str>,
) {
    let current = headers
        .get(reqwest::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let mut parts: Vec<String> = current
        .split(' ')
        .filter(|part| !part.is_empty())
        .map(String::from)
        .collect();
    let last_utils = parts
        .iter_mut()
        .rev()
        .find(|part| part.starts_with("ai-sdk-provider-utils/"));
    match (provider_utils, last_utils) {
        (Some(version), Some(part)) => *part = format!("ai-sdk-provider-utils/{version}"),
        // Nothing to rename or append: the header stays exactly as it is.
        _ if suffix.is_none() => return,
        _ => {}
    }
    parts.extend(suffix.map(String::from));
    let user_agent = parts.join(" ");
    if user_agent == current {
        return;
    }
    if let Ok(value) = reqwest::header::HeaderValue::try_from(user_agent) {
        headers.insert(reqwest::header::USER_AGENT, value);
    }
}
