//! The providerOptions namespace of Replicate.
//!
//! Replicate reads `providerOptions.replicate`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "replicate";

/// The Replicate options in a providerOptions map.
pub(crate) fn replicate_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
