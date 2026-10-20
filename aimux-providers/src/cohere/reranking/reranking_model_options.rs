//! `reranking/cohere-reranking-model-options.ts`.

use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

/// Cohere provider-specific reranking options.
#[derive(Debug, Clone, Default)]
pub(super) struct CohereRerankingOptions {
    pub(super) max_tokens_per_doc: Option<serde_json::Number>,
    pub(super) priority: Option<serde_json::Number>,
}

pub(super) fn parse_cohere_reranking_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<CohereRerankingOptions, AiMuxError> {
    let cohere = super::super::chat_language_model_options::cohere_options(provider_options);
    let number_option = |key: &str| -> Result<Option<serde_json::Number>, AiMuxError> {
        cohere
            .and_then(|options| options.get(key))
            .map(|value| match value {
                Value::Number(number) => Ok(number.clone()),
                _ => Err(AiMuxError::InvalidArgument(format!("Invalid cohere.{key}"))),
            })
            .transpose()
    };
    Ok(CohereRerankingOptions {
        max_tokens_per_doc: number_option("maxTokensPerDoc")?,
        priority: number_option("priority")?,
    })
}
