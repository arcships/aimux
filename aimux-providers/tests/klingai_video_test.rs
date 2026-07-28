//! Rust translation of the KlingAI video model tests.
//! Source: `reference/ai/packages/klingai/src/klingai-video-model.test.ts`

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::video_model::{VideoCallOptions, VideoModel};
use aimux_providers::{KlingAIConfig, KlingAIProvider};

fn options(prompt: &str) -> VideoCallOptions {
    VideoCallOptions::new(prompt)
}

async fn mock_task_and_result(
    server: &MockServer,
    result_videos: &[Value],
    headers: &[(&str, &str)],
) {
    // Submit task
    let mut t = ResponseTemplate::new(200).set_body_json(json!({
        "code": 0,
        "data": {"task_id": "task-123", "id": "job-456"},
        "message": "success"
    }));
    for (k, v) in headers {
        t = t.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/v1/videos/text2video"))
        .respond_with(t)
        .mount(server)
        .await;

    // Poll result (succeeded on first poll)
    let mut t2 = ResponseTemplate::new(200).set_body_json(json!({
        "code": 0,
        "data": {
            "task_status": "succeeded",
            "task_result": {"videos": result_videos}
        }
    }));
    for (k, v) in headers {
        t2 = t2.insert_header(*k, *v);
    }
    Mock::given(method("GET"))
        .and(path("/v1/videos/text2video/job-456/task-123"))
        .respond_with(t2)
        .mount(server)
        .await;
}

#[tokio::test]
async fn should_generate_video_from_prompt() {
    let server = MockServer::start().await;
    let videos = vec![json!({"url": "https://cdn.klingai.com/video.mp4"})];
    mock_task_and_result(&server, &videos, &[]).await;

    let config = KlingAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = KlingAIProvider::new(config);
    let model = provider.video("kling-v2.1-master-t2v");

    let result = model.do_generate(&options("A cat playing")).await.unwrap();

    assert_eq!(result.videos.len(), 1);
    match &result.videos[0] {
        aimux_core::video_model::VideoData::Url { url, .. } => {
            assert_eq!(url, "https://cdn.klingai.com/video.mp4");
        }
        v => panic!("expected Url, got: {v:?}"),
    }
}

#[tokio::test]
async fn should_pass_model_name_and_prompt() {
    let server = MockServer::start().await;
    let videos = vec![json!({"url": "https://cdn.klingai.com/video.mp4"})];
    mock_task_and_result(&server, &videos, &[]).await;

    let config = KlingAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = KlingAIProvider::new(config);
    let model = provider.video("kling-v2.1-master-t2v");

    model.do_generate(&options("A cat playing")).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model_name"], "kling-v2-1-master");
    assert_eq!(body["prompt"], "A cat playing");
}

#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    let videos = vec![json!({"url": "https://cdn.klingai.com/video.mp4"})];
    mock_task_and_result(&server, &videos, &[]).await;

    let mut ph = HashMap::new();
    ph.insert("Custom-Provider-Header".to_string(), "val".to_string());
    let config = KlingAIConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_headers(ph);
    let provider = KlingAIProvider::new(config);
    let model = provider.video("kling-v2.1-master-t2v");

    let mut opts = options("test");
    let mut rh = HashMap::new();
    rh.insert("Custom-Request-Header".to_string(), "req-val".to_string());
    opts.headers = Some(rh);

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let h = &requests[0].headers;
    assert_eq!(h.get("authorization").unwrap(), "Bearer test-api-key");
    assert_eq!(h.get("custom-provider-header").unwrap(), "val");
    assert_eq!(h.get("custom-request-header").unwrap(), "req-val");
}

#[tokio::test]
async fn should_pass_seed_duration_and_aspect_ratio() {
    let server = MockServer::start().await;
    let videos = vec![json!({"url": "https://cdn.klingai.com/video.mp4"})];
    mock_task_and_result(&server, &videos, &[]).await;

    let config = KlingAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = KlingAIProvider::new(config);
    let model = provider.video("kling-v2.1-master-t2v");

    let mut opts = options("test");
    opts.seed = Some(42);
    opts.duration = Some(5);
    opts.aspect_ratio = Some(aimux_core::shared::AspectRatio::new(16, 9));

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["seed"], 42);
    assert_eq!(body["duration"], 5);
    assert_eq!(body["aspect_ratio"], "16:9");
}

#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    let videos = vec![json!({"url": "https://cdn.klingai.com/video.mp4"})];
    mock_task_and_result(&server, &videos, &[("x-request-id", "test-req")]).await;

    let config = KlingAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = KlingAIProvider::new(config);
    let model = provider.video("kling-v2.1-master-t2v");

    let result = model.do_generate(&options("test")).await.unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(
        result.response.model_id,
        Some("kling-v2.1-master-t2v".to_string())
    );
}

#[tokio::test]
async fn should_return_max_videos_per_call() {
    let config = KlingAIConfig::new("test-api-key");
    let provider = KlingAIProvider::new(config);
    let model = provider.video("kling-v2.1-master-t2v");
    assert_eq!(model.max_videos_per_call(), Some(1));
}
