//! The providerOptions namespace of Linkup.
//!
//! Linkup reads `providerOptions.linkup`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "linkup";

/// The Linkup options in a providerOptions map.
pub(crate) fn linkup_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
