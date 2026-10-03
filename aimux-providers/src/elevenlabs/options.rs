//! The providerOptions namespace of ElevenLabs.
//!
//! `@ai-sdk/elevenlabs` reads `providerOptions.elevenlabs`, whatever the
//! provider is named. This module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "elevenlabs";

/// The ElevenLabs options in a providerOptions map.
pub(crate) fn elevenlabs_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
