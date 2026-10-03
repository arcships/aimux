//! The providerOptions namespace of Voyage AI.
//!
//! Voyage reads `providerOptions.voyage`, whatever the provider is named.
//! This module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "voyage";

/// The Voyage options in a providerOptions map.
pub(crate) fn voyage_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
