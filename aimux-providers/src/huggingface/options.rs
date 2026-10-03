//! The providerOptions and providerMetadata namespace of Hugging Face.
//!
//! `@ai-sdk/huggingface` reads `providerOptions.huggingface` and writes
//! response metadata under `huggingface`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde_json::{Value, json};

/// The providerOptions / providerMetadata key.
pub(crate) const NAMESPACE: &str = "huggingface";

/// The Hugging Face options in a providerOptions map.
pub(crate) fn huggingface_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}

/// `{ "huggingface": payload }`.
pub(crate) fn huggingface_metadata(payload: Value) -> Value {
    json!({ NAMESPACE: payload })
}
