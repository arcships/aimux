//! The providerOptions namespace of ElevenLabs.
//!
//! `@ai-sdk/elevenlabs` reads `providerOptions.elevenlabs`, whatever the
//! provider is named. This module is the one place that spells the key.

#[cfg(feature = "realtime")]
use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "elevenlabs";

/// The ElevenLabs options in a providerOptions map.
///
/// Only the realtime transcription path reads it.
#[cfg(feature = "realtime")]
pub(crate) fn elevenlabs_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
