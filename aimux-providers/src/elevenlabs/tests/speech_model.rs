//! Port of `@ai-sdk/elevenlabs@3.0.54` `src/elevenlabs-speech-model.test.ts`.
//!
//! Harness: upstream answers `https://api.elevenlabs.io/v1/text-to-speech/*`
//! with 100 bytes of `audio/<format>` from a test server; here a
//! [`MockFetch`] answers the same way and records the request. Upstream's
//! `toMatchObject` checks a subset of the body, so each case compares the
//! keys it names.
//!
//! `should include user-agent header` is not ported: the
//! `ai-sdk-elevenlabs/<version>` user-agent part is not sent on this branch
//! (layout.toml; #444 adds it with its own test).

use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::shared::Warning;
use aimux_core::speech_model::{SpeechCallOptions, SpeechModel, SpeechResult};
use aimux_provider_utils::Resolvable;

use crate::elevenlabs::{ElevenLabsProviderSettings, ElevenLabsSpeechModel, create_elevenlabs};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

/// `prepareAudioResponse()`: 100 bytes of `audio/mp3`.
fn audio_fetch() -> Arc<MockFetch> {
    MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".to_string(), "audio/mp3".to_string())],
        body: vec![0u8; 100],
    }])
}

/// `createElevenLabs({ apiKey: 'test-api-key' }).speech('eleven_multilingual_v2')`.
fn model(fetch: &Arc<MockFetch>) -> ElevenLabsSpeechModel {
    create_elevenlabs(ElevenLabsProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
    .speech("eleven_multilingual_v2")
}

/// `{ text, voice: 'test-voice-id' }`.
fn call(text: &str) -> SpeechCallOptions {
    let mut call = SpeechCallOptions::new(text);
    call.voice = Some("test-voice-id".to_string());
    call
}

async fn generate(fetch: &Arc<MockFetch>, call: &SpeechCallOptions) -> (SpeechResult, Seen) {
    let result = model(fetch).do_generate(call).await.unwrap();
    let seen = fetch.seen().remove(0);
    (result, seen)
}

/// `expect(server.calls[0].requestUrl).toContain('output_format=<format>')`.
fn assert_output_format(seen: &Seen, format: &str) {
    assert!(
        seen.url.contains(&format!("output_format={format}")),
        "{}",
        seen.url
    );
}

/// The `{ text, model_id }` part every case's `toMatchObject` names.
fn assert_text_and_model(body: &Value, text: &str) {
    assert_eq!(body["text"], text, "{body}");
    assert_eq!(body["model_id"], "eleven_multilingual_v2", "{body}");
}

mod do_generate {
    use super::*;

    /// upstream: @ai-sdk/elevenlabs src/elevenlabs-speech-model.test.ts › ElevenLabsSpeechModel › doGenerate › should generate speech with required parameters
    #[tokio::test]
    async fn should_generate_speech_with_required_parameters() {
        let fetch = audio_fetch();
        let (_, seen) = generate(&fetch, &call("Hello, world!")).await;

        assert_text_and_model(&seen.json_body(), "Hello, world!");
        assert_output_format(&seen, "mp3_44100_128");
    }

    /// upstream: @ai-sdk/elevenlabs src/elevenlabs-speech-model.test.ts › ElevenLabsSpeechModel › doGenerate › should handle custom output format
    #[tokio::test]
    async fn should_handle_custom_output_format() {
        let fetch = audio_fetch();
        let mut call = call("Hello, world!");
        call.output_format = Some("pcm_44100".to_string());
        let (_, seen) = generate(&fetch, &call).await;

        assert_text_and_model(&seen.json_body(), "Hello, world!");
        assert_output_format(&seen, "pcm_44100");
    }

    /// upstream: @ai-sdk/elevenlabs src/elevenlabs-speech-model.test.ts › ElevenLabsSpeechModel › doGenerate › should handle language parameter
    #[tokio::test]
    async fn should_handle_language_parameter() {
        let fetch = audio_fetch();
        let mut call = call("Hola, mundo!");
        call.language = Some("es".to_string());
        let (_, seen) = generate(&fetch, &call).await;

        let body = seen.json_body();
        assert_text_and_model(&body, "Hola, mundo!");
        assert_eq!(body["language_code"], "es", "{body}");
        assert_output_format(&seen, "mp3_44100_128");
    }

    /// upstream: @ai-sdk/elevenlabs src/elevenlabs-speech-model.test.ts › ElevenLabsSpeechModel › doGenerate › should handle speed parameter in voice settings
    #[tokio::test]
    async fn should_handle_speed_parameter_in_voice_settings() {
        let fetch = audio_fetch();
        let mut call = call("Hello, world!");
        call.speed = Some(1.5);
        let (_, seen) = generate(&fetch, &call).await;

        let body = seen.json_body();
        assert_text_and_model(&body, "Hello, world!");
        assert_eq!(body["voice_settings"], json!({ "speed": 1.5 }), "{body}");
    }

    /// upstream: @ai-sdk/elevenlabs src/elevenlabs-speech-model.test.ts › ElevenLabsSpeechModel › doGenerate › should warn about unsupported instructions parameter
    #[tokio::test]
    async fn should_warn_about_unsupported_instructions_parameter() {
        let fetch = audio_fetch();
        let mut call = call("Hello, world!");
        call.instructions = Some("Speak slowly".to_string());
        let (result, _) = generate(&fetch, &call).await;

        assert_eq!(
            result.warnings,
            vec![Warning::Unsupported {
                feature: "instructions".to_string(),
                details: Some(
                    "ElevenLabs speech models do not support instructions. Instructions parameter was ignored."
                        .to_string()
                ),
            }]
        );
    }

    /// upstream: @ai-sdk/elevenlabs src/elevenlabs-speech-model.test.ts › ElevenLabsSpeechModel › doGenerate › should pass provider-specific options
    #[tokio::test]
    async fn should_pass_provider_specific_options() {
        let fetch = audio_fetch();
        let mut call = call("Hello, world!");
        call.provider_options = Some(
            serde_json::from_value(json!({
                "elevenlabs": {
                    "voiceSettings": { "stability": 0.5, "similarityBoost": 0.75 },
                    "seed": 123,
                },
            }))
            .unwrap(),
        );
        let (_, seen) = generate(&fetch, &call).await;

        let body = seen.json_body();
        assert_text_and_model(&body, "Hello, world!");
        assert_eq!(
            body["voice_settings"],
            json!({ "stability": 0.5, "similarity_boost": 0.75 }),
            "{body}"
        );
        assert_eq!(body["seed"], 123, "{body}");
        assert_output_format(&seen, "mp3_44100_128");
    }
}
