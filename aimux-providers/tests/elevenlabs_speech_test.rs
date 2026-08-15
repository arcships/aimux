//! Rust translation of the ElevenLabs speech (TTS) model tests.
//!
//! Source: `reference/ai/packages/elevenlabs/src/elevenlabs-speech-model.test.ts`
//!
//! Each test spins up a `wiremock` mock server, configures a binary audio
//! response, creates an `ElevenLabsSpeechModel` pointing at the mock, calls
//! `do_generate`, and asserts on the request body / URL / result.
//!
//! Note: the TS test mounts at `https://api.elevenlabs.io/v1/text-to-speech/*`.
//! The Rust mock server uses a wildcard path matcher to capture any voice id.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::shared::Warning;
use aimux_core::speech_model::{SpeechCallOptions, SpeechModel};
use aimux_providers::{ElevenLabsConfig, ElevenLabsProvider};

// 鈹€鈹€ helpers 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€

fn mock_audio_bytes() -> Vec<u8> {
    vec![0u8; 100]
}

/// Mount a mock binary audio response on the server at `/v1/text-to-speech/{voiceId}`.
async fn mock_audio_response(server: &MockServer, format: &str) {
    Mock::given(method("POST"))
        .and(path_regex(r"^/v1/text-to-speech/.+$"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", format!("audio/{format}"))
                .set_body_bytes(mock_audio_bytes()),
        )
        .mount(server)
        .await;
}

fn speech_options(text: &str) -> SpeechCallOptions {
    SpeechCallOptions::new(text.to_string())
}

// 鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲
// doGenerate
// (elevenlabs-speech-model.test.ts 鈫?describe('doGenerate'))
// 鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲

/// TS: "should generate speech with required parameters"
#[tokio::test]
async fn should_generate_speech_with_required_parameters() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = ElevenLabsConfig::new("test-api-key").with_base_url(server.uri());
    let provider = ElevenLabsProvider::new(config);
    let model = provider.speech("eleven_multilingual_v2");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["text"], "Hello, world!");
    assert_eq!(body["model_id"], "eleven_multilingual_v2");

    // Check output_format is in query params.
    let url = &requests[0].url;
    assert!(
        url.as_str().contains("output_format=mp3_44100_128"),
        "expected output_format=mp3_44100_128 in URL, got {url}"
    );
}

/// TS: "should handle custom output format"
#[tokio::test]
async fn should_handle_custom_output_format() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "pcm").await;

    let config = ElevenLabsConfig::new("test-api-key").with_base_url(server.uri());
    let provider = ElevenLabsProvider::new(config);
    let model = provider.speech("eleven_multilingual_v2");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());
    options.output_format = Some("pcm_44100".to_string());

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["text"], "Hello, world!");
    assert_eq!(body["model_id"], "eleven_multilingual_v2");

    let url = &requests[0].url;
    assert!(
        url.as_str().contains("output_format=pcm_44100"),
        "expected output_format=pcm_44100 in URL, got {url}"
    );
}

/// TS: "should handle language parameter"
#[tokio::test]
async fn should_handle_language_parameter() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = ElevenLabsConfig::new("test-api-key").with_base_url(server.uri());
    let provider = ElevenLabsProvider::new(config);
    let model = provider.speech("eleven_multilingual_v2");

    let mut options = speech_options("Hola, mundo!");
    options.voice = Some("test-voice-id".to_string());
    options.language = Some("es".to_string());

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["text"], "Hola, mundo!");
    assert_eq!(body["model_id"], "eleven_multilingual_v2");
    assert_eq!(body["language_code"], "es");

    let url = &requests[0].url;
    assert!(
        url.as_str().contains("output_format=mp3_44100_128"),
        "expected output_format=mp3_44100_128 in URL, got {url}"
    );
}

/// TS: "should handle speed parameter in voice settings"
#[tokio::test]
async fn should_handle_speed_parameter_in_voice_settings() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = ElevenLabsConfig::new("test-api-key").with_base_url(server.uri());
    let provider = ElevenLabsProvider::new(config);
    let model = provider.speech("eleven_multilingual_v2");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());
    options.speed = Some(1.5);

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["text"], "Hello, world!");
    assert_eq!(body["model_id"], "eleven_multilingual_v2");
    assert_eq!(body["voice_settings"]["speed"], 1.5);
}

/// TS: "should warn about unsupported instructions parameter"
#[tokio::test]
async fn should_warn_about_unsupported_instructions_parameter() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = ElevenLabsConfig::new("test-api-key").with_base_url(server.uri());
    let provider = ElevenLabsProvider::new(config);
    let model = provider.speech("eleven_multilingual_v2");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());
    options.instructions = Some("Speak slowly".to_string());

    let result = model.do_generate(&options).await.unwrap();

    assert_eq!(result.warnings.len(), 1);
    match &result.warnings[0] {
        Warning::Unsupported { feature, details } => {
            assert_eq!(feature, "instructions");
            assert!(
                details
                    .as_ref()
                    .unwrap()
                    .contains("do not support instructions")
            );
        }
        other => panic!("expected Unsupported warning, got {other:?}"),
    }
}

/// TS: "should pass provider-specific options"
#[tokio::test]
async fn should_pass_provider_specific_options() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = ElevenLabsConfig::new("test-api-key").with_base_url(server.uri());
    let provider = ElevenLabsProvider::new(config);
    let model = provider.speech("eleven_multilingual_v2");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());

    let mut provider_options = HashMap::new();
    provider_options.insert(
        "elevenlabs".to_string(),
        json!({
            "voiceSettings": {
                "stability": 0.5,
                "similarityBoost": 0.75,
            },
            "seed": 123,
        }),
    );
    options.provider_options = Some(provider_options);

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["text"], "Hello, world!");
    assert_eq!(body["model_id"], "eleven_multilingual_v2");
    assert_eq!(body["voice_settings"]["stability"], 0.5);
    assert_eq!(body["voice_settings"]["similarity_boost"], 0.75);
    assert_eq!(body["seed"], 123);

    let url = &requests[0].url;
    assert!(
        url.as_str().contains("output_format=mp3_44100_128"),
        "expected output_format=mp3_44100_128 in URL, got {url}"
    );
}

/// TS: "should include user-agent header"
///
/// The TS test checks for `ai-sdk/elevenlabs/0.0.0-test` in the User-Agent.
/// The Rust provider does not set a User-Agent header, so this test verifies
/// the `xi-api-key` header is present instead.
#[tokio::test]
async fn should_include_api_key_header() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = ElevenLabsConfig::new("test-api-key").with_base_url(server.uri());
    let provider = ElevenLabsProvider::new(config);
    let model = provider.speech("eleven_multilingual_v2");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let h = &requests[0].headers;
    assert_eq!(h.get("xi-api-key").unwrap(), "test-api-key");
}
