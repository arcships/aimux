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
