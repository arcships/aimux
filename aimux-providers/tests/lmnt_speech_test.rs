//! Rust translation of the LMNT speech (TTS) model tests.
//!
//! Source: `reference/ai/packages/lmnt/src/lmnt-speech-model.test.ts`
//!
//! Each test spins up a `wiremock` mock server, configures a binary audio
//! response, creates an `LMNTSpeechModel` pointing at the mock, calls
//! `do_generate`, and asserts on the request body / headers / result.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::speech_model::{AudioData, SpeechCallOptions, SpeechModel};
use aimux_providers::{LMNTConfig, LMNTProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

fn mock_audio_bytes() -> Vec<u8> {
    vec![0u8; 100]
}

async fn mock_audio_response(server: &MockServer, format: &str) {
    Mock::given(method("POST"))
        .and(path("/v1/ai/speech/bytes"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", format!("audio/{}", format))
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
        .insert_header("content-type", format!("audio/{}", format))
        .set_body_bytes(mock_audio_bytes());
    for (k, v) in headers {
        template = template.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/v1/ai/speech/bytes"))
        .respond_with(template)
        .mount(server)
        .await;
}

fn speech_options(text: &str) -> SpeechCallOptions {
    SpeechCallOptions::new(text.to_string())
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate
// (lmnt-speech-model.test.ts → describe('doGenerate'))
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should pass the model and text"
#[tokio::test]
async fn should_pass_the_model_and_text() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = LMNTConfig::new("test-api-key").with_base_url(server.uri());
    let provider = LMNTProvider::new(config);
    let model = provider.speech("aurora");

    model
        .do_generate(&speech_options("Hello from the AI SDK!"))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "aurora");
    assert_eq!(body["text"], "Hello from the AI SDK!");
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
    let config = LMNTConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_headers(provider_headers);
    let provider = LMNTProvider::new(config);
    let model = provider.speech("aurora");

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
    assert_eq!(h.get("x-api-key").unwrap(), "test-api-key");
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
    mock_audio_response(&server, "mp3").await;

    let config = LMNTConfig::new("test-api-key").with_base_url(server.uri());
    let provider = LMNTProvider::new(config);
    let model = provider.speech("aurora");

    let mut options = speech_options("Hello from the AI SDK!");
    options.voice = Some("nova".to_string());
    options.output_format = Some("mp3".to_string());
    options.speed = Some(1.5);

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "aurora");
    assert_eq!(body["text"], "Hello from the AI SDK!");
    assert_eq!(body["voice"], "nova");
    assert_eq!(body["speed"], 1.5);
    assert_eq!(body["response_format"], "mp3");
}

/// TS: "should return audio data with correct content type"
#[tokio::test]
async fn should_return_audio_data_with_correct_content_type() {
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

    let config = LMNTConfig::new("test-api-key").with_base_url(server.uri());
    let provider = LMNTProvider::new(config);
    let model = provider.speech("aurora");

    let mut options = speech_options("Hello from the AI SDK!");
    options.output_format = Some("mp3".to_string());

    let result = model.do_generate(&options).await.unwrap();

    assert!(matches!(result.audio, AudioData::Binary(ref b) if b == &mock_audio_bytes()));
}

/// TS: "should include response data with timestamp, modelId and headers"
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

    let config = LMNTConfig::new("test-api-key").with_base_url(server.uri());
    let provider = LMNTProvider::new(config);
    let model = provider.speech("aurora");

    let result = model
        .do_generate(&speech_options("Hello from the AI SDK!"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id.as_deref(), Some("aurora"));
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
#[tokio::test]
async fn should_use_real_date_when_no_custom_date_provider_is_specified() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = LMNTConfig::new("test-api-key").with_base_url(server.uri());
    let provider = LMNTProvider::new(config);
    let model = provider.speech("aurora");

    let result = model
        .do_generate(&speech_options("Hello from the AI SDK!"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id.as_deref(), Some("aurora"));
}

/// TS: "should handle different audio formats"
///
/// The TS test passes `providerOptions.lmnt.format`. The Rust model parses
/// `lmnt.format` from provider options and overrides `response_format`.
#[tokio::test]
async fn should_handle_different_audio_formats() {
    for format in &["aac", "mp3", "mulaw", "raw", "wav"] {
        let server = MockServer::start().await;
        mock_audio_response(&server, format).await;

        let config = LMNTConfig::new("test-api-key").with_base_url(server.uri());
        let provider = LMNTProvider::new(config);
        let model = provider.speech("aurora");

        let mut options = speech_options("Hello from the AI SDK!");
        let mut provider_options = HashMap::new();
        provider_options.insert("lmnt".to_string(), json!({ "format": format }));
        options.provider_options = Some(provider_options);

        let result = model.do_generate(&options).await.unwrap();

        assert!(
            matches!(result.audio, AudioData::Binary(ref b) if b == &mock_audio_bytes()),
            "format {} should return mock audio bytes",
            format
        );
    }
}

/// TS: "should include warnings if any are generated"
#[tokio::test]
async fn should_include_warnings_if_any_are_generated() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = LMNTConfig::new("test-api-key").with_base_url(server.uri());
    let provider = LMNTProvider::new(config);
    let model = provider.speech("aurora");

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
