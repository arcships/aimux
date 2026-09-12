//! Rust translation of the Google video model tests.
//! Source: `reference/ai/packages/google/src/google-video-model.test.ts`

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_core::AiMuxError;
use aimux_core::video_model::{VideoCallOptions, generate_video};
use aimux_providers::{GoogleConfig, GoogleProvider};
use serde_json::{Value, json};
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

fn request_header<'a>(request: &'a wiremock::Request, name: &str) -> Option<&'a str> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
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
    let r = generate_video(&model, options("A cat")).await.unwrap();
    assert_eq!(r.videos.len(), 1);
}

#[tokio::test]
async fn poll_retry_does_not_submit_a_second_generation() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/models/veo-3.0-generate-001:predictLongRunning"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"name": "operations/test-op"})),
        )
        .mount(&server)
        .await;

    let poll_attempts = Arc::new(AtomicUsize::new(0));
    let responder_attempts = Arc::clone(&poll_attempts);
    Mock::given(method("GET"))
        .and(path("/operations/test-op"))
        .respond_with(move |_: &wiremock::Request| {
            if responder_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(503)
                    .insert_header("retry-after-ms", "0")
                    .set_body_json(json!({
                        "error": {"message": "try again", "status": "UNAVAILABLE"}
                    }))
            } else {
                ResponseTemplate::new(200).set_body_json(json!({
                    "done": true,
                    "response": {"videos": [{"gcsUri": "gs://bucket/video.mp4"}]}
                }))
            }
        })
        .mount(&server)
        .await;

    let config = GoogleConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_retry_config(aimux_provider_utils::RetryConfig {
            max_retries: 1,
            ..Default::default()
        });
    let provider = GoogleProvider::new(config);
    let model = provider.video("veo-3.0-generate-001");
    let mut opts = options("A cat");
    opts.max_retries = Some(1);

    let result = generate_video(&model, opts).await.unwrap();
    assert_eq!(result.videos.len(), 1);
    assert_eq!(poll_attempts.load(Ordering::SeqCst), 2);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path().ends_with(":predictLongRunning"))
            .count(),
        1
    );
    assert!(
        requests
            .iter()
            .find(|request| request.url.path().ends_with(":predictLongRunning"))
            .and_then(|request| request_header(request, "idempotency-key"))
            .is_some(),
        "the start request should receive a generated idempotency key"
    );
    assert!(
        requests
            .iter()
            .filter(|request| request.url.path() == "/operations/test-op")
            .all(|request| request_header(request, "idempotency-key").is_none()),
        "the generated start key must not leak to status requests"
    );
}

#[tokio::test]
async fn start_retry_reuses_the_same_idempotency_key() {
    let server = MockServer::start().await;
    let start_attempts = Arc::new(AtomicUsize::new(0));
    let responder_attempts = Arc::clone(&start_attempts);

    Mock::given(method("POST"))
        .and(path("/models/veo-3.0-generate-001:predictLongRunning"))
        .respond_with(move |_: &wiremock::Request| {
            if responder_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(503)
                    .insert_header("retry-after-ms", "0")
                    .set_body_json(json!({
                        "error": {"message": "try again", "status": "UNAVAILABLE"}
                    }))
            } else {
                ResponseTemplate::new(200).set_body_json(json!({"name": "operations/test-op"}))
            }
        })
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/operations/test-op"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "done": true,
            "response": {"videos": [{"gcsUri": "gs://bucket/video.mp4"}]}
        })))
        .mount(&server)
        .await;

    let config = GoogleConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_retry_config(aimux_provider_utils::RetryConfig {
            max_retries: 1,
            ..Default::default()
        });
    let provider = GoogleProvider::new(config);
    let model = provider.video("veo-3.0-generate-001");
    let mut opts = options("A cat");
    opts.max_retries = Some(1);

    generate_video(&model, opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let starts: Vec<_> = requests
        .iter()
        .filter(|request| request.url.path().ends_with(":predictLongRunning"))
        .collect();
    assert_eq!(starts.len(), 2);
    let first_key = request_header(starts[0], "idempotency-key");
    assert!(first_key.is_some());
    assert_eq!(first_key, request_header(starts[1], "idempotency-key"));
    assert!(
        requests
            .iter()
            .filter(|request| request.url.path() == "/operations/test-op")
            .all(|request| request_header(request, "idempotency-key").is_none())
    );
}

#[tokio::test]
async fn poll_deadline_reached_during_delay_does_not_issue_status_request() {
    let server = MockServer::start().await;
    let result =
        json!({"done": true, "response": {"videos": [{"gcsUri": "gs://bucket/video.mp4"}]}});
    mock_predict_and_poll(&server, &result).await;
    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.video("veo-3.0-generate-001");
    let mut opts = options("A cat");
    opts.poll = Some(aimux_core::video_model::VideoPollOptions {
        interval_ms: Some(100),
        timeout_ms: Some(10),
    });

    let error = generate_video(&model, opts).await.unwrap_err();

    assert!(matches!(error, AiMuxError::Timeout(_)));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path().ends_with(":predictLongRunning"))
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path() == "/operations/test-op")
            .count(),
        0
    );
}

#[tokio::test]
async fn poll_retry_exhaustion_does_not_submit_a_second_generation() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/models/veo-3.0-generate-001:predictLongRunning"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"name": "operations/test-op"})),
        )
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/operations/test-op"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("retry-after-ms", "0")
                .set_body_json(json!({
                    "error": {"message": "still unavailable", "status": "UNAVAILABLE"}
                })),
        )
        .mount(&server)
        .await;

    let config = GoogleConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_retry_config(aimux_provider_utils::RetryConfig {
            max_retries: 1,
            ..Default::default()
        });
    let provider = GoogleProvider::new(config);
    let model = provider.video("veo-3.0-generate-001");

    let error = generate_video(&model, options("A cat")).await.unwrap_err();
    assert!(matches!(error, AiMuxError::Retry(_)));

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path().ends_with(":predictLongRunning"))
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path() == "/operations/test-op")
            .count(),
        2
    );
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
    generate_video(&model, options("A cat")).await.unwrap();
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
    generate_video(&model, opts).await.unwrap();
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
    let r = generate_video(&model, options("test")).await.unwrap();
    assert!(r.response.timestamp.is_some());
    assert_eq!(
        r.response.model_id,
        Some("veo-3.0-generate-001".to_string())
    );
}
