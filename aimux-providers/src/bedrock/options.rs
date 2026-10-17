//! The providerOptions and providerMetadata namespaces of Amazon Bedrock.
//!
//! Options use `amazonBedrock`, with `bedrock` as the upstream fallback alias.
//! Response metadata uses the canonical `amazonBedrock` key.

use aimux_core::shared::{JsonObject, SharedProviderOptions, provider_namespace};
use aimux_core::types::ProviderMetadata;
use serde_json::Value;

/// The namespace: read and written.
pub(crate) const AMAZON_BEDROCK: &str = "amazonBedrock";

/// The Bedrock options in a providerOptions container (`amazonBedrock`).
pub(crate) fn read(provider_options: Option<&SharedProviderOptions>) -> Option<&JsonObject> {
    let options = provider_options?;
    options
        .get(AMAZON_BEDROCK)
        .or_else(|| options.get("bedrock"))
}

/// Wrap `payload` as response metadata under the namespace key.
pub(crate) fn metadata(payload: Value) -> ProviderMetadata {
    provider_namespace(AMAZON_BEDROCK, payload).expect("provider metadata must be an object")
}
