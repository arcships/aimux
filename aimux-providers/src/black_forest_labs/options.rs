//! The providerOptions namespace of Black Forest Labs.
//!
//! Black Forest Labs reads `providerOptions.blackForestLabs`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "blackForestLabs";

/// The Black Forest Labs options in a providerOptions map.
pub(crate) fn black_forest_labs_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
