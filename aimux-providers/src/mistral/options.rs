//! The providerOptions and providerMetadata namespace of Mistral.
//!
//! `@ai-sdk/mistral` reads `providerOptions.mistral` and writes response
//! metadata under `mistral`, whatever the provider is named. This module is
//! the one place that spells the key.

use serde_json::{Value, json};

/// The providerOptions / providerMetadata key.
pub(crate) const NAMESPACE: &str = "mistral";

/// The Mistral options in a providerOptions map.
pub(crate) fn mistral_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}

/// `{ "mistral": payload }`.
pub(crate) fn mistral_metadata(payload: Value) -> Value {
    json!({ NAMESPACE: payload })
}
