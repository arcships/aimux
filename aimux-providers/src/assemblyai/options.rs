//! The providerOptions namespace of AssemblyAI.
//!
//! AssemblyAI reads `providerOptions.assemblyai`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "assemblyai";

/// The AssemblyAI options in a providerOptions map.
pub(crate) fn assemblyai_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
