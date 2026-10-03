//! The providerOptions namespace of Rev.ai.
//!
//! Rev.ai reads `providerOptions.revai`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "revai";

/// The Rev.ai options in a providerOptions map.
pub(crate) fn revai_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
