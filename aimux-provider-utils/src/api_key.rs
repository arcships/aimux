//! API key loading (from parameter or environment variable).

use aimux_core::AiMuxError;

/// Load an API key from the given value or environment variable.
///
/// If `api_key` is `Some`, returns it directly.
/// Otherwise reads from `environment_variable_name`.
/// Returns `LoadAPIKeyError` if neither is available.
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
        AiMuxError::Auth(format!(
            "No API key found for {}. \
             Please provide it via the `api_key` parameter \
             or set the `{}` environment variable.",
            description, environment_variable_name
        ))
    })
}
