//! The providerOptions and providerMetadata namespace of xAI.
//!
//! `@ai-sdk/xai` reads `providerOptions.xai` and writes response metadata
//! under `xai`, whatever the provider is named. This module is the one place
//! that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions, provider_namespace};
use aimux_core::types::ProviderMetadata;
use serde_json::Value;

/// The providerOptions / providerMetadata key (and the provider-reference key
/// of uploaded files).
pub(crate) const NAMESPACE: &str = "xai";

/// The xAI options in a providerOptions container (a call's map, a part's
/// object).
pub(crate) fn xai_options(provider_options: Option<&SharedProviderOptions>) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}

/// `{ "xai": payload }`.
pub(crate) fn xai_metadata(payload: Value) -> ProviderMetadata {
    provider_namespace(NAMESPACE, payload).expect("metadata payload is an object")
}
