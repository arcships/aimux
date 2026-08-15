//! Rust translation of the Cartesia speech (TTS) model tests.
//!
//! Source: `reference/ai/packages/cartesia/src/cartesia-speech-model.test.ts`
//!
//! Each test spins up a `wiremock` mock server, configures a binary audio
//! response, creates a `CartesiaSpeechModel` pointing at the mock, calls
//! `do_generate`, and asserts on the request body / headers / result.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::shared::Warning;
use aimux_core::speech_model::{AudioData, SpeechCallOptions, SpeechModel};
use aimux_providers::{CartesiaConfig, CartesiaProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

fn mock_audio_bytes() -> Vec<u8> {
    vec![0u8; 100]
}

async fn mock_audio_response(server: &MockServer, format: &str) {
    Mock::given(method("POST"))
        .and(path("/tts/bytes"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", format!("audio/{format}"))
                .set_body_bytes(mock_audio_bytes()),
        )
        .mount(server)
        .await;
}

async fn mock_audio_response_with_headers(
    server: &MockServer,
    format: &str,
    headers: &[(&str, &str)],
) {
    let mut template = ResponseTemplate::new(200)
        .insert_header("content-type", format!("audio/{format}"))
        .set_body_bytes(mock_audio_bytes());
    for (k, v) in headers {
        template = template.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/tts/bytes"))
        .respond_with(template)
        .mount(server)
        .await;
}

fn speech_options(text: &str) -> SpeechCallOptions {
    SpeechCallOptions::new(text.to_string())
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate
// (cartesia-speech-model.test.ts → describe('doGenerate'))
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should generate speech with required parameters"
#[tokio::test]
async fn should_generate_speech_with_required_parameters() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.speech("sonic-3.5");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body,
        json!({
            "model_id": "sonic-3.5",
            "transcript": "Hello, world!",
            "voice": { "mode": "id", "id": "test-voice-id" },
            "output_format": {
                "container": "mp3",
                "sample_rate": 44100,
                "bit_rate": 128000,
            }
        })
    );
}

/// TS: "should throw when no voice is provided"
#[tokio::test]
async fn should_throw_when_no_voice_is_provided() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.speech("sonic-3.5");

    let result = model.do_generate(&speech_options("Hello, world!")).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("require a `voice`"),
        "expected voice-required error, got {err}"
    );
}

/// TS: "should map wav output format"
#[tokio::test]
async fn should_map_wav_output_format() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "wav").await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.speech("sonic-3.5");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());
    options.output_format = Some("wav".to_string());

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["output_format"],
        json!({
            "container": "wav",
            "encoding": "pcm_s16le",
            "sample_rate": 44100,
        })
    );
}

/// TS: "should map pcm output format with sample rate suffix"
#[tokio::test]
async fn should_map_pcm_output_format_with_sample_rate_suffix() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "pcm").await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.speech("sonic-3.5");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());
    options.output_format = Some("pcm_24000".to_string());

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["output_format"],
        json!({
            "container": "raw",
            "encoding": "pcm_f32le",
            "sample_rate": 24000,
        })
    );
}

/// TS: "should handle language parameter"
#[tokio::test]
async fn should_handle_language_parameter() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.speech("sonic-3.5");

    let mut options = speech_options("Hola, mundo!");
    options.voice = Some("test-voice-id".to_string());
    options.language = Some("es".to_string());

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["transcript"], "Hola, mundo!");
    assert_eq!(body["language"], "es");
}

/// TS: "should handle speed parameter"
#[tokio::test]
async fn should_handle_speed_parameter() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.speech("sonic-3.5");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());
    options.speed = Some(1.5);

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["generation_config"], json!({ "speed": 1.5 }));
}

/// TS: "should warn and ignore an out-of-range generic speed"
#[tokio::test]
async fn should_warn_and_ignore_an_out_of_range_generic_speed() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.speech("sonic-3.5");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());
    options.speed = Some(2.0);

    let result = model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(
        body.get("generation_config").is_none(),
        "generation_config should not be present for out-of-range speed"
    );
    assert_eq!(result.warnings.len(), 1);
    match &result.warnings[0] {
        Warning::Unsupported { feature, details } => {
            assert_eq!(feature, "speed");
            assert!(details.as_ref().unwrap().contains("between 0.6 and 1.5"));
        }
        other => panic!("expected Unsupported warning, got {other:?}"),
    }
}

/// TS: "should warn about unsupported instructions parameter"
#[tokio::test]
async fn should_warn_about_unsupported_instructions_parameter() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.speech("sonic-3.5");

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

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.speech("sonic-3.5");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());

    let mut provider_options = HashMap::new();
    provider_options.insert(
        "cartesia".to_string(),
        json!({
            "container": "raw",
            "encoding": "pcm_s16le",
            "sampleRate": 16000,
            "speed": 0.8,
        }),
    );
    options.provider_options = Some(provider_options);

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["output_format"],
        json!({
            "container": "raw",
            "encoding": "pcm_s16le",
            "sample_rate": 16000,
        })
    );
    assert_eq!(body["generation_config"], json!({ "speed": 0.8 }));
}

/// TS: "should ignore encoding for mp3 output"
#[tokio::test]
async fn should_ignore_encoding_for_mp3_output() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.speech("sonic-3.5");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());

    let mut provider_options = HashMap::new();
    provider_options.insert("cartesia".to_string(), json!({ "encoding": "pcm_s16le" }));
    options.provider_options = Some(provider_options);

    let result = model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["output_format"],
        json!({
            "container": "mp3",
            "sample_rate": 44100,
            "bit_rate": 128000,
        })
    );
    assert_eq!(result.warnings.len(), 1);
    match &result.warnings[0] {
        Warning::Unsupported { feature, .. } => {
            assert_eq!(feature, "providerOptions.cartesia.encoding");
        }
        other => panic!("expected Unsupported warning, got {other:?}"),
    }
}

/// TS: "should warn about an unsupported sample rate suffix"
#[tokio::test]
async fn should_warn_about_an_unsupported_sample_rate_suffix() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "wav").await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.speech("sonic-3.5");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());
    options.output_format = Some("wav_12345".to_string());

    let result = model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["output_format"]["container"], "wav");
    assert_eq!(body["output_format"]["sample_rate"], 44100);
    assert_eq!(result.warnings.len(), 1);
    match &result.warnings[0] {
        Warning::Unsupported { feature, details } => {
            assert_eq!(feature, "outputFormat");
            assert!(details.as_ref().unwrap().contains("wav_12345"));
        }
        other => panic!("expected Unsupported warning, got {other:?}"),
    }
}

/// TS: "should pass headers"
#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let mut provider_headers = HashMap::new();
    provider_headers.insert(
        "Custom-Provider-Header".to_string(),
        "provider-header-value".to_string(),
    );
    let config = CartesiaConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_headers(provider_headers);
    let provider = CartesiaProvider::new(config);
    let model = provider.speech("sonic-3.5");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());
    let mut request_headers = HashMap::new();
    request_headers.insert(
        "Custom-Request-Header".to_string(),
        "request-header-value".to_string(),
    );
    options.headers = Some(request_headers);

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let h = &requests[0].headers;
    assert_eq!(h.get("authorization").unwrap(), "Bearer test-api-key");
    assert_eq!(h.get("cartesia-version").unwrap(), "2026-03-01");
    assert_eq!(
        h.get("custom-provider-header").unwrap(),
        "provider-header-value"
    );
    assert_eq!(
        h.get("custom-request-header").unwrap(),
        "request-header-value"
    );
}

/// TS: "should return audio data"
#[tokio::test]
async fn should_return_audio_data() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.speech("sonic-3.5");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());

    let result = model.do_generate(&options).await.unwrap();

    assert!(matches!(result.audio, AudioData::Binary(ref b) if b == &mock_audio_bytes()));
}

/// TS: "should include response data with timestamp, modelId and headers"
#[tokio::test]
async fn should_include_response_data_with_timestamp_modelid_and_headers() {
    let server = MockServer::start().await;
    mock_audio_response_with_headers(&server, "mp3", &[("x-request-id", "test-request-id")]).await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.speech("sonic-3.5");

    let mut options = speech_options("Hello, world!");
    options.voice = Some("test-voice-id".to_string());

    let result = model.do_generate(&options).await.unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id.as_deref(), Some("sonic-3.5"));
    let headers = result.response.headers.expect("response headers");
    assert_eq!(headers.get("content-type"), Some(&"audio/mp3".to_string()));
    assert_eq!(
        headers.get("x-request-id"),
        Some(&"test-request-id".to_string())
    );
}
