//! API key loading (from parameter or environment variable).

use aimux_core::AiMuxError;

/// Load an API key from the given value or environment variable.
///
/// If `api_key` is `Some`, returns it directly.
/// Otherwise reads from `environment_variable_name`.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when neither the `api_key` value nor
/// the environment variable yields a key.
pub fn load_api_key(
    api_key: Option<&str>,
    environment_variable_name: &str,
    description: &str,
) -> Result<String, AiMuxError> {
    if let Some(key) = api_key
        && !key.is_empty()
    {
        return Ok(key.to_string());
    }

    std::env::var(environment_variable_name).map_err(|_| {
        AiMuxError::InvalidArgument(format!(
            "No API key found for {description}. \
             Please provide it via the `api_key` parameter \
             or set the `{environment_variable_name}` environment variable."
        ))
    })
}
