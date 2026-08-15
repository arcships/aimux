//! Amazon Polly speech (TTS) provider tests.
//!
//! Each test spins up a `wiremock` mock server, configures a binary audio
//! response, creates an `AwsPollySpeechModel` pointing at the mock (with fake
//! credentials), calls `do_generate`, and asserts on the request body /
//! headers / result.
//!
//! No network access, no real credentials.

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::speech_model::{AudioData, SpeechCallOptions, SpeechModel};
use aimux_providers::{AwsPollyConfig, AwsPollyProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

/// Fake credentials used for all tests. Never real.
const TEST_ACCESS_KEY: &str = "test-access-key";
const TEST_SECRET_KEY: &str = "test-secret-key";
const TEST_REGION: &str = "us-east-1";

fn mock_audio_bytes() -> Vec<u8> {
    // A minimal valid MP3-like header followed by zeros — only the bytes
    // need to round-trip through the binary response.
    let mut bytes = vec![0xff, 0xfb, 0x90, 0x00];
    bytes.extend(std::iter::repeat_n(0u8, 96));
    bytes
}

fn test_config(server: &MockServer) -> AwsPollyConfig {
    AwsPollyConfig::new(TEST_ACCESS_KEY, TEST_SECRET_KEY, TEST_REGION).with_base_url(server.uri())
}

fn speech_options(text: &str) -> SpeechCallOptions {
    SpeechCallOptions::new(text.to_string())
}

async fn mock_audio_response(server: &MockServer, format: &str) {
    Mock::given(method("POST"))
        .and(path("/v1/speech"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", format!("audio/{format}"))
                .set_body_bytes(mock_audio_bytes()),
        )
        .mount(server)
        .await;
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate
// ════════════════════════════════════════════════════════════════════════════

/// `do_generate` returns the binary audio stream.
#[tokio::test]
async fn do_generate_returns_binary_audio() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let provider = AwsPollyProvider::new(test_config(&server));
    let model = provider.speech("aws_polly/neural");

    let result = model
        .do_generate(&speech_options("Hello from the AI SDK!"))
        .await
        .unwrap();

    assert!(
        matches!(result.audio, AudioData::Binary(ref b) if b == &mock_audio_bytes()),
        "expected binary audio matching mock bytes"
    );
}

/// The request targets the correct endpoint (`/v1/speech`).
#[tokio::test]
async fn requests_correct_url() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let provider = AwsPollyProvider::new(test_config(&server));
    let model = provider.speech("aws_polly/neural");

    model
        .do_generate(&speech_options("Hello from the AI SDK!"))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert!(!requests.is_empty(), "expected at least one request");
    assert_eq!(requests[0].url.path(), "/v1/speech");
    assert_eq!(requests[0].method.as_str(), "POST");
}

/// The request body carries the engine, text, voice and output format.
#[tokio::test]
async fn request_body_carries_engine_text_voice_and_format() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let provider = AwsPollyProvider::new(test_config(&server));
    let model = provider.speech("aws_polly/neural");

    let mut options = speech_options("Hello from the AI SDK!");
    options.voice = Some("Matthew".to_string());
    options.output_format = Some("mp3".to_string());

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert!(!requests.is_empty(), "expected at least one request");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["Engine"], "neural");
    assert_eq!(body["Text"], "Hello from the AI SDK!");
    assert_eq!(body["VoiceId"], "Matthew");
    assert_eq!(body["OutputFormat"], "mp3");
}

/// The model id maps to the correct Polly engine for each canonical id.
#[tokio::test]
async fn model_id_maps_to_engine() {
    for (model_id, expected_engine) in [
        ("aws_polly/standard", "standard"),
        ("aws_polly/neural", "neural"),
        ("aws_polly/long-form", "long-form"),
        ("aws_polly/generative", "generative"),
    ] {
        let server = MockServer::start().await;
        mock_audio_response(&server, "mp3").await;

        let provider = AwsPollyProvider::new(test_config(&server));
        let model = provider.speech(model_id);

        model.do_generate(&speech_options("Hello")).await.unwrap();

        let requests = server.received_requests().await.expect("requests recorded");
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            body["Engine"], expected_engine,
            "model id {model_id} should map to engine {expected_engine}"
        );
    }
}

/// The request carries a SigV4 `Authorization` header.
#[tokio::test]
async fn request_carries_sigv4_authorization_header() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let provider = AwsPollyProvider::new(test_config(&server));
    let model = provider.speech("aws_polly/neural");

    model
        .do_generate(&speech_options("Hello from the AI SDK!"))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert!(!requests.is_empty(), "expected at least one request");
    let h = &requests[0].headers;
    let auth = h
        .get("authorization")
        .expect("Authorization header should be present");
    let auth_str = auth.to_str().unwrap();
    assert!(
        auth_str.starts_with("AWS4-HMAC-SHA256 "),
        "expected SigV4 Authorization header, got: {auth_str}"
    );
    // The credential scope must reference the polly service.
    assert!(
        auth_str.contains("/polly/aws4_request"),
        "expected polly service in credential scope, got: {auth_str}"
    );
}

/// The request carries the supporting SigV4 headers.
#[tokio::test]
async fn request_carries_sigv4_supporting_headers() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let provider = AwsPollyProvider::new(test_config(&server));
    let model = provider.speech("aws_polly/neural");

    model
        .do_generate(&speech_options("Hello from the AI SDK!"))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let h = &requests[0].headers;
    assert!(
        h.get("x-amz-date").is_some(),
        "x-amz-date header should be present"
    );
    assert!(
        h.get("x-amz-content-sha256").is_some(),
        "x-amz-content-sha256 header should be present"
    );
    assert!(h.get("host").is_some(), "host header should be present");
    // No session token configured → no x-amz-security-token header.
    assert!(
        h.get("x-amz-security-token").is_none(),
        "x-amz-security-token should not be present without a session token"
    );
}

/// A session token is forwarded as `x-amz-security-token` and signed.
#[tokio::test]
async fn session_token_is_forwarded_and_signed() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let config = AwsPollyConfig::new(TEST_ACCESS_KEY, TEST_SECRET_KEY, TEST_REGION)
        .with_base_url(server.uri())
        .with_session_token("test-session-token");
    let provider = AwsPollyProvider::new(config);
    let model = provider.speech("aws_polly/neural");

    model
        .do_generate(&speech_options("Hello from the AI SDK!"))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let h = &requests[0].headers;
    assert_eq!(
        h.get("x-amz-security-token").unwrap().to_str().unwrap(),
        "test-session-token"
    );
    // The session token must be part of the signed headers.
    let auth = h.get("authorization").unwrap().to_str().unwrap();
    assert!(
        auth.contains("SignedHeaders="),
        "Authorization should list signed headers"
    );
    assert!(
        auth.contains("x-amz-security-token"),
        "x-amz-security-token should be a signed header"
    );
}

/// The `language` option maps to `LanguageCode`.
#[tokio::test]
async fn language_maps_to_language_code() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let provider = AwsPollyProvider::new(test_config(&server));
    let model = provider.speech("aws_polly/neural");

    let mut options = speech_options("Hola mundo");
    options.language = Some("es-US".to_string());

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["LanguageCode"], "es-US");
}

/// Provider options (`aws_polly` key) forward Polly-specific fields.
#[tokio::test]
async fn provider_options_forward_polly_fields() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let provider = AwsPollyProvider::new(test_config(&server));
    let model = provider.speech("aws_polly/standard");

    let mut options = speech_options("<speak>Hello</speak>");
    let mut provider_options = std::collections::HashMap::new();
    provider_options.insert(
        "aws_polly".to_string(),
        json!({
            "engine": "neural",
            "sampleRate": "22050",
            "textType": "ssml",
            "lexiconNames": ["lex1"],
            "speechMarkTypes": ["sentence"]
        }),
    );
    options.provider_options = Some(provider_options);

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["Engine"], "neural", "engine override should apply");
    assert_eq!(body["SampleRate"], "22050");
    assert_eq!(body["TextType"], "ssml");
    assert_eq!(body["LexiconNames"], json!(["lex1"]));
    assert_eq!(body["SpeechMarkTypes"], json!(["sentence"]));
}

/// The response carries timestamp, model id and headers.
#[tokio::test]
async fn response_carries_metadata() {
    let server = MockServer::start().await;
    let template = ResponseTemplate::new(200)
        .insert_header("content-type", "audio/mpeg")
        .insert_header("x-amzn-requestid", "test-request-id")
        .set_body_bytes(mock_audio_bytes());
    Mock::given(method("POST"))
        .and(path("/v1/speech"))
        .respond_with(template)
        .mount(&server)
        .await;

    let provider = AwsPollyProvider::new(test_config(&server));
    let model = provider.speech("aws_polly/neural");

    let result = model
        .do_generate(&speech_options("Hello from the AI SDK!"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(
        result.response.model_id.as_deref(),
        Some("aws_polly/neural")
    );
    let headers = result.response.headers.expect("response headers");
    assert_eq!(headers.get("content-type"), Some(&"audio/mpeg".to_string()));
    assert_eq!(
        headers.get("x-amzn-requestid"),
        Some(&"test-request-id".to_string())
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Error mapping
// ════════════════════════════════════════════════════════════════════════════

/// A 401 response maps to `ApiCall` (401 in `status_code`).
#[tokio::test]
async fn error_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/speech"))
        .respond_with(
            ResponseTemplate::new(401).set_body_string(
                r#"{"__type":"UnrecognizedClientException","message":"The security token included in the request is invalid."}"#,
            ),
        )
        .mount(&server)
        .await;

    let provider = AwsPollyProvider::new(test_config(&server));
    let model = provider.speech("aws_polly/neural");

    let err = model
        .do_generate(&speech_options("Hello"))
        .await
        .expect_err("expected an error for 401");

    assert!(
        matches!(err, ref e if e.status_code() == Some(401)),
        "expected Auth error for 401, got: {err:?}"
    );
    // The error must not leak the secret key.
    let err_str = format!("{err:?}");
    assert!(
        !err_str.contains(TEST_SECRET_KEY),
        "error must not leak the secret key"
    );
}

/// A 403 response keeps its observed status in the field.
#[tokio::test]
async fn error_403_keeps_the_observed_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/speech"))
        .respond_with(ResponseTemplate::new(403).set_body_string(
            r#"{"__type":"AccessDeniedException","message":"User: arn:... is not authorized."}"#,
        ))
        .mount(&server)
        .await;

    let provider = AwsPollyProvider::new(test_config(&server));
    let model = provider.speech("aws_polly/neural");

    let err = model
        .do_generate(&speech_options("Hello"))
        .await
        .expect_err("expected an error for 403");

    assert!(
        matches!(err, ref e if e.status_code() == Some(403)),
        "expected a 403 provider error, got: {err:?}"
    );
}

/// A 404 response maps to `ApiCall` (404 in `status_code`).
#[tokio::test]
async fn error_404_maps_to_model_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/speech"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string(r#"{"__type":"NotFoundException","message":"Voice not found."}"#),
        )
        .mount(&server)
        .await;

    let provider = AwsPollyProvider::new(test_config(&server));
    let model = provider.speech("aws_polly/neural");

    let err = model
        .do_generate(&speech_options("Hello"))
        .await
        .expect_err("expected an error for 404");

    assert!(
        matches!(err, ref e if e.status_code() == Some(404)),
        "expected ModelNotFound error for 404, got: {err:?}"
    );
}

/// Unsupported output formats emit a warning and fall back to mp3.
#[tokio::test]
async fn unsupported_output_format_emits_warning() {
    let server = MockServer::start().await;
    mock_audio_response(&server, "mp3").await;

    let provider = AwsPollyProvider::new(test_config(&server));
    let model = provider.speech("aws_polly/neural");

    let mut options = speech_options("Hello");
    options.output_format = Some("wav".to_string());

    let result = model.do_generate(&options).await.unwrap();

    assert!(
        result
            .warnings
            .iter()
            .any(|w| matches!(w, aimux_core::types::Warning::Unsupported { feature, .. } if feature == "outputFormat")),
        "expected an unsupported outputFormat warning, got: {:?}",
        result.warnings
    );

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["OutputFormat"], "mp3", "should fall back to mp3");
}
