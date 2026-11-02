//! The provider-level request headers: where the credential comes from, how
//! it goes on the wire, and the fixed and user headers around it. Upstream
//! each package's `getHeaders` builds these with `loadApiKey`,
//! `combineHeaders` and `withUserAgentSuffix`; here the providers describe
//! them as data and resolve them per request (RFC-0046 §3.2).

use aimux_core::AiMuxError;

use crate::{
    HeaderMapOpt, Resolvable, combine_headers, load_api_key, normalize_headers,
    with_user_agent_suffix,
};

/// Where the credential of a provider comes from.
#[derive(Clone, Debug)]
pub enum Credential {
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
    #[must_use]
    pub fn explicit_or_env(
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
    ///
    /// # Errors
    ///
    /// The error of an `Explicit` key that fails to resolve, or
    /// `AiMuxError::LoadApiKey` when the variable of `Env` is unset.
    pub async fn secret(&self) -> Result<Option<String>, AiMuxError> {
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
pub enum AuthScheme {
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
pub struct ProviderHeaders {
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
    #[must_use]
    pub fn new(
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
    #[must_use]
    pub fn bearer(
        credential: Credential,
        fixed: Vec<(String, String)>,
        user: Option<HeaderMapOpt>,
    ) -> Self {
        Self::new(credential, AuthScheme::Bearer, fixed, user)
    }

    /// The same headers with the `ai-sdk-{package}/{version}` user-agent
    /// suffix.
    #[must_use]
    pub fn with_user_agent(mut self, package: &'static str, version: &'static str) -> Self {
        self.user_agent = Some((package, version));
        self
    }

    /// The headers of one request, in the order the type describes.
    ///
    /// # Errors
    ///
    /// The credential's error, as [`Credential::secret`] returns it.
    pub async fn resolve(&self) -> Result<HeaderMapOpt, AiMuxError> {
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
