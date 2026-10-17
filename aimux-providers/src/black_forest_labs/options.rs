//! The providerOptions namespace of Black Forest Labs.
//!
//! Black Forest Labs reads `providerOptions.blackForestLabs`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "blackForestLabs";

/// The Black Forest Labs options in a providerOptions map.
pub(crate) fn black_forest_labs_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
