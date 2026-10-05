//! The providerOptions namespace of Stability.
//!
//! Stability reads `providerOptions.stability`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "stability";

/// The Stability options in a providerOptions map.
pub(crate) fn stability_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
