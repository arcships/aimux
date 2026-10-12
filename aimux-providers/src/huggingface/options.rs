//! The providerOptions and providerMetadata namespace of Hugging Face.
//!
//! `@ai-sdk/huggingface` reads `providerOptions.huggingface` and writes
//! response metadata under `huggingface`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions, provider_namespace};
use aimux_core::types::ProviderMetadata;
use serde_json::Value;

/// The providerOptions / providerMetadata key.
pub(crate) const NAMESPACE: &str = "huggingface";

/// The Hugging Face options in a providerOptions map.
pub(crate) fn huggingface_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}

/// `{ "huggingface": payload }`.
pub(crate) fn huggingface_metadata(payload: Value) -> ProviderMetadata {
    provider_namespace(NAMESPACE, payload).expect("metadata payload is an object")
}
