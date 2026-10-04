//! The providerOptions namespace of Google PSE.
//!
//! Google PSE reads `providerOptions.google_pse`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "google_pse";

/// The Google PSE options in a providerOptions map.
pub(crate) fn google_pse_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
