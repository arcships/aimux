//! Rust translation of the Fal video model tests.
//! Source: `reference/ai/packages/fal/src/fal-video-model.test.ts`

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::download::{Download, DownloadRequest, DownloadedFile};
use aimux_core::video_model::{VideoCallOptions, VideoModel, generate_video};
use aimux_provider_utils::Resolvable;
use aimux_providers::{FalProviderSettings, create_fal};

fn fast_poll() -> Option<aimux_core::video_model::VideoPollOptions> {
    Some(aimux_core::video_model::VideoPollOptions {
        interval_ms: Some(1),
        timeout_ms: Some(10_000),
    })
}

fn options(prompt: &str) -> VideoCallOptions {
    let mut o = VideoCallOptions::new(prompt);
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

    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_fal(config).unwrap();
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

    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_fal(config).unwrap();
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
    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        headers: Some((ph).into_iter().map(|(k, v)| (k, Some(v))).collect()),
        ..Default::default()
    };
    let provider = create_fal(config).unwrap();
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

    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_fal(config).unwrap();
    let model = provider.video("fal-ai/kling-video");

    let r = generate_video(&model, options("test")).await.unwrap();
    assert!(r.response.timestamp.is_some());
    assert_eq!(r.response.model_id, Some("fal-ai/kling-video".to_string()));
}

const LUMA_PROMPT: &str = "A futuristic city with flying cars";

/// `luma-dream-machine` on a server that queues every request, as the
/// upstream `createBasicModel` does.
async fn luma_dream_machine(server: &MockServer) -> aimux_providers::FalVideoModel {
    Mock::given(method("POST"))
        .and(path("/fal-ai/luma-dream-machine"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "request_id": "test-request-id-123",
            "response_url": format!(
                "{}/fal-ai/luma-dream-machine/requests/test-request-id-123",
                server.uri()
            ),
        })))
        .mount(server)
        .await;
    create_fal(FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap()
    .video("luma-dream-machine")
}

fn with_fal(fal: Value) -> VideoCallOptions {
    let mut call = VideoCallOptions::new(LUMA_PROMPT);
    call.provider_options = serde_json::from_value(json!({ "fal": fal })).unwrap();
    call
}

async fn submitted_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    serde_json::from_slice(&requests.last().unwrap().body).unwrap()
}

/// fal-video-model.ts buildRequestBody: the five schema keys go out in
/// snake_case, every other key as given (upstream "should pass loop
/// option" .. "should pass through additional options").
#[tokio::test]
async fn should_pass_the_fal_options() {
    let server = MockServer::start().await;
    let model = luma_dream_machine(&server).await;
    for (fal, sent) in [
        (json!({ "loop": true }), json!({ "loop": true })),
        (
            json!({ "motionStrength": 0.8 }),
            json!({ "motion_strength": 0.8 }),
        ),
        (
            json!({ "resolution": "1080p" }),
            json!({ "resolution": "1080p" }),
        ),
        (
            json!({ "negativePrompt": "blurry, low quality" }),
            json!({ "negative_prompt": "blurry, low quality" }),
        ),
        (
            json!({ "promptOptimizer": true }),
            json!({ "prompt_optimizer": true }),
        ),
        (
            json!({ "custom_param": "custom_value", "another_param": 123 }),
            json!({ "custom_param": "custom_value", "another_param": 123 }),
        ),
        (
            json!({ "loop": null, "motionStrength": null, "extra": null }),
            json!({ "extra": null }),
        ),
    ] {
        model.do_start(&with_fal(fal.clone())).await.unwrap();
        let mut expected = sent;
        expected["prompt"] = json!(LUMA_PROMPT);
        assert_eq!(submitted_body(&server).await, expected, "{fal}");
    }
}

/// `providerOptions.fal` is parsed against `falVideoModelOptionsSchema`
/// first: a value outside it fails before anything is sent.
#[tokio::test]
async fn should_reject_video_options_outside_the_schema() {
    let model = create_fal(FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some("http://127.0.0.1:9".to_string()),
        ..Default::default()
    })
    .unwrap()
    .video("luma-dream-machine");
    for fal in [
        json!({ "loop": "yes" }),
        json!({ "motionStrength": 1.5 }),
        json!({ "motionStrength": -0.1 }),
        json!({ "resolution": 720 }),
        json!({ "negativePrompt": false }),
        json!({ "promptOptimizer": "true" }),
    ] {
        let error = model.do_start(&with_fal(fal.clone())).await.unwrap_err();
        let aimux_core::error::AiMuxError::InvalidArgument { message, cause } = &error else {
            panic!("expected InvalidArgument for {fal}, got {error:?}");
        };
        assert_eq!(
            message, "Invalid argument for parameter providerOptions: invalid fal provider options",
            "{fal}"
        );
        assert!(cause.is_some(), "{fal}");
    }
}
