//! Helpers the OpenAI-shaped packages (`openai`, `openai_compatible`, the
//! presets) share.
//!
//! Only transport plumbing lives here: how the provider-level request headers
//! are produced. Nothing in this module knows a vendor.

use std::sync::Arc;

use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_provider_utils::{HeaderMapOpt, HeadersFn, Resolvable, combine_headers, load_api_key};

/// A provider-level rewrite of every JSON request body, called once after the
/// body is serialized and before it is sent.
pub type TransformRequestBody = Arc<dyn Fn(Value) -> Value + Send + Sync>;

/// Where the `Authorization: Bearer` credential of a provider comes from.
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

    async fn bearer(&self) -> Result<Option<String>, AiMuxError> {
        match self {
            Self::None => Ok(None),
            Self::Explicit(key) => Ok(Some(format!("Bearer {}", key.resolve().await?))),
            Self::Env { var, description } => Ok(Some(format!(
                "Bearer {}",
                load_api_key(None, var, description)?
            ))),
        }
    }
}

/// Provider headers, evaluated on every request: the credential first, then
/// the fixed headers (organization, project, ...), then the user's headers,
/// which may override or remove any of them (case-insensitively, `None`
/// removes).
pub(crate) fn provider_headers(
    credential: Credential,
    fixed: Vec<(String, String)>,
    user: Option<HeaderMapOpt>,
) -> HeadersFn {
    Resolvable::from_async_fn(move || {
        let credential = credential.clone();
        let fixed = fixed.clone();
        let user = user.clone();
        async move {
            let mut layer = HeaderMapOpt::new();
            if let Some(bearer) = credential.bearer().await? {
                layer.insert("Authorization".to_string(), Some(bearer));
            }
            for (name, value) in fixed {
                layer.insert(name, Some(value));
            }
            Ok(match &user {
                Some(user) => combine_headers(&[&layer, user]),
                None => layer,
            })
        }
    })
}
