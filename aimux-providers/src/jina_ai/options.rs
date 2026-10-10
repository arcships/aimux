//! The providerOptions namespace of Jina AI.
//!
//! Jina AI reads `providerOptions.jina`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "jina";

/// The Jina AI options in a providerOptions map.
pub(crate) fn jina_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
