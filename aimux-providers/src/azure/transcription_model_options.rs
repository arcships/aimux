//! The `providerOptions.azure` of the transcription model
//! (azure-transcription-model-options.ts): the API to use plus the Speech
//! options.

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::options::NAMESPACE;
use super::speech_transcription_model_options::{
    Diarization, PhraseList, Timestamps, TranscribeStyle,
};

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct AzureOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) api: Option<Api>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) timestamps: Option<Timestamps>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) transcribe_style: Option<TranscribeStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) locales: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) diarization: Option<Diarization>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
    let Some(azure) = options.and_then(|options| options.get(NAMESPACE)) else {
        return Ok(AzureOptions::default());
    };
    // Optional in the upstream schema means absent, never explicitly null.
    if azure.values().any(Value::is_null) {
        return Err(AiMuxError::InvalidArgument(
            "Invalid Azure transcription options: null field".into(),
        ));
    }
    let parsed: AzureOptions =
        serde_json::from_value(Value::Object(azure.clone())).map_err(|error| {
            AiMuxError::InvalidArgument(format!("Invalid Azure transcription options: {error}"))
        })?;
    if parsed
        .locales
        .as_ref()
        .is_some_and(|locales| locales.len() != 1)
    {
        return Err(AiMuxError::InvalidArgument(
            "Azure transcription locales must contain one language".into(),
        ));
    }
    Ok(parsed)
}
