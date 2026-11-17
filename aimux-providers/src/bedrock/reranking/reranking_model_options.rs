//! Provider options of the Bedrock reranking model (amazon-bedrock-reranking-model-options.ts).

use serde::Deserialize;
use serde_json::{Map, Value};

use super::super::options;
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::optional_field;

/// `amazonBedrockRerankingModelOptionsSchema`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BedrockRerankingOptions {
    #[serde(default, deserialize_with = "optional_field")]
    pub(super) next_token: Option<String>,
    /// `z.record(z.string(), z.any())`.
    #[serde(default, deserialize_with = "optional_field")]
    additional_model_request_fields: Option<Map<String, Value>>,
}

impl BedrockRerankingOptions {
    pub(super) fn additional_model_request_fields(&self) -> Option<Value> {
        self.additional_model_request_fields
            .clone()
            .map(Value::Object)
    }
}

/// Parse Bedrock reranking provider options: `amazonBedrock`, else the
/// legacy `bedrock` key (amazon-bedrock-reranking-model.ts:48-58).
///
/// # Errors
///
/// `InvalidArgument` "invalid amazonBedrock provider options" (or `bedrock`)
/// when the namespace does not match the schema.
pub(super) fn parse_bedrock_reranking_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<BedrockRerankingOptions, AiMuxError> {
    Ok(options::parse(provider_options)?.unwrap_or_default())
}
