//! `mistral-speech-model-options.ts`.

use aimux_core::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

use super::chat_language_model_options::{field, mistral_options, non_empty_str};

/// `refAudio` of `mistralSpeechModelOptions`.
pub(crate) fn speech_ref_audio(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<Option<&str>, AiMuxError> {
    match mistral_options(provider_options) {
        Some(options) => Ok(field(options, "refAudio", non_empty_str)?),
        None => Ok(None),
    }
}
