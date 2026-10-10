//! The providerOptions namespace of Recraft.
//!
//! Recraft reads `providerOptions.recraft`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "recraft";

/// The Recraft options in a providerOptions map.
pub(crate) fn recraft_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
