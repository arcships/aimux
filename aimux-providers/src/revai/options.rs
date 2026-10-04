//! The providerOptions namespace of Rev.ai.
//!
//! Rev.ai reads `providerOptions.revai`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "revai";

/// The Rev.ai options in a providerOptions map.
pub(crate) fn revai_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
