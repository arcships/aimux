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

/// `parseProviderOptions` of `amazonBedrock`, then of the legacy `bedrock`
/// key when the first is absent (`... ?? parseProviderOptions({ provider:
/// 'bedrock' })`): the same namespace [`read`] returns.
///
/// # Errors
///
/// `InvalidArgument` "invalid amazonBedrock provider options" (or
/// "invalid bedrock provider options") when the namespace does not match
/// the schema, with the serde error as its cause.
pub(crate) fn parse<T: serde::de::DeserializeOwned>(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<Option<T>, aimux_core::AiMuxError> {
    match aimux_provider_utils::parse_provider_options(AMAZON_BEDROCK, provider_options)? {
        Some(options) => Ok(Some(options)),
        None => aimux_provider_utils::parse_provider_options("bedrock", provider_options),
    }
}

/// Wrap `payload` as response metadata under the namespace key.
pub(crate) fn metadata(payload: Value) -> ProviderMetadata {
    let mut metadata =
        provider_namespace(AMAZON_BEDROCK, payload.clone()).expect("metadata payload is an object");
    metadata.extend(provider_namespace("bedrock", payload).expect("metadata payload is an object"));
    metadata
}
