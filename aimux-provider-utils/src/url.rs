//! URL utilities.

/// Remove a trailing slash from a URL string.
#[must_use]
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
///
/// Parses the URL with the `url` crate and requires:
/// - scheme is `http` or `https`
/// - host is present (non-empty)
///
/// Userinfo (username/password) is permitted but not required — some
/// self-hosted endpoints use basic-auth credentials embedded in the URL.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when the URL cannot be parsed, the
/// scheme is not `http`/`https`, or the host is missing.
pub fn validate_base_url(url: &str) -> Result<String, aimux_core::AiMuxError> {
    if url.is_empty() {
        return Err(aimux_core::AiMuxError::InvalidArgument(
            "base URL cannot be empty".to_string(),
        ));
    }
    let parsed = url::Url::parse(url)
        .map_err(|e| aimux_core::AiMuxError::InvalidArgument(format!("invalid base URL: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(aimux_core::AiMuxError::InvalidArgument(format!(
            "base URL must be http or https, got: {}",
            parsed.scheme()
        )));
    }
    if parsed.host_str().map(str::is_empty).unwrap_or(true) {
        return Err(aimux_core::AiMuxError::InvalidArgument(
            "base URL must have a host".to_string(),
        ));
    }
    Ok(without_trailing_slash(url))
}
