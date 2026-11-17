//! `reranking/cohere-reranking-model-options.ts`.

use serde::Deserialize;

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::optional_field;

/// `cohereRerankingModelOptionsSchema`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CohereRerankingOptions {
    #[serde(default, deserialize_with = "optional_field")]
    pub(super) max_tokens_per_doc: Option<serde_json::Number>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(super) priority: Option<serde_json::Number>,
}

/// The reranking options, parsed (all `None` without a `cohere` namespace).
///
/// # Errors
///
/// `InvalidArgument` "invalid cohere provider options" when the namespace
/// does not match the schema.
pub(super) fn parse_cohere_reranking_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<CohereRerankingOptions, AiMuxError> {
    Ok(super::super::chat_language_model_options::parse(provider_options)?.unwrap_or_default())
}
