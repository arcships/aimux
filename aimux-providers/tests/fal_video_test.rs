//! Rust translation of the Fal video model tests.
//! Source: `reference/ai/packages/fal/src/fal-video-model.test.ts`

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::download::{Download, DownloadRequest, DownloadedFile};
use aimux_core::video_model::{VideoCallOptions, VideoModel, VideoOperationStatus, generate_video};
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
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "request_id": "test-id",
            "response_url": format!("{}/fal-ai/kling-video/requests/test-id", server.uri()),
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/fal-ai/kling-video/requests/test-id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(result.clone()))
        .mount(server)
        .await;
}

/// partial: `should return completed status with video data`; full port in src/fal/tests/video_model.rs.
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

/// partial: `should pass the correct parameters including prompt`; full port in src/fal/tests/video_model.rs.
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

/// partial: `should pass headers`; full port in src/fal/tests/video_model.rs.
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

/// partial: `should return completed status with video data`; full port in src/fal/tests/video_model.rs.
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

/// A seed or duration of 0 is not sent: upstream tests `if (options.seed)`
/// and `if (options.duration)`.
#[tokio::test]
async fn should_not_send_a_zero_seed_or_duration() {
    let server = MockServer::start().await;
    let model = luma_dream_machine(&server).await;
    let mut call = VideoCallOptions::new(LUMA_PROMPT);
    call.seed = Some(0);
    call.duration = Some(0);
    model.do_start(&call).await.unwrap();
    assert_eq!(
        submitted_body(&server).await,
        json!({ "prompt": LUMA_PROMPT })
    );
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

/// fal-video-model.ts doStart: the queue answers with a `response_url` and
/// the operation carries it; a queue answer without one fails.
#[tokio::test]
async fn should_require_a_response_url_from_the_queue() {
    let server = MockServer::start().await;
    let model = luma_dream_machine(&server).await;
    let start = model
        .do_start(&VideoCallOptions::new(LUMA_PROMPT))
        .await
        .unwrap();
    assert_eq!(
        start.operation,
        json!({
            "responseUrl": format!("{}/fal-ai/luma-dream-machine/requests/test-request-id-123", server.uri()),
            "submitUrl": format!("{}/fal-ai/luma-dream-machine", server.uri()),
        })
    );

    Mock::given(method("POST"))
        .and(path("/fal-ai/luma-dream-machine"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "request_id": "no-response-url" })),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    let error = model
        .do_start(&VideoCallOptions::new(LUMA_PROMPT))
        .await
        .unwrap_err();
    let aimux_core::error::AiMuxError::InvalidResponseData { message, .. } = &error else {
        panic!("expected InvalidResponseData, got {error:?}");
    };
    assert_eq!(message, "No response URL returned from queue endpoint");
}

/// A status poll answered with `detail: "Request is still in progress"` is
/// pending, whatever its status; any other error response ends the call
/// (fal-video-model.ts fetchStatus).
#[tokio::test]
async fn should_poll_only_while_the_request_is_in_progress() {
    let server = MockServer::start().await;
    let model = luma_dream_machine(&server).await;
    Mock::given(method("GET"))
        .and(path(
            "/fal-ai/luma-dream-machine/requests/test-request-id-123",
        ))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({ "detail": "Request is still in progress" })),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/fal-ai/luma-dream-machine/requests/test-request-id-123",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "video": { "url": "https://fal.media/files/video-output.mp4" },
        })))
        .mount(&server)
        .await;
    let call = VideoCallOptions::new(LUMA_PROMPT);
    let start = model.do_start(&call).await.unwrap();
    let first = model.do_status(&start.operation, &call).await.unwrap();
    assert!(matches!(first, VideoOperationStatus::Pending), "{first:?}");
    let second = model.do_status(&start.operation, &call).await.unwrap();
    assert!(
        matches!(second, VideoOperationStatus::Completed(_)),
        "{second:?}"
    );

    let failed = MockServer::start().await;
    let model = luma_dream_machine(&failed).await;
    Mock::given(method("GET"))
        .and(path(
            "/fal-ai/luma-dream-machine/requests/test-request-id-123",
        ))
        .respond_with(
            ResponseTemplate::new(400).set_body_json(json!({ "detail": "Invalid prompt" })),
        )
        .with_priority(1)
        .mount(&failed)
        .await;
    let start = model
        .do_start(&VideoCallOptions::new(LUMA_PROMPT))
        .await
        .unwrap();
    let error = model
        .do_status(&start.operation, &VideoCallOptions::new(LUMA_PROMPT))
        .await
        .unwrap_err();
    let aimux_core::error::AiMuxError::ApiCall(detail) = &error else {
        panic!("expected ApiCall, got {error:?}");
    };
    assert_eq!(detail.status_code, Some(400));
}

/// fal-video-model.ts buildResult: `mediaType` is the response's
/// `content_type`, falling back to `video/mp4`. Read through `do_status`,
/// which returns the model's own answer before Core downloads it.
#[tokio::test]
async fn should_take_the_media_type_from_the_response() {
    let server = MockServer::start().await;
    mock_queue_and_result(
        &server,
        &json!({
            "video": { "url": "https://cdn.fal.run/video.webm", "content_type": "video/webm" },
        }),
    )
    .await;
    let model = create_fal(FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    })
    .unwrap()
    .video("fal-ai/kling-video");
    let call = VideoCallOptions::new("A cat playing");
    let start = model.do_start(&call).await.unwrap();

    let status = model.do_status(&start.operation, &call).await.unwrap();
    let VideoOperationStatus::Completed(result) = status else {
        panic!("expected a completed status, got {status:?}");
    };
    let aimux_core::video_model::VideoData::Url { media_type, .. } = &result.videos[0] else {
        panic!("expected a URL video");
    };
    assert_eq!(media_type, "video/webm");
}

/// fal-video-model.ts doStart returns `response: { ..., headers: undefined }`:
/// the queue submission's headers are not carried.
#[tokio::test]
async fn should_not_report_headers_from_the_queue_submission() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fal-ai/kling-video"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-request-id", "queue-request-id")
                .set_body_json(json!({
                    "request_id": "test-id",
                    "response_url": format!("{}/fal-ai/kling-video/requests/test-id", server.uri()),
                })),
        )
        .mount(&server)
        .await;
    let model = create_fal(FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    })
    .unwrap()
    .video("fal-ai/kling-video");

    let start = model
        .do_start(&VideoCallOptions::new("A cat playing"))
        .await
        .unwrap();
    assert_eq!(start.response.headers, None);
}

/// fal-video-model.ts buildResult: a completed response without
/// `video.url` is an error, and there is no top-level `url` fallback.
#[tokio::test]
async fn should_throw_when_the_video_url_is_missing() {
    let server = MockServer::start().await;
    mock_queue_and_result(&server, &json!({})).await;
    let model = create_fal(FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    })
    .unwrap()
    .video("fal-ai/kling-video");
    let call = VideoCallOptions::new("A cat playing");
    let start = model.do_start(&call).await.unwrap();

    let error = model.do_status(&start.operation, &call).await.unwrap_err();
    let aimux_core::error::AiMuxError::InvalidResponseData { message, .. } = &error else {
        panic!("expected InvalidResponseData, got {error:?}");
    };
    assert_eq!(message, "No video URL in response");

    // The top-level `url` is not read, and an empty string is not a URL.
    for body in [
        json!({ "url": "https://cdn.fal.run/video.mp4" }),
        json!({ "video": { "url": "" } }),
    ] {
        let server = MockServer::start().await;
        mock_queue_and_result(&server, &body).await;
        let model = create_fal(FalProviderSettings {
            api_key: Some(Resolvable::Value("test-key".to_string())),
            base_url: Some(server.uri().to_string()),
            ..Default::default()
        })
        .unwrap()
        .video("fal-ai/kling-video");
        let call = VideoCallOptions::new("A cat playing");
        let start = model.do_start(&call).await.unwrap();
        assert!(
            model.do_status(&start.operation, &call).await.is_err(),
            "{body} should not produce a video"
        );
    }
}

/// `providerMetadata.fal` of a completed status (fal-video-model.ts
/// buildResult): the video under `videos`, then `seed` and `timings`.
#[tokio::test]
async fn should_include_provider_metadata_in_a_completed_status() {
    let server = MockServer::start().await;
    mock_queue_and_result(
        &server,
        &json!({
            "video": {
                "url": "https://fal.media/files/video-output.mp4",
                "width": 1920,
                "height": 1080,
                "duration": 5.0,
                "fps": 24,
                "content_type": "video/mp4",
            },
            "seed": 12345,
            "timings": { "inference": 45.5 },
        }),
    )
    .await;
    let model = create_fal(FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    })
    .unwrap()
    .video("fal-ai/kling-video");
    let call = VideoCallOptions::new("A cat playing");
    let start = model.do_start(&call).await.unwrap();

    let status = model.do_status(&start.operation, &call).await.unwrap();
    let VideoOperationStatus::Completed(result) = status else {
        panic!("expected a completed status, got {status:?}");
    };
    assert_eq!(
        serde_json::to_value(result.provider_metadata.as_ref().unwrap()).unwrap(),
        json!({
            "fal": {
                "videos": [{
                    "url": "https://fal.media/files/video-output.mp4",
                    "width": 1920,
                    "height": 1080,
                    "duration": 5.0,
                    "fps": 24,
                    "contentType": "video/mp4",
                }],
                "seed": 12345,
                "timings": { "inference": 45.5 },
            },
        })
    );
}
