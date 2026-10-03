//! The providerOptions namespace of Jina AI.
//!
//! Jina AI reads `providerOptions.jina`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "jina";

/// The Jina AI options in a providerOptions map.
pub(crate) fn jina_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
