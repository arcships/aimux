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
    BaseUrl, Endpoint, EndpointConfig, EndpointHeaders, HttpEndpoint, ResolveEndpoint,
    SupportedUrlsSource,
};
pub(crate) use poll::{
    POLL_INTERVAL_MILLIS_KEY, POLL_INTERVAL_MS_KEY, PollStep, is_poll_control_key,
    poll_interval_ms, poll_until, retry_download,
};

use std::sync::Arc;

use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_provider_utils::{FetchFunction, UserAgentFetch};

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
            Some(url) => normalize(&url).map_err(|error| AiMuxError::InvalidArgument {
                message: format!("{var}: {error}"),
                cause: Some(error.into()),
            }),
            None => Ok(default.to_string()),
        },
    )
}

/// A provider-level rewrite of every JSON request body, called once after the
/// body is serialized and before it is sent.
pub type TransformRequestBody = Arc<dyn Fn(Value) -> Value + Send + Sync>;

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
