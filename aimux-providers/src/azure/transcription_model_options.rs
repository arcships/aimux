//! The `providerOptions.azure` of the transcription model
//! (azure-transcription-model-options.ts): the API to use plus the Speech
//! options.

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use serde::{Deserialize, Serialize};

use super::options::NAMESPACE;
use super::speech_transcription_model_options::{
    Diarization, PhraseList, Timestamps, TranscribeStyle,
};

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct AzureOptions {
    #[serde(
        default,
        deserialize_with = "aimux_provider_utils::optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) api: Option<Api>,
    #[serde(
        default,
        deserialize_with = "aimux_provider_utils::optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) timestamps: Option<Timestamps>,
    #[serde(
        default,
        deserialize_with = "aimux_provider_utils::optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) transcribe_style: Option<TranscribeStyle>,
    #[serde(
        default,
        deserialize_with = "aimux_provider_utils::optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) locales: Option<Vec<String>>,
    #[serde(
        default,
        deserialize_with = "aimux_provider_utils::optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) diarization: Option<Diarization>,
    #[serde(
        default,
        deserialize_with = "aimux_provider_utils::optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) phrase_list: Option<PhraseList>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Api {
    Openai,
    Speech,
}

pub(super) fn is_default_speech(model_id: &str) -> bool {
    model_id.eq_ignore_ascii_case("mai-transcribe-2")
}

pub(super) fn parse_options(
    options: Option<&SharedProviderOptions>,
) -> Result<AzureOptions, AiMuxError> {
    let Some(parsed): Option<AzureOptions> =
        aimux_provider_utils::parse_provider_options(NAMESPACE, options)?
    else {
        return Ok(AzureOptions::default());
    };
    if parsed
        .locales
        .as_ref()
        .is_some_and(|locales| locales.len() != 1)
    {
        return Err(AiMuxError::invalid_argument(
            "Azure transcription locales must contain one language",
        ));
    }
    Ok(parsed)
}
