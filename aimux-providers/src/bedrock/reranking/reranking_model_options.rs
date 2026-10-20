//! Provider options of the Bedrock reranking model (amazon-bedrock-reranking-model-options.ts).

use super::super::options;
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use serde_json::Value;

/// Bedrock provider-specific reranking options.
#[derive(Debug, Clone, Default)]
pub(super) struct BedrockRerankingOptions {
    pub(super) next_token: Option<String>,
    pub(super) additional_model_request_fields: Option<Value>,
}

pub(super) fn parse_bedrock_reranking_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<BedrockRerankingOptions, AiMuxError> {
    let mut opts = BedrockRerankingOptions::default();
    if let Some(po) = provider_options {
        let bedrock = options::read(Some(po));
        if let Some(bedrock) = bedrock {
            if bedrock.get("nextToken").is_some_and(|v| !v.is_string())
                || bedrock
                    .get("additionalModelRequestFields")
                    .is_some_and(|v| !v.is_object())
            {
                return Err(AiMuxError::InvalidArgument(
                    "invalid amazonBedrock provider options".into(),
                ));
            }
            if let Some(token) = bedrock.get("nextToken").and_then(|v| v.as_str()) {
                opts.next_token = Some(token.to_string());
            }
            if let Some(fields) = bedrock.get("additionalModelRequestFields") {
                opts.additional_model_request_fields = Some(fields.clone());
            }
        }
    }
    Ok(opts)
}
