//! The providerOptions namespace of Prodia.
//!
//! Prodia reads `providerOptions.prodia`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "prodia";

/// The Prodia options in a providerOptions map.
pub(crate) fn prodia_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
