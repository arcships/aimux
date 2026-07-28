//! Rust translation of the Google Vertex transcription model tests.
//! Source: `reference/ai/packages/google-vertex/src/google-vertex-transcription-model.test.ts`

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_providers::{VertexProvider, VertexProviderConfig};

fn mock_audio() -> Vec<u8> {
    vec![1u8, 2, 3, 4, 5, 6, 7, 8]
}
fn options(audio: Vec<u8>, mt: &str) -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(audio), mt)
}

fn default_body() -> Value {
    json!({
        "results": [{
            "alternatives": [{
                "transcript": "hello world",
                "words": [
                    {"word": "hello", "startOffset": "0s", "endOffset": "0.500s"},
                    {"word": "world", "startOffset": "0.500s", "endOffset": "1s"}
                ]
            }],
            "languageCode": "en-US"
        }],
        "metadata": {"totalBilledDuration": "1s"}
    })
}

fn make_provider(server_uri: &str) -> VertexProvider {
    let config = VertexProviderConfig::new("test-token", "test-project", "us-central1")
        .with_base_url(server_uri);
    VertexProvider::new(config)
}

async fn mock_response(server: &MockServer, body: &Value, headers: &[(&str, &str)]) {
    let path_str = "/v2/projects/test-project/locations/us-central1/recognizers/_:recognize";
    let mut t = ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_json(body.clone());
    for (k, v) in headers {
        t = t.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path(path_str))
        .respond_with(t)
        .mount(server)
        .await;
}

#[tokio::test]
async fn should_send_model_language_codes_features_and_base64() {
    let server = MockServer::start().await;
    mock_response(&server, &default_body(), &[]).await;

    let provider = make_provider(&server.uri());
    let model = provider.transcription("chirp_2").unwrap();

    model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["config"]["model"], "chirp_2");
    assert_eq!(body["config"]["languageCodes"], json!(["auto"]));
    assert_eq!(body["config"]["autoDecodingConfig"], json!({}));
    assert_eq!(body["config"]["features"]["enableWordTimeOffsets"], true);
    assert_eq!(
        body["config"]["features"]["enableAutomaticPunctuation"],
        true
    );
    assert_eq!(body["content"], "AQIDBAUGBwg=");
}

#[tokio::test]
async fn should_extract_text_segments_language_and_duration() {
    let server = MockServer::start().await;
    mock_response(&server, &default_body(), &[]).await;

    let provider = make_provider(&server.uri());
    let model = provider.transcription("chirp_2").unwrap();

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.text, "hello world");
    assert_eq!(result.segments.len(), 2);
    assert_eq!(result.segments[0].text, "hello");
    assert_eq!(result.segments[0].start_second, 0.0);
    assert_eq!(result.segments[0].end_second, 0.5);
    assert_eq!(result.segments[1].text, "world");
    assert_eq!(result.segments[1].start_second, 0.5);
    assert_eq!(result.segments[1].end_second, 1.0);
    assert_eq!(result.language, Some("en".to_string()));
    assert_eq!(result.duration_in_seconds, Some(1.0));
}

#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_response(&server, &default_body(), &[]).await;

    let config = VertexProviderConfig::new("test-token", "test-project", "us-central1")
        .with_base_url(server.uri());
    let provider = VertexProvider::new(config);
    let model = provider.transcription("chirp_2").unwrap();

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
    assert_eq!(h.get("authorization").unwrap(), "Bearer test-token");
    assert_eq!(
        h.get("custom-request-header").unwrap(),
        "request-header-value"
    );
}

#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    mock_response(&server, &default_body(), &[("x-request-id", "test-req")]).await;

    let provider = make_provider(&server.uri());
    let model = provider.transcription("chirp_2").unwrap();

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("chirp_2".to_string()));
}

#[tokio::test]
async fn should_use_real_date() {
    let server = MockServer::start().await;
    mock_response(&server, &default_body(), &[]).await;

    let provider = make_provider(&server.uri());
    let model = provider.transcription("chirp_2").unwrap();

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("chirp_2".to_string()));
}

#[tokio::test]
async fn should_handle_no_results() {
    let server = MockServer::start().await;
    mock_response(&server, &json!({}), &[]).await;

    let provider = make_provider(&server.uri());
    let model = provider.transcription("chirp_2").unwrap();

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.text, "");
    assert!(result.segments.is_empty());
    assert_eq!(result.language, None);
    assert_eq!(result.duration_in_seconds, None);
}

#[tokio::test]
async fn should_use_provider_options_for_region_and_language() {
    let server = MockServer::start().await;
    // Different region → different path
    let path_str = "/v2/projects/test-project/locations/us/recognizers/_:recognize";
    Mock::given(method("POST"))
        .and(path(path_str))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(default_body()),
        )
        .mount(&server)
        .await;

    let config = VertexProviderConfig::new("test-token", "test-project", "us-central1")
        .with_base_url(server.uri());
    let provider = VertexProvider::new(config);
    let model = provider.transcription("chirp_2").unwrap();

    let mut opts = options(mock_audio(), "audio/wav");
    let mut po = HashMap::new();
    po.insert(
        "googleVertex".to_string(),
        json!({"region": "us", "languageCodes": ["en", "es"]}),
    );
    opts.provider_options = Some(po);

    let result = model.do_generate(&opts).await.unwrap();

    assert_eq!(result.text, "hello world");
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["config"]["languageCodes"], json!(["en", "es"]));
}
