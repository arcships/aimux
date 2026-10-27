use std::sync::Arc;

use base64::Engine;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_core::speech_model::{AudioData, SpeechCallOptions, SpeechModel, SpeechResult};

use crate::google::{GoogleProviderSettings, GoogleSpeechModel, create_google};
use crate::test_support::mock_fetch::{Canned, MockFetch};

/// The pinned user agent (upstream mocks `0.0.0-test`).
const PINNED_USER_AGENT: &str = "ai-sdk-google/4.0.87 ai-sdk-provider-utils/5.0.53";

// 8 bytes of raw PCM ([1..8]) base64-encoded.
const PCM_BASE64: &str = "AQIDBAUGBwg=";
const WAV_BASE64: &str = "UklGRiwAAABXQVZFZm10IBAAAAABAAEAwF0AAIC7AAACABAAZGF0YQgAAAABAgMEBQYHCA==";
const MODERN_MODELS: [&str; 3] = [
    "gemini-3.8-flash-tts",
    "gemini-3.8-flash-lite-tts",
    "custom-tts-model",
];

fn convert_base64_to_bytes(data: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap()
}

/// A response with one inline audio part (upstream `prepareJsonResponse`).
fn audio_response(mime_type: &str, data: &str) -> Canned {
    Canned::json(&json!({
        "candidates": [{"content": {"parts": [{"inlineData": {"mimeType": mime_type, "data": data}}]}}],
    }))
}

/// `createGoogle({ apiKey: 'test-api-key', fetch }).speech(modelId)`.
fn speech_model(mock: &Arc<MockFetch>, model_id: &str) -> GoogleSpeechModel {
    create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .speech(model_id)
}

fn google_options(value: Value) -> Option<SharedProviderOptions> {
    let Value::Object(map) = value else {
        panic!("options must be an object")
    };
    Some([("google".to_string(), map)].into_iter().collect())
}

fn audio(result: &SpeechResult) -> Vec<u8> {
    match &result.audio {
        AudioData::Binary(bytes) => bytes.clone(),
        AudioData::Base64(data) => convert_base64_to_bytes(data),
    }
}

/// `rejects.toMatchObject({ name: 'AI_InvalidArgumentError', argument })`:
/// aimux carries the argument in the message
/// (`Invalid argument for parameter <argument>: …`).
fn assert_invalid_argument(error: &AiMuxError, argument: &str) {
    let wire = serde_json::to_value(error).unwrap();
    assert_eq!(wire["name"], "AI_InvalidArgumentError", "{error:?}");
    let message = wire["message"].as_str().unwrap_or_default();
    assert!(
        message.starts_with(&format!("Invalid argument for parameter {argument}")),
        "{message}"
    );
}

fn multi_speaker_voice_config() -> Value {
    json!({"speakerVoiceConfigs": [
        {"speaker": "Joe", "voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Kore"}}},
        {"speaker": "Jane", "voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Puck"}}},
    ]})
}

mod do_generate {
    use super::*;

    /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › preserves legacy requests and PCM conversion for %s
    #[tokio::test]
    async fn preserves_legacy_requests_and_pcm_conversion_for_s() {
        for model_id in [
            "gemini-2.5-flash-preview-tts",
            "gemini-2.5-pro-preview-tts",
            "gemini-2.5-flash-tts",
            "gemini-3.1-flash-tts-preview",
            "gemini-3.1-flash-tts",
        ] {
            let mock = MockFetch::new(vec![audio_response("audio/L16;rate=24000", PCM_BASE64)]);
            let mut call = SpeechCallOptions::new("Hello.");
            call.instructions = Some("Whisper".into());
            call.output_format = Some("wav".into());
            let result = speech_model(&mock, model_id)
                .do_generate(&call)
                .await
                .unwrap();
            assert_eq!(
                mock.seen()[0].json_body(),
                json!({
                    "contents": [{"role": "user", "parts": [{"text": "Whisper: Hello."}]}],
                    "generationConfig": {
                        "responseModalities": ["AUDIO"],
                        "speechConfig": {
                            "voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Kore"}},
                        },
                    },
                }),
                "{model_id}"
            );
            assert_eq!(
                audio(&result),
                convert_base64_to_bytes(WAV_BASE64),
                "{model_id}"
            );
        }
    }

    /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let mock = MockFetch::new(vec![audio_response("audio/L16;rate=24000", PCM_BASE64)]);
        let provider = create_google(GoogleProviderSettings {
            api_key: Some("test-api-key".into()),
            headers: Some(
                [(
                    "Custom-Provider-Header".to_string(),
                    Some("provider-header-value".to_string()),
                )]
                .into_iter()
                .collect(),
            ),
            fetch: Some(mock.transport()),
            ..Default::default()
        })
        .unwrap();

        let mut call = SpeechCallOptions::new("Hello from the AI SDK!");
        call.headers = Some(
            [(
                "Custom-Request-Header".to_string(),
                "request-header-value".to_string(),
            )]
            .into_iter()
            .collect(),
        );
        provider
            .speech("gemini-2.5-flash-preview-tts")
            .do_generate(&call)
            .await
            .unwrap();

        let seen = &mock.seen()[0];
        for (name, value) in [
            ("content-type", "application/json"),
            ("x-goog-api-key", "test-api-key"),
            ("custom-provider-header", "provider-header-value"),
            ("custom-request-header", "request-header-value"),
        ] {
            assert_eq!(seen.headers.get(name).map(String::as_str), Some(value));
        }
        assert!(
            seen.headers["user-agent"].contains(PINNED_USER_AGENT),
            "{}",
            seen.headers["user-agent"]
        );
    }

    /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should use the real date when no custom date provider is specified
    #[tokio::test]
    async fn should_use_the_real_date_when_no_custom_date_provider_is_specified() {
        let mock = MockFetch::new(vec![audio_response("audio/L16;rate=24000", PCM_BASE64)]);
        let model = speech_model(&mock, "gemini-2.5-flash-preview-tts");

        let before_date = aimux_core::Timestamp::now();
        let result = model
            .do_generate(&SpeechCallOptions::new("Hello from the AI SDK!"))
            .await
            .unwrap();
        let after_date = aimux_core::Timestamp::now();

        let timestamp = result.response.timestamp.unwrap();
        assert!(timestamp.unix_millis() >= before_date.unix_millis());
        assert!(timestamp.unix_millis() <= after_date.unix_millis());
        assert_eq!(
            result.response.model_id.as_deref(),
            Some("gemini-2.5-flash-preview-tts")
        );
    }
}

mod modern_models {
    use super::*;

    /// upstream: @ai-sdk/google src/google-speech-model.test.ts › %s › rejects custom speaker voice configuration %j before fetching
    #[tokio::test]
    async fn rejects_custom_speaker_voice_configuration_j_before_fetching() {
        for model_id in MODERN_MODELS {
            for voice_config in [
                json!({"voice": "voice_custom"}),
                json!({"voice": "voicekey_custom"}),
                json!({"voice": "voice_custom", "prebuiltVoiceConfig": {"voiceName": "Kore"}}),
            ] {
                let mock = MockFetch::new(vec![]);
                let mut call = SpeechCallOptions::new("");
                call.provider_options = google_options(json!({
                    "turns": [{"text": "Hello", "speechMetadata": {"speaker": "Joe"}}],
                    "multiSpeakerVoiceConfig": {
                        "speakerVoiceConfigs": [{"speaker": "Joe", "voiceConfig": voice_config}],
                    },
                }));
                let error = speech_model(&mock, model_id)
                    .do_generate(&call)
                    .await
                    .unwrap_err();
                assert_invalid_argument(&error, "providerOptions");
                assert!(mock.seen().is_empty());
            }
        }
    }

    /// upstream: @ai-sdk/google src/google-speech-model.test.ts › %s › rejects missing or unconfigured speaker %s
    #[tokio::test]
    async fn rejects_missing_or_unconfigured_speaker_s() {
        for model_id in MODERN_MODELS {
            for speaker in [None, Some("Unknown")] {
                let mock = MockFetch::new(vec![]);
                let speech_metadata = match speaker {
                    None => json!({}),
                    Some(speaker) => json!({"speaker": speaker}),
                };
                let mut call = SpeechCallOptions::new("");
                call.provider_options = google_options(json!({
                    "multiSpeakerVoiceConfig": multi_speaker_voice_config(),
                    "turns": [{"text": "Hello", "speechMetadata": speech_metadata}],
                }));
                let error = speech_model(&mock, model_id)
                    .do_generate(&call)
                    .await
                    .unwrap_err();
                assert!(
                    error
                        .to_string()
                        .contains("Every multi-speaker turn must specify"),
                    "{model_id} {speaker:?}: {error}"
                );
                assert!(mock.seen().is_empty());
            }
        }
    }

    /// upstream: @ai-sdk/google src/google-speech-model.test.ts › %s › rejects a labelled transcript without structured speakers
    #[tokio::test]
    async fn rejects_a_labelled_transcript_without_structured_speakers() {
        for model_id in MODERN_MODELS {
            let mock = MockFetch::new(vec![]);
            let mut call = SpeechCallOptions::new("Joe: Hi. Jane: Hello.");
            call.provider_options =
                google_options(json!({"multiSpeakerVoiceConfig": multi_speaker_voice_config()}));
            let error = speech_model(&mock, model_id)
                .do_generate(&call)
                .await
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("Every multi-speaker turn must specify"),
                "{model_id}: {error}"
            );
            assert!(mock.seen().is_empty());
        }
    }

    /// upstream: @ai-sdk/google src/google-speech-model.test.ts › %s › rejects conflicting global and per-turn metadata
    #[tokio::test]
    async fn rejects_conflicting_global_and_per_turn_metadata() {
        for model_id in MODERN_MODELS {
            let mock = MockFetch::new(vec![]);
            let mut call = SpeechCallOptions::new("");
            call.provider_options = google_options(json!({
                "speechMetadata": {"style": "excited"},
                "turns": [{"text": "Hello"}],
            }));
            let error = speech_model(&mock, model_id)
                .do_generate(&call)
                .await
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("Set speechMetadata on each turn"),
                "{model_id}: {error}"
            );
        }
    }
}
