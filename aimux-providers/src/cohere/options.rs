//! The providerOptions and providerMetadata namespace of Cohere.
//!
//! `@ai-sdk/cohere` reads `providerOptions.cohere` and writes response
//! metadata under `cohere`, whatever the provider is named. This module is
//! the one place that spells the key.

use serde_json::{Value, json};

/// The providerOptions / providerMetadata key.
pub(crate) const NAMESPACE: &str = "cohere";

/// The Cohere options in a providerOptions map.
pub(crate) fn cohere_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}

/// `{ "cohere": payload }`.
pub(crate) fn cohere_metadata(payload: Value) -> Value {
    json!({ NAMESPACE: payload })
}
