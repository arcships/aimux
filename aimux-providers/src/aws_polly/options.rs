//! The providerOptions namespace of Amazon Polly.
//!
//! Amazon Polly reads `providerOptions.aws_polly`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "aws_polly";

/// The Amazon Polly options in a providerOptions map.
pub(crate) fn aws_polly_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
