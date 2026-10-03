//! Rust translation of the Google Vertex video model tests.
//! Source: `reference/ai/packages/google-vertex/src/google-vertex-video-model.test.ts`

use aimux_core::video_model::{VideoCallOptions, generate_video};
use aimux_provider_utils::Resolvable;
use aimux_providers::{VertexProvider, VertexProviderSettings, create_google_vertex};

/// A standard-mode Vertex provider (project `test-project`, location
/// `us-central1`, bearer token `test-token`) pointed at `base_url`.
fn vertex_at(base_url: &str) -> VertexProvider {
    create_google_vertex(VertexProviderSettings {
        base_url: Some(base_url.to_string()),
        project: Some("test-project".to_string()),
        location: Some("us-central1".to_string()),
        access_token: Some(Resolvable::Value("test-token".to_string())),
        ..Default::default()
    })
    .expect("valid settings")
}

use serde_json::{Value, json};
use std::collections::HashMap;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fast_poll() -> Option<aimux_core::video_model::VideoPollOptions> {
    Some(aimux_core::video_model::VideoPollOptions {
        interval_ms: Some(1),
        timeout_ms: Some(10_000),
    })
}

fn options(p: &str) -> VideoCallOptions {
    let mut o = VideoCallOptions::new(p);
    o.poll = fast_poll();
    o
}

fn make_provider(server_uri: &str) -> VertexProvider {
    vertex_at(server_uri)
}

async fn mock_predict_and_poll(server: &MockServer, result: &Value) {
    Mock::given(method("POST"))
        .and(path("/models/veo-3.0-generate-001:predictLongRunning"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"name": "operations/test-op"})),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/operations/test-op"))
        .respond_with(ResponseTemplate::new(200).set_body_json(result.clone()))
        .mount(server)
        .await;
}

#[tokio::test]
async fn should_generate_video() {
    let server = MockServer::start().await;
    let result =
        json!({"done": true, "response": {"videos": [{"gcsUri": "gs://bucket/video.mp4"}]}});
    mock_predict_and_poll(&server, &result).await;
    let provider = make_provider(&server.uri());
    let model = provider.video("veo-3.0-generate-001");
    let r = generate_video(&model, options("A cat")).await.unwrap();
    assert_eq!(r.videos.len(), 1);
}

#[tokio::test]
async fn should_pass_prompt() {
    let server = MockServer::start().await;
    let result =
        json!({"done": true, "response": {"videos": [{"gcsUri": "gs://bucket/video.mp4"}]}});
    mock_predict_and_poll(&server, &result).await;
    let provider = make_provider(&server.uri());
    let model = provider.video("veo-3.0-generate-001");
    generate_video(&model, options("A cat")).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["instances"][0]["prompt"], "A cat");
}

#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    let result =
        json!({"done": true, "response": {"videos": [{"gcsUri": "gs://bucket/video.mp4"}]}});
    mock_predict_and_poll(&server, &result).await;
    let provider = make_provider(&server.uri());
    let model = provider.video("veo-3.0-generate-001");
    let mut opts = options("test");
    let mut rh = HashMap::new();
    rh.insert("Custom-Header".to_string(), "val".to_string());
    opts.headers = Some(rh);
    generate_video(&model, opts).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let h = &requests[0].headers;
    assert_eq!(h.get("authorization").unwrap(), "Bearer test-token");
    assert_eq!(h.get("custom-header").unwrap(), "val");
}

#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    let result =
        json!({"done": true, "response": {"videos": [{"gcsUri": "gs://bucket/video.mp4"}]}});
    mock_predict_and_poll(&server, &result).await;
    let provider = make_provider(&server.uri());
    let model = provider.video("veo-3.0-generate-001");
    let r = generate_video(&model, options("test")).await.unwrap();
    assert!(r.response.timestamp.is_some());
    assert_eq!(
        r.response.model_id,
        Some("veo-3.0-generate-001".to_string())
    );
}
