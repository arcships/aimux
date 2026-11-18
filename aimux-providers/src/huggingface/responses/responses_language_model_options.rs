//! The providerOptions and providerMetadata namespace of Hugging Face.
//!
//! `@ai-sdk/huggingface` reads `providerOptions.huggingface` and writes
//! response metadata under `huggingface`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{SharedProviderOptions, provider_namespace};
use aimux_core::types::ProviderMetadata;
use serde_json::Value;

/// The providerOptions / providerMetadata key.
pub(crate) const NAMESPACE: &str = "huggingface";

/// `huggingfaceLanguageModelResponsesOptions`.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HuggingFaceResponsesOptions {
    /// `z.record(z.string(), z.string())`, in the order given.
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(crate) metadata: Option<indexmap::IndexMap<String, String>>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(crate) instructions: Option<String>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(crate) strict_json_schema: Option<bool>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(crate) reasoning_effort: Option<String>,
}

/// `parseProviderOptions({ provider: 'huggingface', providerOptions, schema:
/// huggingfaceLanguageModelResponsesOptions })`
/// (huggingface-responses-language-model.ts:119-123); all `None` without a
/// `huggingface` namespace.
///
/// # Errors
///
/// `InvalidArgument` "invalid huggingface provider options" when the
/// namespace does not match the schema, with the serde error as its cause.
pub(crate) fn huggingface_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<HuggingFaceResponsesOptions, aimux_core::AiMuxError> {
    Ok(
        aimux_provider_utils::parse_provider_options(NAMESPACE, provider_options)?
            .unwrap_or_default(),
    )
}

/// `{ "huggingface": payload }`.
pub(crate) fn huggingface_metadata(payload: Value) -> ProviderMetadata {
    provider_namespace(NAMESPACE, payload).expect("metadata payload is an object")
}
