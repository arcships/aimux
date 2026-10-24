use serde_json::{Value, json};

use crate::google::speech::google_speech_input;

mod get_google_speech_input {
    use super::*;

    /// upstream: @ai-sdk/google src/google-speech-input.test.ts › getGoogleSpeechInput › does not infer custom voices from unrelated fields: %j
    #[test]
    fn does_not_infer_custom_voices_from_unrelated_fields_j() {
        // `undefined` leaves the key out; the other rows are the value itself.
        let configs = [
            None,
            Some(Value::Null),
            Some(json!("invalid")),
            Some(json!({"speakerVoiceConfigs": null})),
            Some(json!({"speakerVoiceConfigs": [null, {}, {"voiceConfig": null}]})),
            Some(json!({"speakerVoiceConfigs": [
                {"speaker": "Alice", "voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Kore"}}},
            ]})),
        ];
        for config in configs {
            let google = match &config {
                None => json!({}),
                Some(config) => json!({"multiSpeakerVoiceConfig": config}),
            };
            let provider_options = json!({"google": google});
            assert!(
                !google_speech_input("Hello", Some("Kore"), Some(&provider_options))
                    .uses_custom_voice,
                "{config:?}"
            );
        }
    }
}
