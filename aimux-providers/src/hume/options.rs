//! The providerOptions namespace of Hume.
//!
//! Hume reads `providerOptions.hume`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "hume";

/// The Hume options in a providerOptions map.
pub(crate) fn hume_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
