//! `google-speech-input.ts`.

use serde_json::Value;

use super::options::GOOGLE;

/// The transcript and voice kind of a speech request.
#[derive(Debug, PartialEq, Eq)]
pub struct GoogleSpeechInput {
    pub text: String,
    pub uses_custom_voice: bool,
}

/// Inspects speech input without replacing provider option validation. Also
/// used by intermediaries that need the transcript or voice kind before
/// invoking a model. `provider_options` is the whole providerOptions record
/// and is read leniently: anything malformed is left to provider validation.
#[must_use]
pub fn google_speech_input(
    text: &str,
    voice: Option<&str>,
    provider_options: Option<&Value>,
) -> GoogleSpeechInput {
    let options = provider_options
        .and_then(|options| options.get(GOOGLE))
        .filter(|google| google.is_object());
    let mut text = text.to_owned();
    if let Some(turns) = options
        .and_then(|options| options.get("turns"))
        .and_then(Value::as_array)
    {
        let texts: Vec<&str> = turns
            .iter()
            .map_while(|turn| turn.get("text").and_then(Value::as_str))
            .collect();
        if !turns.is_empty() && texts.len() == turns.len() {
            text = texts.concat();
        }
    }
    let speakers = options
        .and_then(|options| options.get("multiSpeakerVoiceConfig"))
        .and_then(|config| config.get("speakerVoiceConfigs"))
        .and_then(Value::as_array);
    let uses_custom_voice = voice
        .is_some_and(|voice| voice.starts_with("voice_") || voice.starts_with("voicekey_"))
        || speakers.is_some_and(|speakers| {
            speakers.iter().any(|speaker| {
                speaker
                    .get("voiceConfig")
                    .and_then(Value::as_object)
                    .is_some_and(|config| config.contains_key("voice"))
            })
        });
    GoogleSpeechInput {
        text,
        uses_custom_voice,
    }
}
