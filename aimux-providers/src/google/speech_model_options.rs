//! `google-speech-model-options.ts`.

use serde::de::{Deserialize, Deserializer, Error as _};
use serde::{Deserialize as DeriveDeserialize, Serialize};

use aimux_core::error::AiMuxError;

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
        deserialize_with = "aimux_provider_utils::optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) style: Option<String>,
}

#[derive(DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Turn {
    pub(super) text: String,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
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
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(super) speech_metadata: Option<SpeechMetadata>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(super) turns: Option<Vec<Turn>>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(super) multi_speaker_voice_config: Option<MultiSpeakerVoiceConfig>,
}

pub(super) fn parse_options(
    namespace: super::options::Namespace,
    provider_options: Option<&aimux_core::shared::SharedProviderOptions>,
) -> Result<SpeechOptions, AiMuxError> {
    let Some(options) = namespace.parse::<SpeechOptions>(provider_options)? else {
        return Ok(SpeechOptions {
            speech_metadata: None,
            turns: None,
            multi_speaker_voice_config: None,
        });
    };
    if options.turns.as_ref().is_some_and(Vec::is_empty) {
        return Err(AiMuxError::invalid_argument(
            "Invalid argument for parameter providerOptions: turns must contain at least one turn",
        ));
    }
    Ok(options)
}
