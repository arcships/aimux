//! `parseProviderOptions` (parse-provider-options.ts): read one provider's
//! namespace of `providerOptions` against its options schema.

use serde::de::DeserializeOwned;
use serde_json::Value;

use aimux_core::shared::SharedProviderOptions;
use aimux_core::{AiMuxError, ErrorCause};

/// The `provider` namespace of `provider_options`, deserialized as `T`.
///
/// `T`'s serde shape is the options schema: unknown keys are dropped unless
/// it denies them, as a zod object strips them unless it is strict. No
/// namespace is `None`.
///
/// # Errors
///
/// `InvalidArgument` "invalid {provider} provider options" for the argument
/// `providerOptions` (aimux writes the argument as the message's
/// `Invalid argument for parameter providerOptions: ` prefix) when the
/// namespace does not match `T`, with the serde error as its cause.
pub fn parse_provider_options<T: DeserializeOwned>(
    provider: &str,
    provider_options: Option<&SharedProviderOptions>,
) -> Result<Option<T>, AiMuxError> {
    let Some(options) = provider_options.and_then(|options| options.get(provider)) else {
        return Ok(None);
    };
    serde_json::from_value(Value::Object(options.clone()))
        .map(Some)
        .map_err(|error| AiMuxError::InvalidArgument {
            message: format!(
                "Invalid argument for parameter providerOptions: invalid {provider} provider options"
            ),
            cause: Some(ErrorCause::source(&error)),
        })
}

/// `deserialize_with` for a field of a zod `.optional()` schema: an absent
/// field is `None` (with `#[serde(default)]`), an explicit `null` fails the
/// schema as a wrong type does.
///
/// # Errors
///
/// The deserializer's error for `null` or a value that does not match `T`.
pub fn optional_field<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// `deserialize_with` for a zod `z.number().positive().nullish()` field:
/// absent (with `#[serde(default)]`) or `null` is `None`, a number above 0
/// is kept.
///
/// # Errors
///
/// The deserializer's error for a value that is not a number or not above 0.
pub fn nullish_positive<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    use serde::de::Error as _;
    match Option::<f64>::deserialize(deserializer)? {
        Some(value) if value <= 0.0 => Err(D::Error::custom("expected a number > 0")),
        other => Ok(other),
    }
}
