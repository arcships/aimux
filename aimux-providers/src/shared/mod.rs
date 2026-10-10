//! Helpers the native packages and the presets share.
//!
//! Only transport plumbing lives here: where the base URL and the
//! provider-level request headers of a request come from, as data. The few
//! vendors whose endpoint is decided by more than a URL and a credential
//! (Vertex, Bedrock, Polly) keep that logic in their own modules.

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

use aimux_core::AiMuxError;
use aimux_provider_utils::{
    HeaderMapOpt, HeadersFn, Resolvable, combine_headers, load_api_key, normalize_headers,
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

/// Provider headers with a bearer credential: [`credential_headers`] with
/// [`AuthScheme::Bearer`].
pub(crate) fn provider_headers(
    credential: Credential,
    fixed: Vec<(String, String)>,
    user: Option<HeaderMapOpt>,
) -> HeadersFn {
    credential_headers(credential, AuthScheme::Bearer, fixed, user)
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

    /// These headers as a [`HeadersFn`], for the configs that still take
    /// one; it resolves them on every request.
    pub(crate) fn into_headers_fn(self) -> HeadersFn {
        Resolvable::from_async_fn(move || {
            let headers = self.clone();
            async move { headers.resolve().await }
        })
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

/// [`ProviderHeaders`] without a user-agent suffix, as a [`HeadersFn`] for
/// the packages that still take one.
pub(crate) fn credential_headers(
    credential: Credential,
    scheme: AuthScheme,
    fixed: Vec<(String, String)>,
    user: Option<HeaderMapOpt>,
) -> HeadersFn {
    ProviderHeaders::new(credential, scheme, fixed, user).into_headers_fn()
}
