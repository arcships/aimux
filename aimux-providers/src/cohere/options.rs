//! The providerOptions and providerMetadata namespace of Cohere.
//!
//! `@ai-sdk/cohere` reads `providerOptions.cohere` and writes response
//! metadata under `cohere`, whatever the provider is named. This module is
//! the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions, provider_namespace};
use aimux_core::types::ProviderMetadata;
use serde_json::Value;

/// The providerOptions / providerMetadata key.
pub(crate) const NAMESPACE: &str = "cohere";

/// The Cohere options in a providerOptions map.
pub(crate) fn cohere_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}

/// `{ "cohere": payload }`.
pub(crate) fn cohere_metadata(payload: Value) -> ProviderMetadata {
    provider_namespace(NAMESPACE, payload).expect("metadata payload is an object")
}
