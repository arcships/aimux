//! The providerOptions namespace of Fal.
//!
//! Fal reads `providerOptions.fal`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "fal";

/// The Fal options in a providerOptions map.
pub(crate) fn fal_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
