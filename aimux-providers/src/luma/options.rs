//! The providerOptions namespace of Luma.
//!
//! Luma reads `providerOptions.luma`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "luma";

/// The Luma options in a providerOptions map.
pub(crate) fn luma_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
