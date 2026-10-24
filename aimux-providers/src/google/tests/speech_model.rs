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

// ── Older ports (previously `tests/google_speech_test.rs`) ──
//
// Rust translation of the Google speech model tests.
//
// Sources: `reference/aisdk-pinned/google/src/google-speech-model.test.ts` and
// `reference/aisdk-pinned/google/src/google-speech-input.test.ts`.

mod older_speech {
    use std::sync::Arc;

    use serde_json::{Value, json};
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::{
        GoogleProviderSettings, VertexProviderSettings, create_google, create_google_vertex,
    };
    use aimux_core::error::AiMuxError;
    use aimux_core::provider::Provider;
    use aimux_core::shared::{SharedProviderOptions, Warning};
    use aimux_core::speech_model::{AudioData, SpeechCallOptions, SpeechModel, SpeechResult};
    use aimux_provider_utils::Resolvable;

    // 8 bytes of raw PCM ([1..8]) base64-encoded.
    const PCM_BASE64: &str = "AQIDBAUGBwg=";
    const PCM_BYTES: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
    const WAV_BASE64: &str =
        "UklGRiwAAABXQVZFZm10IBAAAAABAAEAwF0AAIC7AAACABAAZGF0YQgAAAABAgMEBQYHCA==";
    const LEGACY_MODEL: &str = "gemini-2.5-flash-preview-tts";
    const MODERN_MODELS: [&str; 3] = [
        "gemini-3.8-flash-tts",
        "gemini-3.8-flash-lite-tts",
        "custom-tts-model",
    ];

    fn wav_bytes() -> Vec<u8> {
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, WAV_BASE64).unwrap()
    }

    async fn mount(server: &MockServer, mime_type: &str, data: &str) {
        mount_parts(
            server,
            json!([{"inlineData": {"mimeType": mime_type, "data": data}}]),
        )
        .await;
    }

    async fn mount_parts(server: &MockServer, parts: Value) {
        Mock::given(method("POST"))
            .and(path_regex(r"^/models/[^/]+:generateContent$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-request-id", "test-request-id")
                    .set_body_json(json!({"candidates": [{"content": {"parts": parts}}]})),
            )
            .mount(server)
            .await;
    }

    fn model(server: &MockServer, model_id: &str) -> Arc<dyn SpeechModel> {
        create_google(GoogleProviderSettings {
            api_key: Some("test-api-key".to_string()),
            base_url: Some(server.uri()),
            ..Default::default()
        })
        .unwrap()
        .speech_model(model_id)
        .unwrap()
        .unwrap()
    }

    fn options(text: &str) -> SpeechCallOptions {
        SpeechCallOptions::new(text)
    }

    fn google_options(value: Value) -> Option<SharedProviderOptions> {
        provider_options("google", value)
    }

    fn provider_options(key: &str, value: Value) -> Option<SharedProviderOptions> {
        let Value::Object(map) = value else {
            panic!("options must be an object")
        };
        Some([(key.to_string(), map)].into_iter().collect())
    }

    async fn request_body(server: &MockServer, index: usize) -> Value {
        server.received_requests().await.unwrap()[index]
            .body_json()
            .unwrap()
    }

    fn audio(result: &SpeechResult) -> Vec<u8> {
        match &result.audio {
            AudioData::Binary(bytes) => bytes.clone(),
            AudioData::Base64(_) => panic!("expected binary audio"),
        }
    }

    fn has_warning(result: &SpeechResult, expected: &str) -> bool {
        result.warnings.iter().any(
        |warning| matches!(warning, Warning::Unsupported { feature, .. } if feature == expected),
    )
    }

    fn invalid_argument(error: AiMuxError) -> String {
        match error {
            AiMuxError::InvalidArgument(message) => message,
            other => panic!("expected InvalidArgument, got {other:?}"),
        }
    }

    fn multi_speaker() -> Value {
        json!({"speakerVoiceConfigs": [
            {"speaker": "Joe", "voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Kore"}}},
            {"speaker": "Jane", "voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Puck"}}},
        ]})
    }

    // ── doGenerate ──────────────────────────────────────────────────────────────

    mod do_generate {
        use super::*;

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › preserves raw usage including cached input tokens for %s
        #[tokio::test]
        async fn preserves_raw_response_body() {
            for model_id in [LEGACY_MODEL, "gemini-3.1-flash-tts-preview"]
                .into_iter()
                .chain(MODERN_MODELS)
            {
                let server = MockServer::start().await;
                let body = json!({
                    "candidates": [{"content": {"parts": [{"inlineData": {"data": WAV_BASE64, "mimeType": "audio/wav"}}]}}],
                    "usageMetadata": {
                        "promptTokenCount": 6, "cachedContentTokenCount": 5, "candidatesTokenCount": 67,
                        "totalTokenCount": 73,
                        "candidatesTokensDetails": [{"modality": "AUDIO", "tokenCount": 67}],
                    },
                });
                Mock::given(method("POST"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(body.clone()))
                    .mount(&server)
                    .await;
                let result = model(&server, model_id)
                    .do_generate(&options("Hello."))
                    .await
                    .unwrap();
                assert_eq!(result.response.body, Some(body.clone()), "{model_id}");
                // The provider's usage object, passed through unchanged
                // (`@ai-sdk/google@4.0.87 src/google-speech-model.ts:356-358`).
                assert_eq!(
                    result.usage.map(Value::Object),
                    Some(body["usageMetadata"].clone()),
                    "{model_id}"
                );
            }
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › rejects an empty transcript before fetching for %s
        #[tokio::test]
        async fn rejects_an_empty_transcript_before_fetching() {
            for model_id in [LEGACY_MODEL, "gemini-3.1-flash-tts-preview"]
                .into_iter()
                .chain(MODERN_MODELS)
            {
                let server = MockServer::start().await;
                let mut call = options("");
                call.instructions = Some("Whisper".into());
                let error = model(&server, model_id)
                    .do_generate(&call)
                    .await
                    .unwrap_err();
                assert!(
                    invalid_argument(error).contains("parameter text"),
                    "{model_id}"
                );
                assert!(server.received_requests().await.unwrap().is_empty());
            }
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › rejects empty turns even with top-level text for %s
        #[tokio::test]
        async fn rejects_empty_turns_even_with_top_level_text() {
            for model_id in MODERN_MODELS {
                let server = MockServer::start().await;
                let mut call = options("Ignored");
                call.provider_options = google_options(json!({"turns": [{"text": ""}]}));
                let error = model(&server, model_id)
                    .do_generate(&call)
                    .await
                    .unwrap_err();
                assert!(
                    invalid_argument(error).contains("parameter text"),
                    "{model_id}"
                );
                assert!(server.received_requests().await.unwrap().is_empty());
            }
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should send the text and the default voice
        #[tokio::test]
        async fn sends_the_text_and_the_default_voice() {
            let server = MockServer::start().await;
            mount(&server, "audio/L16;rate=24000", PCM_BASE64).await;
            model(&server, LEGACY_MODEL)
                .do_generate(&options("Hello from the AI SDK!"))
                .await
                .unwrap();
            assert_eq!(
                request_body(&server, 0).await,
                json!({
                    "contents": [{"role": "user", "parts": [{"text": "Hello from the AI SDK!"}]}],
                    "generationConfig": {
                        "responseModalities": ["AUDIO"],
                        "speechConfig": {"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Kore"}}},
                    },
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should use the provided voice
        #[tokio::test]
        async fn uses_the_provided_voice() {
            let server = MockServer::start().await;
            mount(&server, "audio/L16;rate=24000", PCM_BASE64).await;
            let mut call = options("Hello from the AI SDK!");
            call.voice = Some("Puck".into());
            model(&server, LEGACY_MODEL)
                .do_generate(&call)
                .await
                .unwrap();
            assert_eq!(
                request_body(&server, 0).await["generationConfig"]["speechConfig"],
                json!({"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Puck"}}})
            );
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should wrap PCM audio in a WAV container by default
        #[tokio::test]
        async fn wraps_pcm_audio_in_a_wav_container_by_default() {
            let server = MockServer::start().await;
            mount(&server, "audio/L16;rate=24000", PCM_BASE64).await;
            let result = model(&server, LEGACY_MODEL)
                .do_generate(&options("Hello from the AI SDK!"))
                .await
                .unwrap();
            let audio = audio(&result);
            // 44-byte WAV header + 8 bytes PCM payload.
            assert_eq!(audio.len(), 52);
            assert_eq!(&audio[..4], b"RIFF");
            assert_eq!(&audio[8..12], b"WAVE");
            assert_eq!(u32::from_le_bytes(audio[24..28].try_into().unwrap()), 24000);
            assert_eq!(u16::from_le_bytes(audio[22..24].try_into().unwrap()), 1);
            assert_eq!(u16::from_le_bytes(audio[34..36].try_into().unwrap()), 16);
            assert_eq!(&audio[44..], PCM_BYTES);
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should preserve WAV responses even from older models
        #[tokio::test]
        async fn preserves_wav_responses_from_older_models() {
            let server = MockServer::start().await;
            mount(&server, "audio/wav", WAV_BASE64).await;
            let result = model(&server, LEGACY_MODEL)
                .do_generate(&options("Hello"))
                .await
                .unwrap();
            assert_eq!(audio(&result), wav_bytes());
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should derive the WAV sample rate from the response mime type
        #[tokio::test]
        async fn derives_the_wav_sample_rate_from_the_mime_type() {
            let server = MockServer::start().await;
            mount(&server, "audio/L16;rate=16000", PCM_BASE64).await;
            let result = model(&server, LEGACY_MODEL)
                .do_generate(&options("Hello from the AI SDK!"))
                .await
                .unwrap();
            let audio = audio(&result);
            assert_eq!(u32::from_le_bytes(audio[24..28].try_into().unwrap()), 16000);
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should return raw PCM and warn for outputFormat "pcm"
        #[tokio::test]
        async fn returns_raw_pcm_and_warns_for_pcm_output_format() {
            let server = MockServer::start().await;
            mount(&server, "audio/L16;rate=24000", PCM_BASE64).await;
            let mut call = options("Hello from the AI SDK!");
            call.output_format = Some("pcm".into());
            let result = model(&server, LEGACY_MODEL)
                .do_generate(&call)
                .await
                .unwrap();
            assert_eq!(audio(&result), PCM_BYTES);
            assert!(has_warning(&result, "outputFormat"));
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should warn for unsupported speed and language options
        #[tokio::test]
        async fn warns_for_unsupported_speed_and_language() {
            let server = MockServer::start().await;
            mount(&server, "audio/L16;rate=24000", PCM_BASE64).await;
            let mut call = options("Hello from the AI SDK!");
            call.speed = Some(1.5);
            call.language = Some("en".into());
            let result = model(&server, LEGACY_MODEL)
                .do_generate(&call)
                .await
                .unwrap();
            assert!(has_warning(&result, "speed"));
            assert!(has_warning(&result, "language"));
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should prepend instructions to the prompt text
        #[tokio::test]
        async fn prepends_instructions_to_the_prompt_text() {
            let server = MockServer::start().await;
            mount(&server, "audio/L16;rate=24000", PCM_BASE64).await;
            let mut call = options("Hello there");
            call.instructions = Some("Say cheerfully".into());
            model(&server, LEGACY_MODEL)
                .do_generate(&call)
                .await
                .unwrap();
            assert_eq!(
                request_body(&server, 0).await["contents"],
                json!([{"role": "user", "parts": [{"text": "Say cheerfully: Hello there"}]}])
            );
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should map multi-speaker provider options into speechConfig
        #[tokio::test]
        async fn maps_multi_speaker_options_into_speech_config() {
            let server = MockServer::start().await;
            mount(&server, "audio/L16;rate=24000", PCM_BASE64).await;
            let mut call = options("Joe: Hi. Jane: Hello.");
            call.provider_options =
                google_options(json!({"multiSpeakerVoiceConfig": multi_speaker()}));
            model(&server, LEGACY_MODEL)
                .do_generate(&call)
                .await
                .unwrap();
            // Strict equality proves the single-voice `voiceConfig` is absent.
            assert_eq!(
                request_body(&server, 0).await,
                json!({
                    "contents": [{"role": "user", "parts": [{"text": "Joe: Hi. Jane: Hello."}]}],
                    "generationConfig": {
                        "responseModalities": ["AUDIO"],
                        "speechConfig": {"multiSpeakerVoiceConfig": multi_speaker()},
                    },
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should read provider options under `googleVertex` for a Vertex provider
        #[tokio::test]
        async fn reads_provider_options_under_google_vertex_for_a_vertex_provider() {
            let server = MockServer::start().await;
            mount(&server, "audio/L16;rate=24000", PCM_BASE64).await;
            let vertex = create_google_vertex(VertexProviderSettings {
                api_key: Some(Resolvable::Value("test-api-key".to_string())),
                base_url: Some(server.uri()),
                ..Default::default()
            })
            .unwrap();
            let model = vertex.speech_model(LEGACY_MODEL).unwrap().unwrap();
            assert_eq!(model.provider(), "google.vertex.speech");
            let mut call = options("Joe: Hi. Jane: Hello.");
            call.provider_options = provider_options(
                "googleVertex",
                json!({"multiSpeakerVoiceConfig": multi_speaker()}),
            );
            model.do_generate(&call).await.unwrap();
            assert_eq!(
                request_body(&server, 0).await,
                json!({
                    "contents": [{"role": "user", "parts": [{"text": "Joe: Hi. Jane: Hello."}]}],
                    "generationConfig": {
                        "responseModalities": ["AUDIO"],
                        "speechConfig": {"multiSpeakerVoiceConfig": multi_speaker()},
                    },
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should ignore instructions (with a warning) when multi-speaker is set
        #[tokio::test]
        async fn ignores_instructions_with_a_warning_when_multi_speaker_is_set() {
            let server = MockServer::start().await;
            mount(&server, "audio/L16;rate=24000", PCM_BASE64).await;
            let mut call = options("Joe: Hi. Jane: Hello.");
            call.instructions = Some("Say cheerfully".into());
            call.provider_options =
                google_options(json!({"multiSpeakerVoiceConfig": multi_speaker()}));
            let result = model(&server, LEGACY_MODEL)
                .do_generate(&call)
                .await
                .unwrap();
            // instructions are NOT prepended to the multi-speaker transcript.
            assert_eq!(
                request_body(&server, 0).await["contents"],
                json!([{"role": "user", "parts": [{"text": "Joe: Hi. Jane: Hello."}]}])
            );
            assert!(has_warning(&result, "instructions"));
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should expose sample rate and mime type in provider metadata
        #[tokio::test]
        async fn exposes_sample_rate_and_mime_type_in_provider_metadata() {
            let server = MockServer::start().await;
            mount(&server, "audio/L16;rate=24000", PCM_BASE64).await;
            let result = model(&server, LEGACY_MODEL)
                .do_generate(&options("Hello from the AI SDK!"))
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(result.provider_metadata.unwrap()).unwrap(),
                json!({"google": {"sampleRate": 24000, "mimeType": "audio/L16;rate=24000"}})
            );
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should return empty audio when no inline data is present
        #[tokio::test]
        async fn returns_empty_audio_when_no_inline_data_is_present() {
            let server = MockServer::start().await;
            mount_parts(&server, json!([{"text": "no audio here"}])).await;
            let result = model(&server, LEGACY_MODEL)
                .do_generate(&options("Hello from the AI SDK!"))
                .await
                .unwrap();
            assert!(audio(&result).is_empty());
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should include response data with timestamp, modelId and headers
        #[tokio::test]
        async fn includes_response_data_with_timestamp_model_id_and_headers() {
            let server = MockServer::start().await;
            mount(&server, "audio/L16;rate=24000", PCM_BASE64).await;
            let result = model(&server, LEGACY_MODEL)
                .do_generate(&options("Hello from the AI SDK!"))
                .await
                .unwrap();
            assert!(result.response.timestamp.is_some());
            assert_eq!(result.response.model_id.as_deref(), Some(LEGACY_MODEL));
            assert_eq!(
                result
                    .response
                    .headers
                    .unwrap()
                    .get("x-request-id")
                    .map(String::as_str),
                Some("test-request-id")
            );
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › doGenerate › should have no warnings on the happy path
        #[tokio::test]
        async fn has_no_warnings_on_the_happy_path() {
            let server = MockServer::start().await;
            mount(&server, "audio/L16;rate=24000", PCM_BASE64).await;
            let result = model(&server, LEGACY_MODEL)
                .do_generate(&options("Hello from the AI SDK!"))
                .await
                .unwrap();
            assert!(result.warnings.is_empty());
        }
    }

    /// A response without usage (or with `usageMetadata: null`, which
    /// `google-speech-api.ts:35` reads as `.nullish()`) leaves `usage` unset
    /// (`@ai-sdk/google@4.0.87 src/google-speech-model.ts:356`:
    /// `...(response.usageMetadata != null && { usage })`).
    #[tokio::test]
    async fn omits_usage_the_response_does_not_report() {
        for usage in [None, Some(Value::Null)] {
            let server = MockServer::start().await;
            let mut body = json!({
                "candidates": [{"content": {"parts": [{"inlineData": {"data": WAV_BASE64, "mimeType": "audio/wav"}}]}}],
            });
            if let Some(usage) = usage.clone() {
                body["usageMetadata"] = usage;
            }
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            let result = model(&server, LEGACY_MODEL)
                .do_generate(&options("Hello."))
                .await
                .unwrap();
            assert_eq!(result.usage, None, "{usage:?}");
        }
    }

    /// partial: `preserves legacy requests and PCM conversion for %s`; full port in src/google/tests/speech_model.rs.
    #[tokio::test]
    async fn preserves_legacy_requests_and_pcm_conversion() {
        for model_id in [
            "gemini-2.5-flash-preview-tts",
            "gemini-2.5-pro-preview-tts",
            "gemini-2.5-flash-tts",
            "gemini-3.1-flash-tts-preview",
            "gemini-3.1-flash-tts",
        ] {
            let server = MockServer::start().await;
            mount(&server, "audio/L16;rate=24000", PCM_BASE64).await;
            let mut call = options("Hello.");
            call.instructions = Some("Whisper".into());
            call.output_format = Some("wav".into());
            let result = model(&server, model_id).do_generate(&call).await.unwrap();
            assert_eq!(
                request_body(&server, 0).await,
                json!({
                    "contents": [{"role": "user", "parts": [{"text": "Whisper: Hello."}]}],
                    "generationConfig": {
                        "responseModalities": ["AUDIO"],
                        "speechConfig": {"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Kore"}}},
                    },
                }),
                "{model_id}"
            );
            let audio = audio(&result);
            assert_eq!(&audio[..4], b"RIFF", "{model_id}");
            assert_eq!(&audio[44..], PCM_BYTES, "{model_id}");
        }
    }

    /// partial: `should pass headers`; full port in src/google/tests/speech_model.rs.
    #[tokio::test]
    async fn passes_headers() {
        let server = MockServer::start().await;
        mount(&server, "audio/L16;rate=24000", PCM_BASE64).await;
        let provider = create_google(GoogleProviderSettings {
            api_key: Some("test-api-key".to_string()),
            base_url: Some(server.uri()),
            headers: Some(
                [(
                    "Custom-Provider-Header".to_string(),
                    Some("provider-header-value".to_string()),
                )]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        })
        .unwrap();
        let mut call = options("Hello from the AI SDK!");
        call.headers = Some(
            [(
                "Custom-Request-Header".to_string(),
                "request-header-value".to_string(),
            )]
            .into_iter()
            .collect(),
        );
        provider
            .speech(LEGACY_MODEL)
            .do_generate(&call)
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        let header = |name: &str| {
            requests[0]
                .headers
                .get(name)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(header("content-type"), "application/json");
        assert_eq!(header("x-goog-api-key"), "test-api-key");
        assert_eq!(header("custom-provider-header"), "provider-header-value");
        assert_eq!(header("custom-request-header"), "request-header-value");
        assert!(header("user-agent").contains("ai-sdk-google/"));
    }

    // ── modern models (describe.each) ───────────────────────────────────────────

    mod modern_models {
        use super::*;

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › %s › preserves the transcript and maps instructions to speech metadata
        #[tokio::test]
        async fn modern_maps_instructions_to_speech_metadata() {
            for model_id in MODERN_MODELS {
                let server = MockServer::start().await;
                mount(&server, "audio/wav", WAV_BASE64).await;
                let mut call = options("Hello. <laugh> How are you? <short pause>");
                call.instructions = Some("whispering".into());
                let result = model(&server, model_id).do_generate(&call).await.unwrap();
                assert_eq!(
                    request_body(&server, 0).await,
                    json!({
                        "contents": [{"role": "user", "parts": [{
                            "text": "Hello. <laugh> How are you? <short pause>",
                            "speechMetadata": {"style": "whispering"},
                        }]}],
                        "generationConfig": {
                            "responseModalities": ["AUDIO"],
                            "speechConfig": {"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Kore"}}},
                        },
                    }),
                    "{model_id}"
                );
                assert_eq!(audio(&result), wav_bytes());
                assert!(result.warnings.is_empty());
            }
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › %s › allows an empty style to override instructions
        #[tokio::test]
        async fn modern_allows_an_empty_style_to_override_instructions() {
            for model_id in MODERN_MODELS {
                let server = MockServer::start().await;
                mount(&server, "audio/wav", WAV_BASE64).await;
                let mut call = options("Hello");
                call.instructions = Some("excited".into());
                call.provider_options = google_options(json!({"speechMetadata": {"style": ""}}));
                model(&server, model_id).do_generate(&call).await.unwrap();
                assert_eq!(
                    request_body(&server, 0).await["contents"][0]["parts"],
                    json!([{"text": "Hello", "speechMetadata": {"style": ""}}]),
                    "{model_id}"
                );
            }
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › %s › rejects custom voice %s before fetching
        #[tokio::test]
        async fn modern_rejects_custom_voice_before_fetching() {
            for model_id in MODERN_MODELS {
                for voice in ["voice_custom", "voicekey_custom"] {
                    let server = MockServer::start().await;
                    let mut call = options("Hello");
                    call.voice = Some(voice.into());
                    let error = model(&server, model_id)
                        .do_generate(&call)
                        .await
                        .unwrap_err();
                    assert!(invalid_argument(error).contains("parameter voice"));
                    assert!(server.received_requests().await.unwrap().is_empty());
                }
            }
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › %s › sends separate turns with explicit speakers and per-turn styles
        #[tokio::test]
        async fn modern_sends_separate_turns_with_speakers_and_styles() {
            for model_id in MODERN_MODELS {
                let server = MockServer::start().await;
                mount(&server, "audio/wav", WAV_BASE64).await;
                let mut call = options("");
                call.instructions = Some("speaking slowly".into());
                call.provider_options = google_options(json!({
                    "multiSpeakerVoiceConfig": multi_speaker(),
                    "turns": [
                        {"text": "Hi.", "speechMetadata": {"speaker": "Joe"}},
                        {"text": "<sigh> Hello.", "speechMetadata": {"speaker": "Jane", "style": ""}},
                    ],
                }));
                let result = model(&server, model_id).do_generate(&call).await.unwrap();
                let body = request_body(&server, 0).await;
                assert_eq!(
                    body["contents"][0]["parts"],
                    json!([
                        {"text": "Hi.", "speechMetadata": {"speaker": "Joe", "style": "speaking slowly"}},
                        {"text": "<sigh> Hello.", "speechMetadata": {"speaker": "Jane", "style": ""}},
                    ]),
                    "{model_id}"
                );
                assert_eq!(
                    body["generationConfig"]["speechConfig"],
                    json!({"multiSpeakerVoiceConfig": multi_speaker()})
                );
                assert!(result.warnings.is_empty());
            }
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › %s › warns when turns replace nonempty top-level text
        #[tokio::test]
        async fn modern_warns_when_turns_replace_nonempty_top_level_text() {
            for model_id in MODERN_MODELS {
                let server = MockServer::start().await;
                mount(&server, "audio/wav", WAV_BASE64).await;
                let mut call = options("Ignored");
                call.provider_options = google_options(json!({"turns": [{"text": "Spoken"}]}));
                let result = model(&server, model_id).do_generate(&call).await.unwrap();
                assert!(has_warning(&result, "text"));
                assert_eq!(
                    request_body(&server, 0).await["contents"][0]["parts"],
                    json!([{"text": "Spoken"}])
                );
            }
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › %s › requests %s and preserves raw bytes
        #[tokio::test]
        async fn modern_requests_raw_formats_and_preserves_bytes() {
            for (output_format, mime_type, response_mime_type) in [
                ("pcm", "AUDIO_L16", "audio/l16"),
                ("audio/l16", "AUDIO_L16", "audio/l16"),
                ("mulaw", "AUDIO_MULAW", "audio/mulaw"),
                ("audio/mulaw", "AUDIO_MULAW", "audio/mulaw"),
                ("alaw", "AUDIO_ALAW", "audio/alaw"),
                ("audio/alaw", "AUDIO_ALAW", "audio/alaw"),
            ] {
                for model_id in MODERN_MODELS {
                    let server = MockServer::start().await;
                    mount(
                        &server,
                        &format!("{response_mime_type}; rate=24000; channels=1"),
                        PCM_BASE64,
                    )
                    .await;
                    let mut call = options("Hello");
                    call.output_format = Some(output_format.into());
                    let result = model(&server, model_id).do_generate(&call).await.unwrap();
                    assert_eq!(
                        request_body(&server, 0).await["generationConfig"]["responseFormat"],
                        json!({"audio": {"mimeType": mime_type}})
                    );
                    assert_eq!(audio(&result), PCM_BYTES);
                    assert!(result.warnings.is_empty());
                    assert_eq!(
                        serde_json::to_value(result.provider_metadata.unwrap()).unwrap()["google"]
                            ["sampleRate"],
                        24000
                    );
                }
            }
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › %s › requests %s without adding another header
        #[tokio::test]
        async fn modern_requests_wav_without_adding_another_header() {
            for output_format in ["wav", "audio/wav"] {
                for model_id in MODERN_MODELS {
                    let server = MockServer::start().await;
                    mount(&server, "audio/wav", WAV_BASE64).await;
                    let mut call = options("Hello");
                    call.output_format = Some(output_format.into());
                    let result = model(&server, model_id).do_generate(&call).await.unwrap();
                    assert_eq!(
                        request_body(&server, 0).await["generationConfig"]["responseFormat"],
                        json!({"audio": {"mimeType": "AUDIO_WAV"}})
                    );
                    assert_eq!(audio(&result), wav_bytes());
                }
            }
        }

        /// upstream: @ai-sdk/google src/google-speech-model.test.ts › %s › returns empty audio without a WAV header
        #[tokio::test]
        async fn modern_returns_empty_audio_without_a_wav_header() {
            for model_id in MODERN_MODELS {
                let server = MockServer::start().await;
                mount(&server, "audio/wav", "").await;
                let result = model(&server, model_id)
                    .do_generate(&options("Hello"))
                    .await
                    .unwrap();
                assert!(audio(&result).is_empty());
            }
        }
    }

    /// partial: `rejects custom speaker voice configuration %j before fetching`; full port in src/google/tests/speech_model.rs.
    #[tokio::test]
    async fn modern_rejects_custom_speaker_voice_configuration_before_fetching() {
        for voice_config in [
            json!({"voice": "voice_custom"}),
            json!({"voice": "voicekey_custom"}),
            json!({"voice": "voice_custom", "prebuiltVoiceConfig": {"voiceName": "Kore"}}),
        ] {
            let server = MockServer::start().await;
            let mut call = options("");
            call.provider_options = google_options(json!({
                "turns": [{"text": "Hello", "speechMetadata": {"speaker": "Joe"}}],
                "multiSpeakerVoiceConfig": {"speakerVoiceConfigs": [{"speaker": "Joe", "voiceConfig": voice_config}]},
            }));
            let error = model(&server, MODERN_MODELS[0])
                .do_generate(&call)
                .await
                .unwrap_err();
            assert!(invalid_argument(error).contains("providerOptions"));
            assert!(server.received_requests().await.unwrap().is_empty());
        }
    }

    /// partial: `rejects missing or unconfigured speaker %s`; full port in src/google/tests/speech_model.rs.
    #[tokio::test]
    async fn modern_rejects_missing_or_unconfigured_speaker() {
        for speaker_metadata in [json!({}), json!({"speaker": "Unknown"})] {
            let server = MockServer::start().await;
            let mut call = options("");
            call.provider_options = google_options(json!({
                "multiSpeakerVoiceConfig": multi_speaker(),
                "turns": [{"text": "Hello", "speechMetadata": speaker_metadata}],
            }));
            let error = model(&server, MODERN_MODELS[0])
                .do_generate(&call)
                .await
                .unwrap_err();
            assert!(invalid_argument(error).contains("Every multi-speaker turn must specify"));
            assert!(server.received_requests().await.unwrap().is_empty());
        }
    }

    /// partial: `rejects a labelled transcript without structured speakers`; full port in src/google/tests/speech_model.rs.
    #[tokio::test]
    async fn modern_rejects_a_labelled_transcript_without_structured_speakers() {
        let server = MockServer::start().await;
        let mut call = options("Joe: Hi. Jane: Hello.");
        call.provider_options = google_options(json!({"multiSpeakerVoiceConfig": multi_speaker()}));
        let error = model(&server, MODERN_MODELS[0])
            .do_generate(&call)
            .await
            .unwrap_err();
        assert!(invalid_argument(error).contains("Every multi-speaker turn must specify"));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    /// partial: `rejects conflicting global and per-turn metadata`; full port in src/google/tests/speech_model.rs.
    #[tokio::test]
    async fn modern_rejects_conflicting_global_and_per_turn_metadata() {
        let server = MockServer::start().await;
        let mut call = options("");
        call.provider_options = google_options(json!({
            "speechMetadata": {"style": "excited"},
            "turns": [{"text": "Hello"}],
        }));
        let error = model(&server, MODERN_MODELS[0])
            .do_generate(&call)
            .await
            .unwrap_err();
        assert!(invalid_argument(error).contains("Set speechMetadata on each turn"));
    }
}
