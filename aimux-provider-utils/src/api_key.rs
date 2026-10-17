//! API key and setting loading (from a parameter or an environment variable).

use aimux_core::AiMuxError;

/// Load an API key from the given value or environment variable.
///
/// A supplied value is returned exactly as given — including the empty
/// string. An explicit key never falls back to the environment: a caller that
/// deliberately passes `""` (a keyless local server) must not have a
/// different credential substituted from the process environment. Only
/// `None` reads `environment_variable_name`.
///
/// # Errors
///
/// Returns `AiMuxError::LoadApiKey` when `api_key` is `None` and the
/// environment variable is unset.
pub fn load_api_key(
    api_key: Option<&str>,
    environment_variable_name: &str,
    description: &str,
) -> Result<String, AiMuxError> {
    if let Some(key) = api_key {
        return Ok(key.to_string());
    }

    std::env::var(environment_variable_name).map_err(|_| AiMuxError::LoadApiKey {
        env_var: environment_variable_name.to_string(),
        description: description.to_string(),
    })
}

/// Load a required setting from the given value or environment variable,
/// with the same no-fallback rule for an explicit value as [`load_api_key`].
///
/// # Errors
///
/// Returns `AiMuxError::LoadSetting` when `value` is `None` and the
/// environment variable is unset.
pub fn load_setting(
    value: Option<&str>,
    environment_variable_name: &str,
    setting_name: &str,
) -> Result<String, AiMuxError> {
    load_optional_setting(value, environment_variable_name).ok_or_else(|| AiMuxError::LoadSetting {
        env_var: environment_variable_name.to_string(),
        name: setting_name.to_string(),
    })
}

/// Load an optional setting from the given value or environment variable.
/// An explicit value (even `""`) wins; `None` reads the environment; an unset
/// variable yields `None`.
#[must_use]
pub fn load_optional_setting(
    value: Option<&str>,
    environment_variable_name: &str,
) -> Option<String> {
    match value {
        Some(value) => Some(value.to_string()),
        None => std::env::var(environment_variable_name).ok(),
    }
}
