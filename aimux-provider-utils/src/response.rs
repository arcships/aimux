//! Response handling helpers.

use aimux_core::AiMuxError;

/// Provider error structure (allows each provider to define its own error JSON shape).
pub struct ErrorStructure {
    /// JSON path to the error message (e.g. `["error", "message"]`).
    pub message_path: &'static [&'static str],
    /// JSON path to the error type (e.g. `["error", "type"]`).
    pub type_path: &'static [&'static str],
}

/// Default OpenAI-compatible error structure: `{ "error": { "message": "...", "type": "..." } }`.
pub const DEFAULT_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["error", "message"],
    type_path: &["error", "type"],
};

/// Parse an HTTP error response into an `AiMuxError`.
///
/// - 401 → `Auth`
/// - 429 → `RateLimited` (retry-after in ms, default 1000)
/// - 404 → `ModelNotFound`
/// - other (incl. 403 and 5xx) → `Provider`
///
/// 403 is intentionally *not* mapped to `Auth`: a 403 is a permission/quota
/// failure, which most providers surface as a provider error, not an
/// authentication failure (only 401 means "bad credentials"). 5xx likewise
/// surfaces here as `Provider`; the shared HTTP layer's retry loop still tags
/// 5xx as `ApiCall` internally for retry decisions — providers re-map that to
/// `Provider` via [`api_call_to_provider_error`].
pub fn parse_provider_error(status: u16, body: &str, structure: &ErrorStructure) -> AiMuxError {
    let mut message = format!("HTTP {}", status);
    let mut _error_type = String::new();

    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
        // Try to extract message.
        let mut current = &val;
        for key in structure.message_path {
            if let Some(v) = current.get(key) {
                current = v;
            } else {
                current = &val;
                break;
            }
        }
        if let Some(msg) = current.as_str() {
            message = msg.to_string();
        } else if !body.is_empty() {
            // The configured message path didn't land on a string value (e.g.
            // xAI's flat responses-API shape `{"code","error"}` where `error`
            // is itself a string, not an object). Surface the raw body so the
            // provider error still carries the upstream payload instead of a
            // bare "HTTP {status}" placeholder.
            message = body.to_string();
        }

        // Try to extract type.
        let mut current = &val;
        for key in structure.type_path {
            if let Some(v) = current.get(key) {
                current = v;
            } else {
                current = &val;
                break;
            }
        }
        if let Some(t) = current.as_str() {
            _error_type = t.to_string();
        }
    } else if !body.is_empty() {
        message = body.to_string();
    }

    match status {
        401 => AiMuxError::Auth(message),
        429 => AiMuxError::RateLimited {
            retry_after_ms: 1000,
        },
        404 => AiMuxError::ModelNotFound(message),
        _ => AiMuxError::Provider(format!("HTTP {}: {}", status, message)),
    }
}

/// Re-map a shared-HTTP-layer 5xx error (`ApiCall`) into a `Provider` error.
///
/// `send_with_retry_raw` returns 5xx responses as [`AiMuxError::ApiCall`] so
/// the retry/backoff machinery (and the `http_send_test` retry contract) keep
/// working. Providers whose contract surfaces 5xx as a *provider* error
/// (recraft, google, vertex) wrap the `send` / `send_stream` result with this
/// so callers see `Provider` rather than `ApiCall`.
///
/// Non-`ApiCall` errors (4xx via `parse_provider_error`, network `Http`,
/// `RateLimited`, …) pass through unchanged.
pub fn api_call_to_provider_error(error: AiMuxError) -> AiMuxError {
    match error {
        AiMuxError::ApiCall(message) => AiMuxError::Provider(message),
        other => other,
    }
}

/// Re-map an HTTP 403 `Provider` error back to `Auth`.
///
/// `parse_provider_error` maps 403 → `Provider` (a permission/quota failure is a
/// provider error for most providers). Amazon Polly, however, treats 403
/// (`AccessDeniedException`) as an authentication/authorization failure, so it
/// re-maps 403 back to `Auth` with this helper. Detection uses the `"HTTP 403: "`
/// prefix that `parse_provider_error`'s default arm stamps onto every
/// status-bearing provider message.
pub fn provider_403_to_auth(error: AiMuxError) -> AiMuxError {
    const PREFIX: &str = "HTTP 403: ";
    match error {
        AiMuxError::Provider(ref message) if message.starts_with(PREFIX) => {
            AiMuxError::Auth(message[PREFIX.len()..].to_string())
        }
        other => other,
    }
}
