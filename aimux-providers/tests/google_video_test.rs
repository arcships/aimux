//! Rust translation of the Google video model tests.
//! Source: `reference/ai/packages/google/src/google-video-model.test.ts`

use aimux_core::provider::Provider;
use aimux_core::video_model::{VideoCallOptions, generate_video};
use aimux_providers::{GoogleProviderSettings, create_google};
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
    let result = json!({"done": true, "response": {"generateVideoResponse": {"generatedSamples": [{"video": {"uri": format!("{}/files/video-123.mp4", server.uri())}}]}}});
    mock_predict_and_poll(&server, &result).await;
    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider
        .video_model("veo-3.0-generate-001")
        .unwrap()
        .unwrap();
    let r = generate_video(model.as_ref(), options("A cat"))
        .await
        .unwrap();
    assert_eq!(r.videos.len(), 1);
    match &r.videos[0] {
        aimux_core::video_model::VideoData::Url { url, media_type } => {
            assert_eq!(
                url,
                &format!("{}/files/video-123.mp4?key=test-api-key", server.uri())
            );
            assert_eq!(media_type, "video/mp4");
        }
        _ => panic!("expected URL video"),
    }
}

#[tokio::test]
async fn should_pass_prompt() {
    let server = MockServer::start().await;
    let result = json!({"done": true, "response": {"generateVideoResponse": {"generatedSamples": [{"video": {"uri": "https://generativelanguage.googleapis.com/files/video-123.mp4"}}]}}});
    mock_predict_and_poll(&server, &result).await;
    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider
        .video_model("veo-3.0-generate-001")
        .unwrap()
        .unwrap();
    generate_video(model.as_ref(), options("A cat"))
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["instances"][0]["prompt"], "A cat");
}

#[tokio::test]
async fn should_pass_aspect_ratio_and_duration() {
    let server = MockServer::start().await;
    let result = json!({"done": true, "response": {"generateVideoResponse": {"generatedSamples": [{"video": {"uri": "https://generativelanguage.googleapis.com/files/video-123.mp4"}}]}}});
    mock_predict_and_poll(&server, &result).await;
    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider
        .video_model("veo-3.0-generate-001")
        .unwrap()
        .unwrap();
    let mut opts = options("test");
    opts.aspect_ratio = Some(aimux_core::shared::AspectRatio::new(16, 9));
    opts.duration = Some(5);
    generate_video(model.as_ref(), opts).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["parameters"]["aspectRatio"], "16:9");
    assert_eq!(body["parameters"]["durationSeconds"], 5);
}

#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    let result = json!({"done": true, "response": {"generateVideoResponse": {"generatedSamples": [{"video": {"uri": "https://generativelanguage.googleapis.com/files/video-123.mp4"}}]}}});
    mock_predict_and_poll(&server, &result).await;
    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider
        .video_model("veo-3.0-generate-001")
        .unwrap()
        .unwrap();
    let r = generate_video(model.as_ref(), options("test"))
        .await
        .unwrap();
    assert!(r.response.timestamp.is_some());
    assert_eq!(
        r.response.model_id,
        Some("veo-3.0-generate-001".to_string())
    );
}
