//! The providerOptions namespace of AssemblyAI.
//!
//! AssemblyAI reads `providerOptions.assemblyai`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "assemblyai";

/// The AssemblyAI options in a providerOptions map.
pub(crate) fn assemblyai_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
