//! The providerOptions and providerMetadata namespace of Mistral.
//!
//! `@ai-sdk/mistral` reads `providerOptions.mistral` and writes response
//! metadata under `mistral`, whatever the provider is named. This module is
//! the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions, provider_namespace};
use aimux_core::types::ProviderMetadata;
use serde_json::Value;

/// The providerOptions / providerMetadata key.
pub(crate) const NAMESPACE: &str = "mistral";

/// The Mistral options in a providerOptions map.
pub(crate) fn mistral_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}

/// `{ "mistral": payload }`.
pub(crate) fn mistral_metadata(payload: Value) -> ProviderMetadata {
    provider_namespace(NAMESPACE, payload).expect("provider metadata must be an object")
}
