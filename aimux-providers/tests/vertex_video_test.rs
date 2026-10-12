//! Rust translation of the Google Vertex video model tests.
//! Source: `reference/ai/packages/google-vertex/src/google-vertex-video-model.test.ts`

use aimux_core::Provider;
use aimux_core::video_model::{VideoCallOptions, generate_video};
use aimux_providers::{VertexProvider, VertexProviderSettings, create_google_vertex};
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

fn make_provider(server_uri: &str) -> VertexProvider {
    create_google_vertex(VertexProviderSettings {
        access_token: Some("test-token".to_string().into()),
        project: Some("test-project".to_string()),
        location: Some("us-central1".to_string()),
        base_url: Some(server_uri.to_string()),
        ..Default::default()
    })
    .unwrap()
}

async fn mock_predict_and_poll(server: &MockServer, result: &Value) {
    Mock::given(method("POST"))
        .and(path("/models/veo-3.0-generate-001:predictLongRunning"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"name": "operations/test-op"})),
        )
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/models/veo-3.0-generate-001:fetchPredictOperation"))
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
            assert_eq!(url, "gs://bucket/video.mp4");
            assert_eq!(media_type, "video/mp4");
        }
        _ => panic!("expected URL video"),
    }
}

#[tokio::test]
async fn should_pass_prompt() {
    let server = MockServer::start().await;
    let result =
        json!({"done": true, "response": {"videos": [{"gcsUri": "gs://bucket/video.mp4"}]}});
    mock_predict_and_poll(&server, &result).await;
    let provider = make_provider(&server.uri());
    let model = provider
        .video_model("veo-3.0-generate-001")
        .unwrap()
        .unwrap();
    generate_video(model.as_ref(), options("A cat"))
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body,
        json!({
            "instances": [{"prompt": "A cat"}],
            "parameters": {"sampleCount": 1},
        })
    );
    let poll_body: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(poll_body, json!({"operationName": "operations/test-op"}));
}

#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    let result =
        json!({"done": true, "response": {"videos": [{"gcsUri": "gs://bucket/video.mp4"}]}});
    mock_predict_and_poll(&server, &result).await;
    let provider = make_provider(&server.uri());
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
