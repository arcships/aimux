//! RunwayML video model tests.
//!
//! All tests run fully offline against a `wiremock` `MockServer` — no network
//! access and no real credentials are used. The polling tests mock a sequence
//! of task-status responses (PENDING → RUNNING → SUCCEEDED) to exercise the
//! async polling loop without a real delay.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::video_model::{VideoCallOptions, VideoModel, generate_video};
use aimux_providers::{RunwaymlConfig, RunwaymlProvider};

const MODEL_ID: &str = "gen3a_turbo";
const VIDEO_URL: &str = "https://cdn.runwayml.com/video.mp4";

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

/// A config pointed at the mock server with a short poll interval so tests run
/// fast. Uses a dummy, non-secret API key.
fn config(server_uri: String) -> RunwaymlConfig {
    RunwaymlConfig::new("test-api-key")
        .with_base_url(server_uri)
        .with_poll_interval(Duration::from_millis(10))
        .with_timeout(Duration::from_secs(10))
}

/// Mount a submit mock (returning task id `task-123`) and a polling sequence:
/// the first poll returns `PENDING`, the second `RUNNING`, then `SUCCEEDED`
/// carrying the generated video URL in `output`.
async fn mock_task_sequence(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/text_to_video"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "task-123"})))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/tasks/task-123"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": "task-123", "status": "PENDING"})),
        )
        .up_to_n_times(1)
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/tasks/task-123"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": "task-123", "status": "RUNNING"})),
        )
        .up_to_n_times(1)
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/tasks/task-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "task-123",
            "status": "SUCCEEDED",
            "output": [VIDEO_URL]
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn should_generate_video_from_prompt() {
    let server = MockServer::start().await;
    mock_task_sequence(&server).await;

    let provider = RunwaymlProvider::new(config(server.uri()));
    let model = provider.video(MODEL_ID);

    let result = generate_video(&model, options("A cat playing"))
        .await
        .unwrap();

    assert_eq!(result.videos.len(), 1);
    match &result.videos[0] {
        aimux_core::video_model::VideoData::Url { url, .. } => {
            assert_eq!(url, VIDEO_URL);
        }
        v => panic!("expected Url, got: {v:?}"),
    }
    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some(MODEL_ID.to_string()));
}

#[tokio::test]
async fn should_post_to_text_to_video_endpoint() {
    let server = MockServer::start().await;
    mock_task_sequence(&server).await;

    let provider = RunwaymlProvider::new(config(server.uri()));
    let model = provider.video(MODEL_ID);

    generate_video(&model, options("A cat playing"))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    // The first request is the task submission.
    assert_eq!(requests[0].method.as_str(), "POST");
    assert_eq!(requests[0].url.path(), "/v1/text_to_video");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], MODEL_ID);
    assert_eq!(body["promptText"], "A cat playing");
}

#[tokio::test]
async fn should_send_auth_and_version_headers() {
    let server = MockServer::start().await;
    mock_task_sequence(&server).await;

    let provider = RunwaymlProvider::new(config(server.uri()));
    let model = provider.video(MODEL_ID);

    generate_video(&model, options("test")).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    // Submit request carries the bearer token and the required version header.
    let submit_headers = &requests[0].headers;
    assert_eq!(
        submit_headers.get("authorization").unwrap(),
        "Bearer test-api-key"
    );
    assert_eq!(
        submit_headers.get("x-runway-version").unwrap(),
        "2024-11-06"
    );

    // The poll request must carry the version header as well.
    let poll_headers = &requests[1].headers;
    assert_eq!(requests[1].method.as_str(), "GET");
    assert_eq!(
        poll_headers.get("authorization").unwrap(),
        "Bearer test-api-key"
    );
    assert_eq!(poll_headers.get("x-runway-version").unwrap(), "2024-11-06");
}

#[tokio::test]
async fn should_pass_custom_headers() {
    let server = MockServer::start().await;
    mock_task_sequence(&server).await;

    let mut provider_headers = HashMap::new();
    provider_headers.insert("Custom-Provider-Header".to_string(), "val".to_string());
    let cfg = RunwaymlConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_headers(provider_headers)
        .with_poll_interval(Duration::from_millis(10))
        .with_timeout(Duration::from_secs(10));
    let provider = RunwaymlProvider::new(cfg);
    let model = provider.video(MODEL_ID);

    let mut opts = options("test");
    let mut request_headers = HashMap::new();
    request_headers.insert("Custom-Request-Header".to_string(), "req-val".to_string());
    opts.headers = Some(request_headers);

    generate_video(&model, opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let h = &requests[0].headers;
    assert_eq!(h.get("custom-provider-header").unwrap(), "val");
    assert_eq!(h.get("custom-request-header").unwrap(), "req-val");
    assert_eq!(h.get("x-runway-version").unwrap(), "2024-11-06");
}

#[tokio::test]
async fn should_return_error_when_task_failed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/text_to_video"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "task-fail"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/tasks/task-fail"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": "task-fail", "status": "FAILED"})),
        )
        .mount(&server)
        .await;

    let provider = RunwaymlProvider::new(config(server.uri()));
    let model = provider.video(MODEL_ID);

    let result = generate_video(&model, options("test")).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("FAILED"),
        "error should mention FAILED status, got: {err}"
    );
}

#[tokio::test]
async fn should_return_error_on_submit_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/text_to_video"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "invalid api key"})))
        .mount(&server)
        .await;

    let provider = RunwaymlProvider::new(config(server.uri()));
    let model = provider.video(MODEL_ID);

    let result = generate_video(&model, options("test")).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn should_return_max_videos_per_call() {
    let provider = RunwaymlProvider::new(RunwaymlConfig::new("test-api-key"));
    let model = provider.video(MODEL_ID);
    assert_eq!(model.max_videos_per_call(), Some(1));
    assert_eq!(model.provider(), "runwayml");
    assert_eq!(model.model_id(), MODEL_ID);
}
