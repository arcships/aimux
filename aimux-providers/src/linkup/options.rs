//! The providerOptions namespace of Linkup.
//!
//! Linkup reads `providerOptions.linkup`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "linkup";

/// The Linkup options in a providerOptions map.
pub(crate) fn linkup_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
