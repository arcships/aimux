//! The providerOptions namespace of Deepgram.
//!
//! Deepgram reads `providerOptions.deepgram`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "deepgram";

/// The Deepgram options in a providerOptions map.
pub(crate) fn deepgram_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
