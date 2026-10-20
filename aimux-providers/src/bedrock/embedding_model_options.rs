//! Settings and provider options of the Amazon Bedrock embedding model
//! (amazon-bedrock-embedding-model-options.ts).

use super::options;
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

/// The embedding families supported by the upstream provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmazonBedrockEmbeddingModelFamily {
    Titan,
    Cohere,
    Nova,
}

/// Optional embedding family override for opaque inference-profile IDs.
#[derive(Clone, Debug, Default)]
pub struct AmazonBedrockEmbeddingModelSettings {
    pub model_family: Option<AmazonBedrockEmbeddingModelFamily>,
}

pub(super) struct BedrockEmbeddingProviderOptions {
    pub(super) dimensions: Option<u32>,
    pub(super) normalize: Option<bool>,
    pub(super) embedding_dimension: Option<u32>,
    pub(super) embedding_purpose: Option<String>,
    pub(super) input_type: Option<String>,
    pub(super) truncate: Option<String>,
    pub(super) output_dimension: Option<u32>,
}

/// Parse Bedrock embedding provider options.
///
/// Reads the `amazonBedrock` key only.
pub(super) fn parse_bedrock_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Result<BedrockEmbeddingProviderOptions, AiMuxError> {
    let provider_opts = options::read(options);
    if let Some(opts) = provider_opts {
        let valid_dimensions = [
            ("dimensions", &[1024, 512, 256][..]),
            ("embeddingDimension", &[256, 384, 1024, 3072][..]),
            ("outputDimension", &[256, 512, 1024, 1536][..]),
        ]
        .iter()
        .all(|(key, allowed)| {
            opts.get(*key).is_none_or(|v| {
                v.as_f64()
                    .is_some_and(|n| allowed.iter().any(|allowed| n == f64::from(*allowed)))
            })
        });
        let valid_enums = [
            (
                "embeddingPurpose",
                &[
                    "GENERIC_INDEX",
                    "TEXT_RETRIEVAL",
                    "IMAGE_RETRIEVAL",
                    "VIDEO_RETRIEVAL",
                    "DOCUMENT_RETRIEVAL",
                    "AUDIO_RETRIEVAL",
                    "GENERIC_RETRIEVAL",
                    "CLASSIFICATION",
                    "CLUSTERING",
                ][..],
            ),
            (
                "inputType",
                &[
                    "search_document",
                    "search_query",
                    "classification",
                    "clustering",
                ][..],
            ),
            ("truncate", &["NONE", "START", "END"][..]),
        ]
        .iter()
        .all(|(key, allowed)| {
            opts.get(*key)
                .is_none_or(|v| v.as_str().is_some_and(|s| allowed.contains(&s)))
        });
        if !valid_dimensions
            || !valid_enums
            || opts.get("normalize").is_some_and(|v| !v.is_boolean())
        {
            return Err(AiMuxError::InvalidArgument(
                "invalid amazonBedrock provider options".into(),
            ));
        }
    }

    Ok(BedrockEmbeddingProviderOptions {
        dimensions: provider_opts
            .and_then(|o| o.get("dimensions"))
            .and_then(serde_json::Value::as_f64)
            .map(|d| d as u32),
        normalize: provider_opts
            .and_then(|o| o.get("normalize"))
            .and_then(serde_json::Value::as_bool),
        embedding_dimension: provider_opts
            .and_then(|o| o.get("embeddingDimension"))
            .and_then(serde_json::Value::as_f64)
            .map(|d| d as u32),
        embedding_purpose: provider_opts
            .and_then(|o| o.get("embeddingPurpose"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        input_type: provider_opts
            .and_then(|o| o.get("inputType"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        truncate: provider_opts
            .and_then(|o| o.get("truncate"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        output_dimension: provider_opts
            .and_then(|o| o.get("outputDimension"))
            .and_then(serde_json::Value::as_f64)
            .map(|d| d as u32),
    })
}
