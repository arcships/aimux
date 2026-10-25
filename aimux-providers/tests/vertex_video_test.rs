//! Rust translation of the Google Vertex video model tests.
//! Source: `reference/ai/packages/google-vertex/src/google-vertex-video-model.test.ts`

use std::sync::{Arc, Mutex};

use aimux_core::Provider;
use aimux_core::download::{Download, DownloadRequest, DownloadedFile};
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
    o.download = Some(recording_download(Arc::default()));
    o
}

/// The bytes the test download returns for every video URL.
const VIDEO_BYTES: &[u8] = b"video bytes";

/// A download that fetches nothing: it records each URL `generate_video`
/// asks for and returns [`VIDEO_BYTES`].
fn recording_download(requests: Arc<Mutex<Vec<String>>>) -> Download {
    Download::new(move |request: DownloadRequest| {
        requests.lock().unwrap().push(request.url);
        async {
            Ok(DownloadedFile {
                data: VIDEO_BYTES.to_vec(),
                media_type: None,
            })
        }
    })
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

/// partial: `should return completed with GCS URI video`; full port in src/vertex/tests/video_model.rs.
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
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut opts = options("A cat");
    opts.download = Some(recording_download(requests.clone()));
    let r = generate_video(model.as_ref(), opts).await.unwrap();
    // `generate_video` downloads the returned URL with the injected download
    // (the guarded default fetches only HTTP(S), as upstream's `fetch`).
    assert_eq!(*requests.lock().unwrap(), ["gs://bucket/video.mp4"]);
    assert_eq!(r.videos.len(), 1);
    match &r.videos[0] {
        aimux_core::video_model::VideoData::Binary { data, media_type } => {
            assert_eq!(data, VIDEO_BYTES);
            assert_eq!(media_type, "video/mp4");
        }
        _ => panic!("expected the downloaded video"),
    }
}

/// partial: `should pass correct request body`; full port in src/vertex/tests/video_model.rs.
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

/// partial: `should include warnings and response metadata`; full port in src/vertex/tests/video_model.rs.
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
