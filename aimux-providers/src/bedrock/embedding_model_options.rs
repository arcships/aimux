//! Settings and provider options of the Amazon Bedrock embedding model
//! (amazon-bedrock-embedding-model-options.ts).

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use super::options;
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::optional_field;

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

/// `amazonBedrockEmbeddingModelOptionsSchema`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmbeddingOptions {
    /// `z.union([z.literal(1024), z.literal(512), z.literal(256)])`.
    #[serde(default, deserialize_with = "dimensions")]
    dimensions: Option<u32>,
    #[serde(default, deserialize_with = "optional_field")]
    normalize: Option<bool>,
    /// `z.union([z.literal(256), z.literal(384), z.literal(1024), z.literal(3072)])`.
    #[serde(default, deserialize_with = "embedding_dimension")]
    embedding_dimension: Option<u32>,
    #[serde(default, deserialize_with = "embedding_purpose")]
    embedding_purpose: Option<String>,
    #[serde(default, deserialize_with = "input_type")]
    input_type: Option<String>,
    #[serde(default, deserialize_with = "truncate")]
    truncate: Option<String>,
    /// `z.union([z.literal(256), z.literal(512), z.literal(1024), z.literal(1536)])`.
    #[serde(default, deserialize_with = "output_dimension")]
    output_dimension: Option<u32>,
}

/// One of `allowed`, as zod's union of number literals reads it (`512.0`
/// is `512`).
fn literal<'de, D: Deserializer<'de>>(d: D, allowed: &[u32]) -> Result<Option<u32>, D::Error> {
    let value = f64::deserialize(d)?;
    allowed
        .iter()
        .copied()
        .find(|allowed| f64::from(*allowed) == value)
        .map(Some)
        .ok_or_else(|| D::Error::custom(format!("expected one of {allowed:?}")))
}

/// One of `allowed` (`z.enum`).
fn one_of<'de, D: Deserializer<'de>>(d: D, allowed: &[&str]) -> Result<Option<String>, D::Error> {
    let value = String::deserialize(d)?;
    if allowed.contains(&value.as_str()) {
        Ok(Some(value))
    } else {
        Err(D::Error::custom(format!("expected one of {allowed:?}")))
    }
}

fn dimensions<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    literal(d, &[1024, 512, 256])
}

fn embedding_dimension<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    literal(d, &[256, 384, 1024, 3072])
}

fn output_dimension<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    literal(d, &[256, 512, 1024, 1536])
}

fn embedding_purpose<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    one_of(
        d,
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
        ],
    )
}

fn input_type<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    one_of(
        d,
        &[
            "search_document",
            "search_query",
            "classification",
            "clustering",
        ],
    )
}

fn truncate<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    one_of(d, &["NONE", "START", "END"])
}

/// Parse Bedrock embedding provider options: `amazonBedrock`, else the
/// legacy `bedrock` key (amazon-bedrock-embedding-model.ts:88-99).
///
/// # Errors
///
/// `InvalidArgument` "invalid amazonBedrock provider options" (or `bedrock`)
/// when the namespace does not match the schema.
pub(super) fn parse_bedrock_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Result<BedrockEmbeddingProviderOptions, AiMuxError> {
    let parsed: EmbeddingOptions = options::parse(options)?.unwrap_or_default();
    Ok(BedrockEmbeddingProviderOptions {
        dimensions: parsed.dimensions,
        normalize: parsed.normalize,
        embedding_dimension: parsed.embedding_dimension,
        embedding_purpose: parsed.embedding_purpose,
        input_type: parsed.input_type,
        truncate: parsed.truncate,
        output_dimension: parsed.output_dimension,
    })
}
