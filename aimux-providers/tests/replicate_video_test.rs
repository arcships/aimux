//! Rust translation of the Replicate video model tests.
//! Source: `reference/ai/packages/replicate/src/replicate-video-model.test.ts`

use aimux_core::video_model::{VideoCallOptions, VideoModel};
use aimux_providers::{ReplicateConfig, ReplicateProvider};
use serde_json::{Value, json};
use std::collections::HashMap;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn options(p: &str) -> VideoCallOptions {
    VideoCallOptions::new(p)
}

async fn mock_predict_and_result(server: &MockServer, output: &Value) {
    Mock::given(method("POST"))
        .and(path("/predictions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "test-id"})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/predictions/test-id"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"status": "succeeded", "output": output.clone()})),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn should_generate_video() {
    let server = MockServer::start().await;
    mock_predict_and_result(&server, &json!("https://cdn.replicate.com/video.mp4")).await;
    let config = ReplicateConfig::new("test-api-key").with_base_url(server.uri());
    let provider = ReplicateProvider::new(config);
    let model = provider.video("wan-lab/wan-2.1-t2v-14b");
    let r = model.do_generate(&options("A cat")).await.unwrap();
    assert_eq!(r.videos.len(), 1);
}

#[tokio::test]
async fn should_pass_model_and_prompt() {
    let server = MockServer::start().await;
    mock_predict_and_result(&server, &json!("https://cdn.replicate.com/video.mp4")).await;
    let config = ReplicateConfig::new("test-api-key").with_base_url(server.uri());
    let provider = ReplicateProvider::new(config);
    let model = provider.video("wan-lab/wan-2.1-t2v-14b");
    model.do_generate(&options("A cat")).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "wan-lab/wan-2.1-t2v-14b");
    assert_eq!(body["input"]["prompt"], "A cat");
}

#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_predict_and_result(&server, &json!("https://cdn.replicate.com/video.mp4")).await;
    let mut ph = HashMap::new();
    ph.insert("Custom-Header".to_string(), "val".to_string());
    let config = ReplicateConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_headers(ph);
    let provider = ReplicateProvider::new(config);
    let model = provider.video("test-model");
    let mut opts = options("test");
    let mut rh = HashMap::new();
    rh.insert("Custom-Request-Header".to_string(), "req-val".to_string());
    opts.headers = Some(rh);
    model.do_generate(&opts).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let h = &requests[0].headers;
    assert_eq!(h.get("authorization").unwrap(), "Token test-api-key");
    assert_eq!(h.get("custom-header").unwrap(), "val");
}

#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    mock_predict_and_result(&server, &json!("https://cdn.replicate.com/video.mp4")).await;
    let config = ReplicateConfig::new("test-api-key").with_base_url(server.uri());
    let provider = ReplicateProvider::new(config);
    let model = provider.video("test-model");
    let r = model.do_generate(&options("test")).await.unwrap();
    assert!(r.response.timestamp.is_some());
    assert_eq!(r.response.model_id, Some("test-model".to_string()));
}
