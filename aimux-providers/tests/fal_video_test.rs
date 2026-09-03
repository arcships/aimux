//! Rust translation of the Fal video model tests.
//! Source: `reference/ai/packages/fal/src/fal-video-model.test.ts`

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::video_model::{VideoCallOptions, generate_video};
use aimux_providers::{FalConfig, FalProvider};

fn fast_poll() -> Option<aimux_core::video_model::VideoPollOptions> {
    Some(aimux_core::video_model::VideoPollOptions {
        interval_ms: Some(1),
        timeout_ms: Some(10_000),
    })
}

fn options(prompt: &str) -> VideoCallOptions {
    let mut o = VideoCallOptions::new(prompt);
    o.poll = fast_poll();
    o
}

async fn mock_queue_and_result(server: &MockServer, result: &Value) {
    Mock::given(method("POST"))
        .and(path("/fal-ai/kling-video"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"request_id": "test-id"})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/fal-ai/kling-video/requests/test-id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(result.clone()))
        .mount(server)
        .await;
}

#[tokio::test]
async fn should_generate_video() {
    let server = MockServer::start().await;
    let result = json!({"video": {"url": "https://cdn.fal.run/video.mp4"}});
    mock_queue_and_result(&server, &result).await;

    let config = FalConfig::new("test-api-key").with_base_url(server.uri());
    let provider = FalProvider::new(config);
    let model = provider.video("fal-ai/kling-video");

    let r = generate_video(&model, options("A cat playing"))
        .await
        .unwrap();
    assert_eq!(r.videos.len(), 1);
}

#[tokio::test]
async fn should_pass_prompt() {
    let server = MockServer::start().await;
    let result = json!({"video": {"url": "https://cdn.fal.run/video.mp4"}});
    mock_queue_and_result(&server, &result).await;

    let config = FalConfig::new("test-api-key").with_base_url(server.uri());
    let provider = FalProvider::new(config);
    let model = provider.video("fal-ai/kling-video");

    generate_video(&model, options("A cat playing"))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["prompt"], "A cat playing");
}

#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    let result = json!({"video": {"url": "https://cdn.fal.run/video.mp4"}});
    mock_queue_and_result(&server, &result).await;

    let mut ph = HashMap::new();
    ph.insert("Custom-Header".to_string(), "val".to_string());
    let config = FalConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_headers(ph);
    let provider = FalProvider::new(config);
    let model = provider.video("fal-ai/kling-video");

    let mut opts = options("test");
    let mut rh = HashMap::new();
    rh.insert("Custom-Request-Header".to_string(), "req-val".to_string());
    opts.headers = Some(rh);

    generate_video(&model, opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let h = &requests[0].headers;
    assert_eq!(h.get("authorization").unwrap(), "Key test-api-key");
    assert_eq!(h.get("custom-header").unwrap(), "val");
    assert_eq!(h.get("custom-request-header").unwrap(), "req-val");
}

#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    let result = json!({"video": {"url": "https://cdn.fal.run/video.mp4"}});
    mock_queue_and_result(&server, &result).await;

    let config = FalConfig::new("test-api-key").with_base_url(server.uri());
    let provider = FalProvider::new(config);
    let model = provider.video("fal-ai/kling-video");

    let r = generate_video(&model, options("test")).await.unwrap();
    assert!(r.response.timestamp.is_some());
    assert_eq!(r.response.model_id, Some("fal-ai/kling-video".to_string()));
}
