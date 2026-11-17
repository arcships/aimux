//! `cohere-embedding-model-options.ts`.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::optional_field;

/// `cohereEmbeddingModelOptions`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CohereEmbeddingProviderOptions {
    #[serde(default, deserialize_with = "optional_field")]
    pub(super) input_type: Option<InputType>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(super) truncate: Option<Truncate>,
    /// `z.union([z.literal(256), z.literal(512), z.literal(1024), z.literal(1536)])`.
    #[serde(default, deserialize_with = "output_dimension")]
    pub(super) output_dimension: Option<u32>,
}

/// `z.enum(['search_document', 'search_query', 'classification', 'clustering'])`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum InputType {
    SearchDocument,
    SearchQuery,
    Classification,
    Clustering,
}

impl InputType {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::SearchDocument => "search_document",
            Self::SearchQuery => "search_query",
            Self::Classification => "classification",
            Self::Clustering => "clustering",
        }
    }
}

/// `z.enum(['NONE', 'START', 'END'])`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub(super) enum Truncate {
    None,
    Start,
    End,
}

impl Truncate {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Start => "START",
            Self::End => "END",
        }
    }
}

fn output_dimension<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    let value = f64::deserialize(d)?;
    [256_u32, 512, 1024, 1536]
        .into_iter()
        .find(|allowed| f64::from(*allowed) == value)
        .map(Some)
        .ok_or_else(|| D::Error::custom("expected 256, 512, 1024 or 1536"))
}

/// The embedding options, parsed (all `None` without a `cohere` namespace).
///
/// # Errors
///
/// `InvalidArgument` "invalid cohere provider options" when the namespace
/// does not match the schema.
pub(super) fn parse_cohere_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Result<CohereEmbeddingProviderOptions, AiMuxError> {
    Ok(super::chat_language_model_options::parse(options)?.unwrap_or_default())
}
