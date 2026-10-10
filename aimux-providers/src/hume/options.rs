//! The providerOptions namespace of Hume.
//!
//! Hume reads `providerOptions.hume`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "hume";

/// The Hume options in a providerOptions map.
pub(crate) fn hume_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
