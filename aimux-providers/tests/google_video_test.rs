//! Rust translation of the Google video model tests.
//! Source: `reference/ai/packages/google/src/google-video-model.test.ts`

use aimux_core::video_model::{VideoCallOptions, VideoModel};
use aimux_providers::{GoogleConfig, GoogleProvider};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn options(p: &str) -> VideoCallOptions {
    VideoCallOptions::new(p)
}

async fn mock_predict_and_poll(server: &MockServer, result: &Value) {
    // predictLongRunning
    Mock::given(method("POST"))
        .and(path("/models/veo-3.0-generate-001:predictLongRunning"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"name": "operations/test-op"})),
        )
        .mount(server)
        .await;
    // poll operation
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
    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.video("veo-3.0-generate-001");
    let r = model.do_generate(&options("A cat")).await.unwrap();
    assert_eq!(r.videos.len(), 1);
}

#[tokio::test]
async fn should_pass_prompt() {
    let server = MockServer::start().await;
    let result =
        json!({"done": true, "response": {"videos": [{"gcsUri": "gs://bucket/video.mp4"}]}});
    mock_predict_and_poll(&server, &result).await;
    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.video("veo-3.0-generate-001");
    model.do_generate(&options("A cat")).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["instances"][0]["prompt"], "A cat");
}

#[tokio::test]
async fn should_pass_aspect_ratio_and_duration() {
    let server = MockServer::start().await;
    let result =
        json!({"done": true, "response": {"videos": [{"gcsUri": "gs://bucket/video.mp4"}]}});
    mock_predict_and_poll(&server, &result).await;
    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.video("veo-3.0-generate-001");
    let mut opts = options("test");
    opts.aspect_ratio = Some(aimux_core::shared::AspectRatio::new(16, 9));
    opts.duration = Some(5);
    model.do_generate(&opts).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["parameters"]["aspectRatio"], "16:9");
    assert_eq!(body["parameters"]["durationSeconds"], 5);
}

#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    let result =
        json!({"done": true, "response": {"videos": [{"gcsUri": "gs://bucket/video.mp4"}]}});
    mock_predict_and_poll(&server, &result).await;
    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.video("veo-3.0-generate-001");
    let r = model.do_generate(&options("test")).await.unwrap();
    assert!(r.response.timestamp.is_some());
    assert_eq!(
        r.response.model_id,
        Some("veo-3.0-generate-001".to_string())
    );
}
