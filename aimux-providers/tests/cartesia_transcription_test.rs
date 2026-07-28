//! Rust translation of the Cartesia transcription model tests.
//! Source: `reference/ai/packages/cartesia/src/cartesia-transcription-model.test.ts`

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::error::AiMuxError;
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_providers::{CartesiaConfig, CartesiaProvider};

fn mock_audio() -> Vec<u8> {
    vec![1u8, 2, 3]
}
fn options(audio: Vec<u8>, mt: &str) -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(audio), mt)
}

fn fixture_response() -> Value {
    json!({
        "text": "Hello from Cartesia.",
        "language": "en",
        "duration": 3.5,
        "words": [
            {"word": "Hello", "start": 0.0, "end": 1.0},
            {"word": " from Cartesia.", "start": 1.0, "end": 3.5}
        ]
    })
}

async fn mock_response(server: &MockServer, body: &Value, headers: &[(&str, &str)]) {
    let mut t = ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_json(body.clone());
    for (k, v) in headers {
        t = t.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/stt"))
        .respond_with(t)
        .mount(server)
        .await;
}

fn parse_multipart(body: &[u8]) -> HashMap<String, String> {
    let s = String::from_utf8_lossy(body);
    let mut form = HashMap::new();
    let boundary = s.lines().next().unwrap_or("").trim_start_matches('-');
    if boundary.is_empty() {
        return form;
    }
    for part in s.split(boundary) {
        let part = part.trim_matches(|c| c == '\r' || c == '\n' || c == '-');
        if part.is_empty() || part == "--" {
            continue;
        }
        if let Some(hd) = part.find("\r\n\r\n") {
            let headers = &part[..hd];
            let value = &part[hd + 4..];
            if let Some(ns) = headers.find("name=\"") {
                let ns = ns + 6;
                if let Some(ne) = headers[ns..].find('"') {
                    let name = &headers[ns..ns + ne];
                    form.insert(name.to_string(), value.trim_end_matches('\r').to_string());
                }
            }
        }
    }
    form
}

#[tokio::test]
async fn should_pass_model() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response(), &[]).await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.transcription("best");

    model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let form = parse_multipart(&requests[0].body);
    assert_eq!(form.get("model").unwrap(), "best");
    assert!(form.contains_key("file"));
}

#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response(), &[]).await;

    let mut ph = HashMap::new();
    ph.insert(
        "Custom-Provider-Header".to_string(),
        "provider-header-value".to_string(),
    );
    let config = CartesiaConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_headers(ph);
    let provider = CartesiaProvider::new(config);
    let model = provider.transcription("best");

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

#[tokio::test]
async fn should_extract_text() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response(), &[]).await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.transcription("best");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.text, "Hello from Cartesia.");
    assert_eq!(result.language, Some("en".to_string()));
    assert_eq!(result.duration_in_seconds, Some(3.5));
    assert_eq!(result.segments.len(), 2);
}

#[tokio::test]
async fn should_reject_streaming_model_for_non_streaming() {
    let server = MockServer::start().await;
    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.transcription("ink-2");

    let result = model.do_generate(&options(mock_audio(), "audio/wav")).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        AiMuxError::Unsupported(msg) => assert!(msg.contains("ink-2")),
        e => panic!("expected Unsupported, got: {e:?}"),
    }
}

#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    mock_response(
        &server,
        &fixture_response(),
        &[("x-request-id", "test-req")],
    )
    .await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.transcription("best");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("best".to_string()));
}

#[tokio::test]
async fn should_use_real_date() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response(), &[]).await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.transcription("best");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("best".to_string()));
}

#[tokio::test]
async fn should_warn_about_streaming_options() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response(), &[]).await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.transcription("best");

    let mut opts = options(mock_audio(), "audio/wav");
    let mut po = HashMap::new();
    po.insert(
        "cartesia".to_string(),
        json!({"streaming": {"delay": "low"}}),
    );
    opts.provider_options = Some(po);

    let result = model.do_generate(&opts).await.unwrap();

    assert_eq!(result.warnings.len(), 1);
    match &result.warnings[0] {
        aimux_core::types::Warning::Unsupported { feature, .. } => {
            assert_eq!(feature, "providerOptions.cartesia.streaming");
        }
        w => panic!("expected Unsupported warning, got: {w:?}"),
    }
}

#[tokio::test]
async fn should_pass_language_and_timestamp_granularities() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response(), &[]).await;

    let config = CartesiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CartesiaProvider::new(config);
    let model = provider.transcription("best");

    let mut opts = options(mock_audio(), "audio/wav");
    let mut po = HashMap::new();
    po.insert(
        "cartesia".to_string(),
        json!({"language": "en", "timestampGranularities": ["word"]}),
    );
    opts.provider_options = Some(po);

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let form = parse_multipart(&requests[0].body);
    assert_eq!(form.get("language").unwrap(), "en");
    assert_eq!(form.get("timestamp_granularities[]").unwrap(), "word");
}
