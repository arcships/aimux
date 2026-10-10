//! Amazon Bedrock canonical and legacy option namespaces.

use aimux_core::shared::{JsonObject, SharedProviderOptions, provider_namespace};
use aimux_core::types::ProviderMetadata;
use serde_json::Value;

/// The namespace: read and written.
pub(crate) const AMAZON_BEDROCK: &str = "amazonBedrock";

/// Model family names used by Bedrock's family-specific request formats.
pub(crate) const COHERE_MODEL_FAMILY: &str = "cohere";
pub(crate) const OPENAI_MODEL_FAMILY: &str = "openai";

/// The Bedrock options in a providerOptions container (`amazonBedrock`).
pub(crate) fn read(provider_options: Option<&SharedProviderOptions>) -> Option<&JsonObject> {
    let options = provider_options?;
    options
        .get(AMAZON_BEDROCK)
        .or_else(|| options.get("bedrock"))
}

/// Wrap `payload` as response metadata under the namespace key.
pub(crate) fn metadata(payload: Value) -> ProviderMetadata {
    let mut metadata =
        provider_namespace(AMAZON_BEDROCK, payload.clone()).expect("metadata payload is an object");
    metadata.extend(provider_namespace("bedrock", payload).expect("metadata payload is an object"));
    metadata
}
