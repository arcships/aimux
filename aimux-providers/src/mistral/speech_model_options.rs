//! `mistral-speech-model-options.ts`.

use serde::Deserialize;

use aimux_core::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::optional_field;

use super::chat_language_model_options::{NonEmptyString, parse};

/// `mistralSpeechModelOptions`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpeechOptions {
    #[serde(default, deserialize_with = "optional_field")]
    ref_audio: Option<NonEmptyString>,
}

/// `refAudio` of `mistralSpeechModelOptions`.
///
/// # Errors
///
/// `InvalidArgument` "invalid mistral provider options" when the namespace
/// does not match the schema.
pub(crate) fn speech_ref_audio(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<Option<String>, AiMuxError> {
    Ok(parse::<SpeechOptions>(provider_options)?
        .and_then(|options| options.ref_audio)
        .map(|audio| audio.0))
}
