//! Rust translation of the Gladia transcription model tests.
//! Source: `reference/ai/packages/gladia/src/gladia-transcription-model.test.ts`

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_providers::{GladiaConfig, GladiaProvider};

fn mock_audio() -> Vec<u8> {
    vec![1u8, 2, 3]
}
fn options(audio: Vec<u8>, mt: &str) -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(audio), mt)
}

async fn mock_all(server: &MockServer, result: &Value, headers: &[(&str, &str)]) {
    // Upload
    Mock::given(method("POST"))
        .and(path("/v2/upload"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"audio_url": "https://upload.gladia.io/test"})),
        )
        .mount(server)
        .await;
    // Initiate
    Mock::given(method("POST"))
        .and(path("/v2/pre-recorded"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"result_url": format!("{}/v2/result/test", server.uri())})),
        )
        .mount(server)
        .await;
    // Poll result
    let mut t = ResponseTemplate::new(200).set_body_json(result.clone());
    for (k, v) in headers {
        t = t.insert_header(*k, *v);
    }
    Mock::given(method("GET"))
        .and(path("/v2/result/test"))
        .respond_with(t)
        .mount(server)
        .await;
}

fn fixture_result() -> Value {
    json!({
        "status": "done",
        "result": {
            "metadata": {"audio_duration": 5.0},
            "transcription": {
                "full_transcript": "Hello from Gladia.",
                "languages": ["en"],
                "utterances": [
                    {"text": "Hello", "start": 0.0, "end": 1.0},
                    {"text": " from Gladia.", "start": 1.0, "end": 2.0}
                ]
            }
        }
    })
}

#[tokio::test]
async fn should_extract_text() {
    let server = MockServer::start().await;
    mock_all(&server, &fixture_result(), &[]).await;

    let config = GladiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GladiaProvider::new(config);
    let model = provider.transcription("default");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.text, "Hello from Gladia.");
    assert_eq!(result.language, Some("en".to_string()));
    assert_eq!(result.duration_in_seconds, Some(5.0));
    assert_eq!(result.segments.len(), 2);
}

#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_all(&server, &fixture_result(), &[]).await;

    let mut ph = HashMap::new();
    ph.insert(
        "Custom-Provider-Header".to_string(),
        "provider-header-value".to_string(),
    );
    let config = GladiaConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_headers(ph);
    let provider = GladiaProvider::new(config);
    let model = provider.transcription("default");

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
    assert_eq!(h.get("authorization").unwrap(), "Bearer test-api-key");
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
async fn should_include_response_data() {
    let server = MockServer::start().await;
    mock_all(&server, &fixture_result(), &[("x-request-id", "test-req")]).await;

    let config = GladiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GladiaProvider::new(config);
    let model = provider.transcription("default");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("default".to_string()));
}

#[tokio::test]
async fn should_use_real_date() {
    let server = MockServer::start().await;
    mock_all(&server, &fixture_result(), &[]).await;

    let config = GladiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GladiaProvider::new(config);
    let model = provider.transcription("default");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("default".to_string()));
}

#[tokio::test]
async fn should_include_provider_metadata() {
    let server = MockServer::start().await;
    mock_all(&server, &fixture_result(), &[]).await;

    let config = GladiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GladiaProvider::new(config);
    let model = provider.transcription("default");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.provider_metadata.is_some());
    assert!(
        result
            .provider_metadata
            .as_ref()
            .unwrap()
            .contains_key("gladia")
    );
}
