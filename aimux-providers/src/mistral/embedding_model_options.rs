//! `mistral-embedding-model-options.ts`.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::optional_field;

use super::chat_language_model_options::parse;

/// `mistralEmbeddingModelOptions`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MistralEmbeddingProviderOptions {
    /// `z.record(z.string(), z.any())`.
    #[serde(default, deserialize_with = "optional_field")]
    pub(super) metadata: Option<Map<String, Value>>,
    /// `z.number().int().positive()`.
    #[serde(default, deserialize_with = "positive_int")]
    pub(super) output_dimension: Option<serde_json::Number>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(super) output_dtype: Option<OutputDtype>,
}

/// `z.enum(['float', 'int8', 'uint8', 'binary', 'ubinary'])`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum OutputDtype {
    Float,
    Int8,
    Uint8,
    Binary,
    Ubinary,
}

impl OutputDtype {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::Int8 => "int8",
            Self::Uint8 => "uint8",
            Self::Binary => "binary",
            Self::Ubinary => "ubinary",
        }
    }
}

/// zod's `.int().positive()`: a safe integer above zero (`2.0` passes, as
/// it is an integer in JavaScript).
fn positive_int<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<serde_json::Number>, D::Error> {
    let number = serde_json::Number::deserialize(deserializer)?;
    let value = number
        .as_f64()
        .ok_or_else(|| D::Error::custom("expected a number"))?;
    if value.fract() != 0.0 || value > 9_007_199_254_740_991.0 {
        return Err(D::Error::custom("expected int, received number"));
    }
    if value <= 0.0 {
        return Err(D::Error::custom("too small: expected number to be >0"));
    }
    Ok(Some(number))
}

/// The embedding options, parsed (all `None` without a `mistral` namespace).
///
/// # Errors
///
/// `InvalidArgument` "invalid mistral provider options" when the namespace
/// does not match the schema.
pub(super) fn parse_mistral_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Result<MistralEmbeddingProviderOptions, AiMuxError> {
    Ok(parse(options)?.unwrap_or_default())
}
