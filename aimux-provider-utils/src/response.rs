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
/// - other → `Provider`
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
