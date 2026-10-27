use serde_json::{Value, json};

use crate::google::speech_input::google_speech_input;

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

// ── Older ports (previously the `getGoogleSpeechInput` section of `tests/google_speech_test.rs`) ──

mod older_speech_input {
    use serde_json::{Value, json};

    use crate::google::speech_input::google_speech_input;

    mod get_google_speech_input {
        use super::*;

        /// upstream: @ai-sdk/google src/google-speech-input.test.ts › getGoogleSpeechInput › extracts only transcript text, preserving Unicode and inline tags
        #[test]
        fn input_extracts_only_transcript_text() {
            let input = google_speech_input(
                "Ignored top-level text",
                None,
                Some(&json!({"google": {"turns": [
                    {"text": "Hello <sigh>", "speechMetadata": {"speaker": "Alice", "style": "whispering"}},
                    {"text": "世界 👋", "speechMetadata": {"speaker": "Bob"}},
                ]}})),
            );
            assert_eq!(input.text, "Hello <sigh>世界 👋");
            assert!(!input.uses_custom_voice);
        }

        /// upstream: @ai-sdk/google src/google-speech-input.test.ts › getGoogleSpeechInput › preserves an empty structured transcript instead of using top-level text
        #[test]
        fn input_preserves_an_empty_structured_transcript() {
            let input = google_speech_input(
                "Ignored",
                None,
                Some(&json!({"google": {"turns": [{"text": ""}]}})),
            );
            assert_eq!(input.text, "");
        }

        /// upstream: @ai-sdk/google src/google-speech-input.test.ts › getGoogleSpeechInput › leaves malformed options to provider validation: %j
        #[test]
        fn input_leaves_malformed_options_to_provider_validation() {
            let cases = [
                None,
                Some(json!({})),
                Some(json!({"google": null})),
                Some(json!({"google": "invalid"})),
                Some(json!({"google": {"turns": []}})),
                Some(json!({"google": {"turns": "Hello"}})),
                Some(json!({"google": {"turns": [null]}})),
                Some(json!({"google": {"turns": [{"text": 123}]}})),
                Some(json!({"google": {"turns": [{"text": "Partial"}, {}]}})),
                Some(json!({"openai": {"turns": [{"text": "Other provider"}]}})),
            ];
            for options in cases {
                let input = google_speech_input("Hello", None, options.as_ref());
                assert_eq!(input.text, "Hello", "{options:?}");
                assert!(!input.uses_custom_voice, "{options:?}");
            }
        }

        /// upstream: @ai-sdk/google src/google-speech-input.test.ts › getGoogleSpeechInput › identifies a top-level custom voice: %s
        #[test]
        fn input_identifies_a_top_level_custom_voice() {
            for voice in ["voice_test", "voicekey_test"] {
                assert!(google_speech_input("Hello", Some(voice), None).uses_custom_voice);
            }
        }

        /// upstream: @ai-sdk/google src/google-speech-input.test.ts › getGoogleSpeechInput › identifies explicit custom voice fields regardless of their value: %j
        #[test]
        fn input_identifies_explicit_custom_voice_fields_regardless_of_value() {
            for voice in [
                json!("voice_test"),
                json!("voicekey_test"),
                json!("unprefixed-id"),
                json!(""),
                Value::Null,
            ] {
                let options = json!({"google": {"multiSpeakerVoiceConfig": {"speakerVoiceConfigs": [
                    {"speaker": "Alice", "voiceConfig": {"voice": voice}},
                ]}}});
                assert!(google_speech_input("Hello", None, Some(&options)).uses_custom_voice);
            }
        }

        /// upstream: @ai-sdk/google src/google-speech-input.test.ts › getGoogleSpeechInput › recognizes custom voices even when multi-speaker configuration overrides voice
        #[test]
        fn input_recognizes_custom_voices_when_multi_speaker_overrides_voice() {
            let options = json!({"google": {"multiSpeakerVoiceConfig": {"speakerVoiceConfigs": [
                {"speaker": "Alice", "voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Kore"}}},
            ]}}});
            assert!(
                google_speech_input("Hello", Some("voice_test"), Some(&options)).uses_custom_voice
            );
        }
    }

    /// partial: `does not infer custom voices from unrelated fields: %j`; full port in src/google/tests/speech_input.rs.
    #[test]
    fn input_does_not_infer_custom_voices_from_unrelated_fields() {
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
            let options = json!({"google": {"multiSpeakerVoiceConfig": config}});
            assert!(
                !google_speech_input("Hello", Some("Kore"), Some(&options)).uses_custom_voice,
                "{config:?}"
            );
        }
    }
}
