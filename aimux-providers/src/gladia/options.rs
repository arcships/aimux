//! The providerOptions namespace of Gladia.
//!
//! Gladia reads `providerOptions.gladia`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "gladia";

/// The Gladia options in a providerOptions map.
pub(crate) fn gladia_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
