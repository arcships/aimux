//! Fetch error normalization.

use aimux_core::AiMuxError;

/// Attach request context to transport failures without changing other errors.
#[must_use]
pub fn handle_fetch_error(
    error: AiMuxError,
    url: &str,
    request_body_values: &serde_json::Value,
) -> AiMuxError {
    match error {
        AiMuxError::ApiCall(mut detail) => {
            detail.url = url.to_string();
            detail.request_body_values = request_body_values.clone();
            AiMuxError::ApiCall(detail)
        }
        AiMuxError::Aborted(_) | AiMuxError::Timeout(_) => error,
        other => other,
    }
}
