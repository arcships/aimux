//! `get-realtime-base-url.ts`: the Live API WebSocket URL.

use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;

/// `getRealtimeWebSocketURL`: the Live API paths carry their own version, so a
/// trailing `/v1beta` or `/v1alpha` of the base URL is dropped.
#[cfg(feature = "realtime")]
pub(super) fn live_web_socket_url(base_url: &str, path: &str) -> Result<url::Url, AiMuxError> {
    let mut url = url::Url::parse(base_url).map_err(|error| AiMuxError::InvalidArgument {
        message: format!("invalid base URL: {error}"),
        cause: Some(ErrorCause::source(&error)),
    })?;
    let mut segments: Vec<&str> = url.path().split('/').collect();
    if matches!(segments.last(), Some(&("v1beta" | "v1alpha"))) {
        segments.pop();
    }
    let base_path = segments.join("/");
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_path(&format!("{}/ws/{path}", base_path.trim_end_matches('/')));
    url.set_scheme(scheme)
        .map_err(|()| AiMuxError::invalid_argument("invalid base URL scheme"))?;
    Ok(url)
}
