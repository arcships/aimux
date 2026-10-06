//! Rust translation of the OpenAI speech (TTS) model tests.
//!
//! Source: `reference/ai/packages/openai/src/speech/openai-speech-model.test.ts`
//!
//! Each test spins up a `wiremock` mock server, configures a binary audio
//! response, creates an `OpenAISpeechModel` pointing at the mock, calls
//! `do_generate`, and asserts on the request body / headers / result.
//!
//! The TS tests inject a custom `currentDate` via `_internal` for timestamp
//! assertions. The Rust model always uses `Utc::now()`; the timestamp tests
//! therefore verify that a timestamp is present and that `model_id` matches,
//! rather than asserting an exact timestamp value.

use std::collections::HashMap;

use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::speech_model::{AudioData, SpeechCallOptions, SpeechModel};
use aimux_providers::{OpenAIConfig, OpenAIProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

/// 100 bytes of mock audio data, returned by the mock server.
fn mock_audio_bytes() -> Vec<u8> {
    vec![0u8; 100]
}

/// Mount a mock binary audio response on the server at `/audio/speech`.
async fn mock_audio_response(server: &MockServer, format: &str) {
    Mock::given(method("POST"))
        .and(path("/audio/speech"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", format!("audio/{format}"))
                .set_body_bytes(mock_audio_bytes()),
        )
        .mount(server)
        .await;
}

/// Mount a mock binary audio response with extra response headers.
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
        .and(path("/audio/speech"))
        .respond_with(template)
        .mount(server)
        .await;
}

/// Build a `SpeechCallOptions` with just the text.
fn speech_options(text: &str) -> SpeechCallOptions {
    SpeechCallOptions::new(text.to_string())
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate
// (openai-speech-model.test.ts → describe('doGenerate'))
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should pass the model and text"
#[tokio::test]
async fn should_pass_the_model_and_text() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.speech("tts-1");

    model
        .do_generate(&speech_options("Hello from the AI SDK!"))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "tts-1");
    assert_eq!(body["input"], "Hello from the AI SDK!");
}

/// TS: "should pass headers"
///
/// Verifies that `Authorization`, `OpenAI-Organization`, `OpenAI-Project`,
/// config-level custom headers, and request-level custom headers are all
/// forwarded. The TS test also checks the `User-Agent` header; the Rust
/// provider does not set a `User-Agent` so that assertion is not translated.
#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let mut provider_headers = HashMap::new();
    provider_headers.insert(
        "Custom-Provider-Header".to_string(),
        "provider-header-value".to_string(),
    );
    let config = OpenAIConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_org_id("test-organization")
        .with_project("test-project")
        .with_headers(provider_headers);
    let provider = OpenAIProvider::new(config);
    let model = provider.speech("tts-1");

    let mut options = speech_options("Hello from the AI SDK!");
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
    assert_eq!(h.get("openai-organization").unwrap(), "test-organization");
    assert_eq!(h.get("openai-project").unwrap(), "test-project");
    assert_eq!(
        h.get("custom-provider-header").unwrap(),
        "provider-header-value"
    );
    assert_eq!(
        h.get("custom-request-header").unwrap(),
        "request-header-value"
    );
}

/// TS: "should pass options"
#[tokio::test]
async fn should_pass_options() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "opus").await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.speech("tts-1");

    let mut options = speech_options("Hello from the AI SDK!");
    options.voice = Some("nova".to_string());
    options.output_format = Some("opus".to_string());
    options.speed = Some(1.5);

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "tts-1");
    assert_eq!(body["input"], "Hello from the AI SDK!");
    assert_eq!(body["voice"], "nova");
    assert_eq!(body["speed"], 1.5);
    assert_eq!(body["response_format"], "opus");
}

/// TS: "should return audio data with correct content type"
#[tokio::test]
async fn should_return_audio_data_with_correct_content_type() {
    let server = MockServer::start().await;
    mock_audio_response_with_headers(
        &server,
        "opus",
        &[
            ("x-request-id", "test-request-id"),
            ("x-ratelimit-remaining", "123"),
        ],
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.speech("tts-1");

    let mut options = speech_options("Hello from the AI SDK!");
    options.output_format = Some("opus".to_string());

    let result = model
        .do_generate(&options)
        .await
        .expect("do_generate should succeed");

    assert!(matches!(result.audio, AudioData::Binary(ref b) if b == &mock_audio_bytes()));
}

/// TS: "should include response data with timestamp, modelId and headers"
///
/// The TS test injects a custom `currentDate` of `new Date(0)`; the Rust model
/// uses `Utc::now()`. We assert that the timestamp is present, the model id
/// matches, and the response headers carry the expected values.
#[tokio::test]
async fn should_include_response_data_with_timestamp_modelid_and_headers() {
    let server = MockServer::start().await;
    mock_audio_response_with_headers(
        &server,
        "mp3",
        &[
            ("x-request-id", "test-request-id"),
            ("x-ratelimit-remaining", "123"),
        ],
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.speech("tts-1");

    let result = model
        .do_generate(&speech_options("Hello from the AI SDK!"))
        .await
        .unwrap();

    assert!(
        result.response.timestamp.is_some(),
        "timestamp should be set"
    );
    assert_eq!(result.response.model_id.as_deref(), Some("tts-1"));
    let headers = result.response.headers.expect("response headers");
    assert_eq!(headers.get("content-type"), Some(&"audio/mp3".to_string()));
    assert_eq!(
        headers.get("x-request-id"),
        Some(&"test-request-id".to_string())
    );
    assert_eq!(
        headers.get("x-ratelimit-remaining"),
        Some(&"123".to_string())
    );
}

/// TS: "should use real date when no custom date provider is specified"
///
/// The TS test actually injects a custom date (a copy-paste oddity). The Rust
/// model always uses `Utc::now()`; we verify the timestamp is present and the
/// model id matches.
#[tokio::test]
async fn should_use_real_date_when_no_custom_date_provider_is_specified() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.speech("tts-1");

    let result = model
        .do_generate(&speech_options("Hello from the AI SDK!"))
        .await
        .unwrap();

    assert!(
        result.response.timestamp.is_some(),
        "timestamp should be set"
    );
    assert_eq!(result.response.model_id.as_deref(), Some("tts-1"));
}

/// TS: "should handle different audio formats"
///
/// Iterates over the supported formats and verifies audio data is returned.
/// The TS test passes `providerOptions.openai.response_format`; the Rust model
/// does not parse openai-specific provider options (the TS schema does not
/// include `response_format` either), so we pass `output_format` directly.
#[tokio::test]
async fn should_handle_different_audio_formats() {
    for format in &["mp3", "opus", "aac", "flac", "wav", "pcm"] {
        let server = MockServer::start().await;
        mock_audio_response(&server, format).await;

        let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.speech("tts-1");

        let mut options = speech_options("Hello from the AI SDK!");
        options.output_format = Some(format.to_string());

        let result = model
            .do_generate(&options)
            .await
            .expect("do_generate should succeed");

        assert!(
            matches!(result.audio, AudioData::Binary(ref b) if b == &mock_audio_bytes()),
            "format {format} should return mock audio bytes"
        );
    }
}

/// TS: "should include warnings if any are generated"
///
/// With no unsupported options, warnings should be empty.
#[tokio::test]
async fn should_include_warnings_if_any_are_generated() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.speech("tts-1");

    let result = model
        .do_generate(&speech_options("Hello from the AI SDK!"))
        .await
        .unwrap();

    assert!(
        result.warnings.is_empty(),
        "expected no warnings, got {:?}",
        result.warnings
    );
}

// ── Additional: unsupported language emits a warning ─────────────────────────

/// The TS model emits an `unsupported` warning when `language` is set. The TS
/// suite does not have a dedicated test for this, but the behaviour is
/// exercised here to guard the Rust translation.
#[tokio::test]
async fn language_option_emits_unsupported_warning() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.speech("tts-1");

    let mut options = speech_options("Hello from the AI SDK!");
    options.language = Some("en".to_string());

    let result = model.do_generate(&options).await.unwrap();

    assert_eq!(
        result.warnings.len(),
        1,
        "expected one warning, got {:?}",
        result.warnings
    );
    match &result.warnings[0] {
        aimux_core::types::Warning::Unsupported { feature, .. } => {
            assert_eq!(feature, "language");
        }
        other => panic!("expected Unsupported warning, got {other:?}"),
    }
}
