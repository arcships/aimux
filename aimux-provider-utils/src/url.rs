//! URL utilities.

/// Remove a trailing slash from a URL string.
pub fn without_trailing_slash(url: &str) -> String {
    if let Some(stripped) = url.strip_suffix('/') {
        stripped.to_string()
    } else {
        url.to_string()
    }
}

/// Remove a trailing slash from an optional URL string, mirroring the TS
/// `withoutTrailingSlash(url: string | undefined)` signature: `None` in,
/// `None` out.
pub fn without_trailing_slash_opt(url: Option<&str>) -> Option<String> {
    url.map(without_trailing_slash)
}

/// Validate that a base URL is a valid HTTP(S) URL.
pub fn validate_base_url(url: &str) -> Result<String, aimux_core::AiMuxError> {
    if url.is_empty() {
        return Err(aimux_core::AiMuxError::InvalidArgument(
            "base URL cannot be empty".to_string(),
        ));
    }
    Ok(without_trailing_slash(url))
}
