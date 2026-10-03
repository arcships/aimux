//! The providerOptions and providerMetadata namespace of xAI.
//!
//! `@ai-sdk/xai` reads `providerOptions.xai` and writes response metadata
//! under `xai`, whatever the provider is named. This module is the one place
//! that spells the key.

use serde_json::{Value, json};

/// The providerOptions / providerMetadata key (and the provider-reference key
/// of uploaded files).
pub(crate) const NAMESPACE: &str = "xai";

/// The xAI options in a providerOptions container (a call's map, a part's
/// object).
pub(crate) fn xai_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}

/// `{ "xai": payload }`.
pub(crate) fn xai_metadata(payload: Value) -> Value {
    json!({ NAMESPACE: payload })
}
