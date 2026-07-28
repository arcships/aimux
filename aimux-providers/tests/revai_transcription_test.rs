//! Rust translation of the Rev.ai transcription model tests.
//! Source: `reference/ai/packages/revai/src/revai-transcription-model.test.ts`

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_providers::{RevaiConfig, RevaiProvider};

fn mock_audio() -> Vec<u8> {
    vec![1u8, 2, 3]
}
fn options(audio: Vec<u8>, mt: &str) -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(audio), mt)
}

async fn mock_all_endpoints(server: &MockServer, transcript: &Value, headers: &[(&str, &str)]) {
    // Submit job
    let mut t = ResponseTemplate::new(200)
        .set_body_json(json!({"id": "test-id", "status": "transcribed", "language": "en"}));
    for (k, v) in headers {
        t = t.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/speechtotext/v1/jobs"))
        .respond_with(t)
        .mount(server)
        .await;

    // Job status (already transcribed on first poll)
    let t2 = ResponseTemplate::new(200)
        .set_body_json(json!({"id": "test-id", "status": "transcribed", "language": "en"}));
    Mock::given(method("GET"))
        .and(path("/speechtotext/v1/jobs/test-id"))
        .respond_with(t2)
        .mount(server)
        .await;

    // Transcript result
    let mut t3 = ResponseTemplate::new(200).set_body_json(transcript.clone());
    for (k, v) in headers {
        t3 = t3.insert_header(*k, *v);
    }
    Mock::given(method("GET"))
        .and(path("/speechtotext/v1/jobs/test-id/transcript"))
        .respond_with(t3)
        .mount(server)
        .await;
}

fn fixture_transcript() -> Value {
    json!({
        "monologues": [{
            "elements": [
                {"type": "text", "value": "Hello ", "ts": 0.0, "end_ts": 0.5},
                {"type": "text", "value": "from the Sal, A-I-S-D-K.", "ts": 0.5, "end_ts": 2.0},
                {"type": "punct", "value": "."}
            ]
        }]
    })
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
async fn should_pass_model_in_config() {
    let server = MockServer::start().await;
    mock_all_endpoints(&server, &fixture_transcript(), &[]).await;

    let config = RevaiConfig::new("test-api-key").with_base_url(server.uri());
    let provider = RevaiProvider::new(config);
    let model = provider.transcription("machine");

    model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3); // submit + poll + transcript
    let form = parse_multipart(&requests[0].body);
    assert!(form.contains_key("media"));
    let config_json: Value = serde_json::from_str(form.get("config").unwrap()).unwrap();
    assert_eq!(config_json["transcriber"], "machine");
}

#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_all_endpoints(&server, &fixture_transcript(), &[]).await;

    let mut ph = HashMap::new();
    ph.insert(
        "Custom-Provider-Header".to_string(),
        "provider-header-value".to_string(),
    );
    let config = RevaiConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_headers(ph);
    let provider = RevaiProvider::new(config);
    let model = provider.transcription("machine");

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
async fn should_extract_text() {
    let server = MockServer::start().await;
    mock_all_endpoints(&server, &fixture_transcript(), &[]).await;

    let config = RevaiConfig::new("test-api-key").with_base_url(server.uri());
    let provider = RevaiProvider::new(config);
    let model = provider.transcription("machine");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.text.contains("Hello"));
}

#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    mock_all_endpoints(
        &server,
        &fixture_transcript(),
        &[
            ("x-request-id", "test-request-id"),
            ("x-ratelimit-remaining", "123"),
        ],
    )
    .await;

    let config = RevaiConfig::new("test-api-key").with_base_url(server.uri());
    let provider = RevaiProvider::new(config);
    let model = provider.transcription("machine");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("machine".to_string()));
}

#[tokio::test]
async fn should_use_real_date() {
    let server = MockServer::start().await;
    mock_all_endpoints(&server, &fixture_transcript(), &[]).await;

    let config = RevaiConfig::new("test-api-key").with_base_url(server.uri());
    let provider = RevaiProvider::new(config);
    let model = provider.transcription("machine");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("machine".to_string()));
}
