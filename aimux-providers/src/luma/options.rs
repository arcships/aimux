//! The providerOptions namespace of Luma.
//!
//! Luma reads `providerOptions.luma`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "luma";

/// The Luma options in a providerOptions map.
pub(crate) fn luma_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
