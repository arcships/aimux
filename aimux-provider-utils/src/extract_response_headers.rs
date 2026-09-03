//! Response header extraction for public error context.

use std::collections::HashMap;

/// Extract response headers as ordered pairs while redacting sensitive values.
/// Recording and public error context share this one policy.
#[must_use]
pub fn extract_response_header_pairs(
    headers: &reqwest::header::HeaderMap,
) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let value = if aimux_core::recording::is_sensitive_key(name.as_str()) {
                "[REDACTED]".to_string()
            } else {
                value.to_str().unwrap_or_default().to_string()
            };
            (name.as_str().to_ascii_lowercase(), value)
        })
        .collect()
}

/// Extract response headers while redacting sensitive values.
#[must_use]
pub fn extract_response_headers(headers: &reqwest::header::HeaderMap) -> HashMap<String, String> {
    extract_response_header_pairs(headers).into_iter().collect()
}
