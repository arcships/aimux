//! Rust translation of the Deepgram transcription model tests.
//!
//! Source: `reference/ai/packages/deepgram/src/deepgram-transcription-model.test.ts`

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_providers::{DeepgramConfig, DeepgramProvider};

fn mock_audio() -> Vec<u8> {
    vec![1u8, 2, 3, 4, 5]
}

fn options(audio: Vec<u8>, media_type: &str) -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(audio), media_type)
}

fn fixture_response() -> Value {
    json!({
        "metadata": {"duration": 36.744},
        "results": {
            "channels": [{
                "detected_language": "en",
                "alternatives": [{
                    "transcript": "galileo was an american robotic space program",
                    "words": [
                        {"word": "galileo", "start": 0.16, "end": 0.8},
                        {"word": "was", "start": 0.8, "end": 0.96}
                    ]
                }]
            }]
        }
    })
}

async fn mock_response(server: &MockServer, body: &Value) {
    Mock::given(method("POST"))
        .and(path("/v1/listen"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(body.clone()),
        )
        .mount(server)
        .await;
}

async fn mock_response_with_headers(server: &MockServer, body: &Value, headers: &[(&str, &str)]) {
    let mut t = ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_json(body.clone());
    for (k, v) in headers {
        t = t.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/v1/listen"))
        .respond_with(t)
        .mount(server)
        .await;
}

#[tokio::test]
async fn should_pass_model_in_query() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response()).await;

    let config = DeepgramConfig::new("test-api-key").with_base_url(server.uri());
    let provider = DeepgramProvider::new(config);
    let model = provider.transcription("nova-3");

    model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    // Model is passed as a query parameter.
    assert!(requests[0].url.as_str().contains("model=nova-3"));
}

#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response()).await;

    let mut ph = HashMap::new();
    ph.insert(
        "Custom-Provider-Header".to_string(),
        "provider-header-value".to_string(),
    );
    let config = DeepgramConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_headers(ph);
    let provider = DeepgramProvider::new(config);
    let model = provider.transcription("nova-3");

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
    assert_eq!(h.get("authorization").unwrap(), "Token test-api-key");
    assert_eq!(h.get("content-type").unwrap(), "audio/wav");
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
    mock_response(&server, &fixture_response()).await;

    let config = DeepgramConfig::new("test-api-key").with_base_url(server.uri());
    let provider = DeepgramProvider::new(config);
    let model = provider.transcription("nova-3");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.text, "galileo was an american robotic space program");
}

#[tokio::test]
async fn should_pass_detect_language_query_param() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response()).await;

    let config = DeepgramConfig::new("test-api-key").with_base_url(server.uri());
    let provider = DeepgramProvider::new(config);
    let model = provider.transcription("nova-3");

    let mut opts = options(mock_audio(), "audio/wav");
    let mut po = HashMap::new();
    po.insert("deepgram".to_string(), json!({"detectLanguage": true}));
    opts.provider_options = Some(po);

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert!(requests[0].url.as_str().contains("detect_language=true"));
}

#[tokio::test]
async fn should_return_detected_language() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response()).await;

    let config = DeepgramConfig::new("test-api-key").with_base_url(server.uri());
    let provider = DeepgramProvider::new(config);
    let model = provider.transcription("nova-3");

    let mut opts = options(mock_audio(), "audio/wav");
    let mut po = HashMap::new();
    po.insert("deepgram".to_string(), json!({"detectLanguage": true}));
    opts.provider_options = Some(po);

    let result = model.do_generate(&opts).await.unwrap();

    assert_eq!(result.language, Some("en".to_string()));
}

#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    mock_response_with_headers(
        &server,
        &fixture_response(),
        &[
            ("x-request-id", "test-request-id"),
            ("x-ratelimit-remaining", "123"),
        ],
    )
    .await;

    let config = DeepgramConfig::new("test-api-key").with_base_url(server.uri());
    let provider = DeepgramProvider::new(config);
    let model = provider.transcription("nova-3");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("nova-3".to_string()));
    let headers = result.response.headers.as_ref().unwrap();
    assert_eq!(headers.get("x-request-id").unwrap(), "test-request-id");
    assert_eq!(headers.get("x-ratelimit-remaining").unwrap(), "123");
}

#[tokio::test]
async fn should_use_real_date() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response()).await;

    let config = DeepgramConfig::new("test-api-key").with_base_url(server.uri());
    let provider = DeepgramProvider::new(config);
    let model = provider.transcription("nova-3");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("nova-3".to_string()));
}

#[tokio::test]
async fn should_return_language_from_inline_response() {
    let server = MockServer::start().await;
    mock_response(
        &server,
        &json!({
            "metadata": {"duration": 1.0},
            "results": {
                "channels": [{
                    "detected_language": "sv",
                    "alternatives": [{"transcript": "hej", "words": []}]
                }]
            }
        }),
    )
    .await;

    let config = DeepgramConfig::new("test-api-key").with_base_url(server.uri());
    let provider = DeepgramProvider::new(config);
    let model = provider.transcription("nova-3");

    let mut opts = options(mock_audio(), "audio/wav");
    let mut po = HashMap::new();
    po.insert("deepgram".to_string(), json!({"detectLanguage": true}));
    opts.provider_options = Some(po);

    let result = model.do_generate(&opts).await.unwrap();

    assert_eq!(result.language, Some("sv".to_string()));
}

#[tokio::test]
async fn should_return_none_language_when_not_detected() {
    let server = MockServer::start().await;
    mock_response(
        &server,
        &json!({
            "metadata": {"duration": 1.0},
            "results": {
                "channels": [{
                    "alternatives": [{"transcript": "hello", "words": []}]
                }]
            }
        }),
    )
    .await;

    let config = DeepgramConfig::new("test-api-key").with_base_url(server.uri());
    let provider = DeepgramProvider::new(config);
    let model = provider.transcription("nova-3");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.language, None);
}
