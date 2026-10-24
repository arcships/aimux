//! `google-speech-model-options.ts`.

use serde::de::{Deserialize, Deserializer, Error as _};
use serde::{Deserialize as DeriveDeserialize, Serialize};
use serde_json::Value;

use aimux_core::error::AiMuxError;

use super::options::no_null;

/// `z.never().optional()`: the key must not be present.
fn never<'de, D: Deserializer<'de>>(_: D) -> Result<Option<()>, D::Error> {
    Err(D::Error::custom("Custom voices are not supported"))
}

/// `z.string().min(1).optional()`.
fn non_empty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    let value = String::deserialize(deserializer)?;
    if value.is_empty() {
        return Err(D::Error::custom("speaker must not be empty"));
    }
    Ok(Some(value))
}

#[derive(Clone, Default, Serialize, DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SpeechMetadata {
    #[serde(
        default,
        deserialize_with = "non_empty",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) speaker: Option<String>,
    #[serde(
        default,
        deserialize_with = "no_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) style: Option<String>,
}

#[derive(DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Turn {
    pub(super) text: String,
    #[serde(default, deserialize_with = "no_null")]
    pub(super) speech_metadata: Option<SpeechMetadata>,
}

#[derive(Clone, Serialize, DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PrebuiltVoiceConfig {
    pub(super) voice_name: String,
}

#[derive(Clone, Serialize, DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct VoiceConfig {
    pub(super) prebuilt_voice_config: PrebuiltVoiceConfig,
    #[serde(
        default,
        rename = "voice",
        deserialize_with = "never",
        skip_serializing
    )]
    pub(super) _voice: Option<()>,
}

#[derive(Clone, Serialize, DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SpeakerVoiceConfig {
    pub(super) speaker: String,
    pub(super) voice_config: VoiceConfig,
}

#[derive(Clone, Serialize, DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MultiSpeakerVoiceConfig {
    pub(super) speaker_voice_configs: Vec<SpeakerVoiceConfig>,
}

#[derive(DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SpeechOptions {
    #[serde(default, deserialize_with = "no_null")]
    pub(super) speech_metadata: Option<SpeechMetadata>,
    #[serde(default, deserialize_with = "no_null")]
    pub(super) turns: Option<Vec<Turn>>,
    #[serde(default, deserialize_with = "no_null")]
    pub(super) multi_speaker_voice_config: Option<MultiSpeakerVoiceConfig>,
}

pub(super) fn parse_options(
    raw: Option<&aimux_core::shared::JsonObject>,
) -> Result<SpeechOptions, AiMuxError> {
    let invalid = |error: String| {
        AiMuxError::InvalidArgument(format!(
            "Invalid argument for parameter providerOptions: {error}"
        ))
    };
    let Some(raw) = raw else {
        return Ok(SpeechOptions {
            speech_metadata: None,
            turns: None,
            multi_speaker_voice_config: None,
        });
    };
    let options: SpeechOptions =
        serde_json::from_value(Value::Object(raw.clone())).map_err(|e| invalid(e.to_string()))?;
    if options.turns.as_ref().is_some_and(Vec::is_empty) {
        return Err(invalid("turns must contain at least one turn".into()));
    }
    Ok(options)
}
