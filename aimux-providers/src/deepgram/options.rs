//! The providerOptions of Deepgram transcription
//! (`deepgram-transcription-model-options.ts`) and their namespace.
//!
//! Deepgram reads `providerOptions.deepgram`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "deepgram";

/// `deepgramTranscriptionModelOptionsSchema`
/// (deepgram-transcription-model-options.ts). Every key is `.nullish()`:
/// `null` reads as absent.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DeepgramOptions {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    detect_language: Option<bool>,
    #[serde(default)]
    smart_format: Option<bool>,
    #[serde(default)]
    punctuate: Option<bool>,
    #[serde(default)]
    paragraphs: Option<bool>,
    #[serde(default)]
    summarize: Option<Summarize>,
    #[serde(default)]
    topics: Option<bool>,
    #[serde(default)]
    intents: Option<bool>,
    #[serde(default)]
    sentiment: Option<bool>,
    #[serde(default)]
    detect_entities: Option<bool>,
    #[serde(default)]
    redact: Option<Redact>,
    #[serde(default)]
    replace: Option<String>,
    #[serde(default)]
    search: Option<String>,
    #[serde(default)]
    keyterm: Option<String>,
    #[serde(default)]
    diarize: Option<bool>,
    #[serde(default)]
    utterances: Option<bool>,
    #[serde(default)]
    utt_split: Option<f64>,
    #[serde(default)]
    filler_words: Option<bool>,
}

/// `summarize`: `z.union([z.literal('v2'), z.literal(false)])`.
#[derive(Debug, Clone, Copy)]
enum Summarize {
    V2,
    Off,
}

impl<'de> Deserialize<'de> for Summarize {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::String(value) if value == "v2" => Ok(Self::V2),
            Value::Bool(false) => Ok(Self::Off),
            _ => Err(serde::de::Error::custom("expected 'v2' or false")),
        }
    }
}

impl Summarize {
    /// `String(value)`.
    fn as_str(self) -> &'static str {
        match self {
            Self::V2 => "v2",
            Self::Off => "false",
        }
    }
}

/// `redact`: `z.union([z.string(), z.array(z.string())])`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Redact {
    One(String),
    Many(Vec<String>),
}

impl Redact {
    /// `String(value)`: an array joins its items with commas.
    fn to_query_value(&self) -> String {
        match self {
            Self::One(value) => value.clone(),
            Self::Many(values) => values.join(","),
        }
    }
}

/// `parseProviderOptions({ provider: 'deepgram', providerOptions, schema:
/// deepgramTranscriptionModelOptionsSchema })` (deepgram-transcription-model.ts
/// getArgs); all `None` without a `deepgram` namespace.
pub(super) fn parse_deepgram_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<DeepgramOptions, AiMuxError> {
    Ok(
        aimux_provider_utils::parse_provider_options(NAMESPACE, provider_options)?
            .unwrap_or_default(),
    )
}

/// The query parameters of a transcription, in the order upstream builds its
/// body (deepgram-transcription-model.ts getArgs): `model`, then each option
/// that is set, as `String(value)`.
pub(super) fn query_params(model_id: &str, dg: &DeepgramOptions) -> Vec<(&'static str, String)> {
    let flag = |value: Option<bool>| value.map(|value| value.to_string());
    [
        ("model", Some(model_id.to_string())),
        ("detect_entities", flag(dg.detect_entities)),
        ("detect_language", flag(dg.detect_language)),
        ("diarize", flag(dg.diarize)),
        ("filler_words", flag(dg.filler_words)),
        ("intents", flag(dg.intents)),
        ("keyterm", dg.keyterm.clone()),
        ("language", dg.language.clone()),
        ("paragraphs", flag(dg.paragraphs)),
        ("punctuate", flag(dg.punctuate)),
        ("redact", dg.redact.as_ref().map(Redact::to_query_value)),
        ("replace", dg.replace.clone()),
        ("search", dg.search.clone()),
        ("sentiment", flag(dg.sentiment)),
        ("smart_format", flag(dg.smart_format)),
        (
            "summarize",
            dg.summarize.map(|value| value.as_str().to_string()),
        ),
        ("topics", flag(dg.topics)),
        ("utterances", flag(dg.utterances)),
        ("utt_split", dg.utt_split.map(|value| value.to_string())),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|value| (key, value)))
    .collect()
}
