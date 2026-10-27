//! The Azure Speech options shared by the transcription options
//! (azure-speech-transcription-model-options.ts).

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Timestamps {
    Word,
    Segment,
    None,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum TranscribeStyle {
    Verbatim,
    Clean,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Diarization {
    enabled: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PhraseList {
    phrases: Vec<String>,
}
