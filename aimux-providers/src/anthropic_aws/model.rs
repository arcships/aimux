//! Anthropic-AWS model wiring.
//!
//! Claude Platform on AWS serves the Anthropic Messages API unchanged, so its
//! model *is* the shared [`AnthropicMessagesModel`]. What is specific to the
//! host is only how a request is authenticated, and that is data on the
//! model's configuration: an `x-api-key` header, or a [`SigV4Fetch`] transport
//! decorator that signs the final method, URL, headers and body bytes just
//! before they are sent.

use std::sync::Arc;

use aimux_provider_utils::{
    FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, SigV4Fetch, default_fetch,
};

use crate::anthropic::AnthropicMessagesModel;
use crate::shared::{AuthScheme, Credential, credential_headers};

use super::AnthropicAwsAuth;

/// An Anthropic-AWS language model: the shared Messages model configured for
/// the AWS host.
pub type AnthropicAwsModel = AnthropicMessagesModel;

/// The SigV4 service name of Claude Platform on AWS.
const SIGV4_SERVICE: &str = "aws-external-anthropic";

const API_KEY_ENV_VAR: &str = "ANTHROPIC_AWS_API_KEY";
const API_VERSION: &str = "2023-06-01";

/// The provider headers and the transport that authenticate requests with
/// `auth`.
///
/// An API key is sent as `x-api-key` (`ANTHROPIC_AWS_API_KEY` when none was
/// given). SigV4 sends no credential header at all: the returned transport
/// wraps `fetch` (the process default when `None`) and signs every request it
/// forwards.
pub(super) fn authenticated(
    auth: Option<AnthropicAwsAuth>,
    workspace_id: Option<String>,
    user_headers: Option<HeaderMapOpt>,
    fetch: Option<FetchFunction>,
) -> (HeadersFn, Option<FetchFunction>) {
    let mut fixed = vec![("anthropic-version".to_string(), API_VERSION.to_string())];
    if let Some(workspace_id) = workspace_id {
        fixed.push(("anthropic-workspace-id".to_string(), workspace_id));
    }
    match auth {
        Some(AnthropicAwsAuth::SigV4(credentials)) => {
            let inner = fetch.unwrap_or_else(default_fetch);
            let signing: FetchFunction =
                Arc::new(SigV4Fetch::new(inner, credentials, SIGV4_SERVICE));
            (
                credential_headers(Credential::None, AuthScheme::Bearer, fixed, user_headers),
                Some(signing),
            )
        }
        key => {
            let key: Option<Resolvable<String>> = match key {
                Some(AnthropicAwsAuth::ApiKey(key)) => Some(key),
                _ => None,
            };
            (
                credential_headers(
                    Credential::explicit_or_env(key, API_KEY_ENV_VAR, "Anthropic AWS"),
                    AuthScheme::Header("x-api-key"),
                    fixed,
                    user_headers,
                ),
                fetch,
            )
        }
    }
}
