//! Rust translation of the Fal transcription model tests.
//! Source: `reference/ai/packages/fal/src/fal-transcription-model.test.ts`

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_providers::{FalConfig, FalProvider};

fn mock_audio() -> Vec<u8> {
    vec![1u8, 2, 3]
}
fn options(audio: Vec<u8>, mt: &str) -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(audio), mt)
}

async fn mock_queue_and_result(server: &MockServer, result_body: &Value, headers: &[(&str, &str)]) {
    // Submit endpoint
    let mut submit_t = ResponseTemplate::new(200).set_body_json(json!({"request_id": "test-id"}));
    for (k, v) in headers {
        submit_t = submit_t.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/fal-ai/wizper"))
        .respond_with(submit_t)
        .mount(server)
        .await;

    // Poll endpoint — return result on first try
    let mut result_t = ResponseTemplate::new(200).set_body_json(result_body.clone());
    for (k, v) in headers {
        result_t = result_t.insert_header(*k, *v);
    }
    Mock::given(method("GET"))
        .and(path("/fal-ai/wizper/requests/test-id"))
        .respond_with(result_t)
        .mount(server)
        .await;
}

fn fixture_result() -> Value {
    json!({
        "text": "Hello from the Versal AISDK.",
        "chunks": [
            {"text": "Hello", "timestamp": [0.0, 1.0]},
            {"text": " from the Versal AISDK.", "timestamp": [1.0, 3.5]}
        ],
        "inferred_languages": ["en"]
    })
}

#[tokio::test]
async fn should_pass_model_and_audio() {
    let server = MockServer::start().await;
    mock_queue_and_result(&server, &fixture_result(), &[]).await;

    let config = FalConfig::new("test-api-key").with_base_url(server.uri());
    let provider = FalProvider::new(config);
    let model = provider.transcription("wizper");

    model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(
        body["audio_url"]
            .as_str()
            .unwrap()
            .starts_with("data:audio/")
    );
    assert_eq!(body["task"], "transcribe");
    assert_eq!(body["diarize"], true);
    assert_eq!(body["chunk_level"], "word");
}

#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_queue_and_result(&server, &fixture_result(), &[]).await;

    let mut ph = HashMap::new();
    ph.insert(
        "Custom-Provider-Header".to_string(),
        "provider-header-value".to_string(),
    );
    let config = FalConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_headers(ph);
    let provider = FalProvider::new(config);
    let model = provider.transcription("wizper");

    let mut opts = options(mock_audio(), "audio/wav");
    let mut rh = HashMap::new();
    rh.insert(
        "Custom-Request-Header".to_string(),
        "request-header-value".to_string(),
    );
    opts.headers = Some(rh);

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let h = &requests[0].headers;
    assert_eq!(h.get("authorization").unwrap(), "Key test-api-key");
    assert_eq!(h.get("content-type").unwrap(), "application/json");
    assert_eq!(
        h.get("custom-provider-header").unwrap(),
        "provider-header-value"
    );
    assert_eq!(
        h.get("custom-request-header").unwrap(),
        "request-header-value"
    );
}

#[tokio::test]
async fn should_extract_text() {
    let server = MockServer::start().await;
    mock_queue_and_result(&server, &fixture_result(), &[]).await;

    let config = FalConfig::new("test-api-key").with_base_url(server.uri());
    let provider = FalProvider::new(config);
    let model = provider.transcription("wizper");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.text, "Hello from the Versal AISDK.");
}

#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    mock_queue_and_result(
        &server,
        &fixture_result(),
        &[
            ("x-request-id", "test-request-id"),
            ("x-ratelimit-remaining", "123"),
        ],
    )
    .await;

    let config = FalConfig::new("test-api-key").with_base_url(server.uri());
    let provider = FalProvider::new(config);
    let model = provider.transcription("wizper");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("wizper".to_string()));
}

#[tokio::test]
async fn should_use_real_date() {
    let server = MockServer::start().await;
    mock_queue_and_result(&server, &fixture_result(), &[]).await;

    let config = FalConfig::new("test-api-key").with_base_url(server.uri());
    let provider = FalProvider::new(config);
    let model = provider.transcription("wizper");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("wizper".to_string()));
}
