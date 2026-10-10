//! The providerOptions namespace of Cartesia.
//!
//! Cartesia reads `providerOptions.cartesia`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "cartesia";

/// The Cartesia options in a providerOptions map.
pub(crate) fn cartesia_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
