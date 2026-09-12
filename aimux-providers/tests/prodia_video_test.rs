//! Rust translation of the Prodia video model tests.
//! Source: `reference/ai/packages/prodia/src/prodia-video-model.test.ts`

use aimux_core::video_model::{VideoCallOptions, generate_video};
use aimux_providers::{ProdiaConfig, ProdiaProvider};
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

async fn mock_job_and_result(server: &MockServer, result: &Value) {
    Mock::given(method("POST"))
        .and(path("/job"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"job": "test-job-id"})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/job/test-job-id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(result.clone()))
        .mount(server)
        .await;
}

#[tokio::test]
async fn should_generate_video() {
    let server = MockServer::start().await;
    mock_job_and_result(
        &server,
        &json!({"status": "done", "videoUrl": "https://cdn.prodia.com/video.mp4"}),
    )
    .await;
    let config = ProdiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = ProdiaProvider::new(config);
    let model = provider.video("svd-xt");
    let r = generate_video(&model, options("A cat")).await.unwrap();
    assert_eq!(r.videos.len(), 1);
}

#[tokio::test]
async fn should_pass_model_type_and_prompt() {
    let server = MockServer::start().await;
    mock_job_and_result(
        &server,
        &json!({"status": "done", "videoUrl": "https://cdn.prodia.com/video.mp4"}),
    )
    .await;
    let config = ProdiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = ProdiaProvider::new(config);
    let model = provider.video("svd-xt");
    generate_video(&model, options("A cat")).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["type"], "svd-xt");
    assert_eq!(body["config"]["prompt"], "A cat");
}

#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_job_and_result(
        &server,
        &json!({"status": "done", "videoUrl": "https://cdn.prodia.com/video.mp4"}),
    )
    .await;
    let mut ph = HashMap::new();
    ph.insert("Custom-Header".to_string(), "val".to_string());
    let config = ProdiaConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_headers(ph);
    let provider = ProdiaProvider::new(config);
    let model = provider.video("svd-xt");
    let mut opts = options("test");
    let mut rh = HashMap::new();
    rh.insert("Custom-Request-Header".to_string(), "req-val".to_string());
    opts.headers = Some(rh);
    generate_video(&model, opts).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let h = &requests[0].headers;
    assert_eq!(h.get("x-prodia-key").unwrap(), "test-api-key");
    assert_eq!(h.get("custom-header").unwrap(), "val");
}

#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    mock_job_and_result(
        &server,
        &json!({"status": "done", "videoUrl": "https://cdn.prodia.com/video.mp4"}),
    )
    .await;
    let config = ProdiaConfig::new("test-api-key").with_base_url(server.uri());
    let provider = ProdiaProvider::new(config);
    let model = provider.video("svd-xt");
    let r = generate_video(&model, options("test")).await.unwrap();
    assert!(r.response.timestamp.is_some());
    assert_eq!(r.response.model_id, Some("svd-xt".to_string()));
}
