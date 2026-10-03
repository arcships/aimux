//! The providerOptions namespace of Gladia.
//!
//! Gladia reads `providerOptions.gladia`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "gladia";

/// The Gladia options in a providerOptions map.
pub(crate) fn gladia_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
