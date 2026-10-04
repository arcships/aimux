//! The providerOptions and providerMetadata namespaces of Amazon Bedrock.
//!
//! `@ai-sdk/amazon-bedrock` takes its options from `providerOptions.amazonBedrock`
//! and writes response metadata under `amazonBedrock`. Only that canonical key
//! exists: the SDK's historical `bedrock` alias is not read and not written.
//! This module is the one place that knows the key, so the models never spell
//! it. (Claude-specific `anthropic` options do not
//! travel through the Converse API, so nothing here reads that key.)

use aimux_core::shared::{JsonObject, SharedProviderOptions, provider_namespace};
use aimux_core::types::ProviderMetadata;
use serde_json::Value;

/// The namespace: read and written.
pub(crate) const AMAZON_BEDROCK: &str = "amazonBedrock";

/// The Bedrock options in a providerOptions container (`amazonBedrock`).
pub(crate) fn read(provider_options: Option<&SharedProviderOptions>) -> Option<&JsonObject> {
    provider_options?.get(AMAZON_BEDROCK)
}

/// Wrap `payload` as response metadata under the namespace key.
pub(crate) fn metadata(payload: Value) -> ProviderMetadata {
    provider_namespace(AMAZON_BEDROCK, payload)
}
