//! The providerOptions namespace of Google PSE.
//!
//! Google PSE reads `providerOptions.google_pse`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "google_pse";

/// The Google PSE options in a providerOptions map.
pub(crate) fn google_pse_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
