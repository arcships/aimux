//! `mistral-transcription-model-options.ts`.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use aimux_core::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::optional_field;

use super::chat_language_model_options::{NonEmptyString, parse};

/// `mistralTranscriptionModelOptions`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TranscriptionOptions {
    #[serde(default, deserialize_with = "optional_field")]
    language: Option<NonEmptyString>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) temperature: Option<f64>,
    /// `z.array(z.enum(['segment', 'word'])).min(1)`.
    #[serde(default, deserialize_with = "granularities")]
    timestamp_granularities: Option<Vec<Granularity>>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) diarize: Option<bool>,
    /// `z.array(z.string().min(1).regex(/^[^\s,]+$/)).max(100)`.
    #[serde(default, deserialize_with = "context_bias")]
    context_bias: Option<Vec<String>>,
}

impl TranscriptionOptions {
    pub(crate) fn language(&self) -> Option<&str> {
        self.language.as_ref().map(|language| language.0.as_str())
    }

    pub(crate) fn timestamp_granularities(&self) -> Option<Vec<&'static str>> {
        self.timestamp_granularities
            .as_ref()
            .map(|items| items.iter().map(|item| item.as_str()).collect())
    }

    pub(crate) fn context_bias(&self) -> Option<Vec<&str>> {
        self.context_bias
            .as_ref()
            .map(|items| items.iter().map(String::as_str).collect())
    }
}

/// `z.enum(['segment', 'word'])`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Granularity {
    Segment,
    Word,
}

impl Granularity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Segment => "segment",
            Self::Word => "word",
        }
    }
}

fn granularities<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<Granularity>>, D::Error> {
    let items = Vec::<Granularity>::deserialize(deserializer)?;
    if items.is_empty() {
        return Err(D::Error::custom(
            "too small: expected array to have >=1 items",
        ));
    }
    Ok(Some(items))
}

fn context_bias<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error> {
    let items = Vec::<NonEmptyString>::deserialize(deserializer)?;
    if items.len() > 100 {
        return Err(D::Error::custom(
            "too big: expected array to have <=100 items",
        ));
    }
    if items
        .iter()
        .any(|item| item.0.contains(|c: char| c == ',' || c.is_whitespace()))
    {
        return Err(D::Error::custom(
            "Context bias items must not contain commas or whitespace.",
        ));
    }
    Ok(Some(items.into_iter().map(|item| item.0).collect()))
}

/// The transcription options, parsed (all `None` without a `mistral`
/// namespace).
///
/// # Errors
///
/// `InvalidArgument` "invalid mistral provider options" when the namespace
/// does not match the schema.
pub(crate) fn transcription_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<TranscriptionOptions, AiMuxError> {
    Ok(parse(provider_options)?.unwrap_or_default())
}
