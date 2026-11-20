//! MiniMax H3 video and image models against a mock server.
//!
//! Wire shapes: MiniMax `POST /v2/video_generation`,
//! `GET /v2/query/video_generation/{task_id}` and `POST /v1/images/generations`.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use aimux_core::AiMuxError;
use aimux_core::download::{Download, DownloadRequest, DownloadedFile};
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs, generate_image};
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoFileData, VideoModel, VideoOperationStatus,
    VideoPollOptions, generate_video,
};
use aimux_provider_utils::Resolvable;
use aimux_providers::{MiniMaxProviderSettings, create_minimax};

const TASK_ID: &str = "424010985738629";
const VIDEO_BYTES: &[u8] = b"video bytes";

fn provider(base_url: &str) -> aimux_providers::MiniMaxProvider {
    create_minimax(MiniMaxProviderSettings {
        api_key: Some(Resolvable::Value("test-key".into())),
        base_url: Some(base_url.to_string()),
        ..Default::default()
    })
    .unwrap()
}

/// Core's download takes a URL and an abort signal only: no provider header
/// (the key) can reach it.
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

fn video_options(prompt: &str) -> VideoCallOptions {
    let mut options = VideoCallOptions::new(prompt);
    options.poll = Some(VideoPollOptions {
        interval_ms: Some(1),
        timeout_ms: Some(10_000),
    });
    options
}

fn succeeded_task(video_url: &str) -> Value {
    json!({
        "task": {
            "id": TASK_ID,
            "model": "MiniMax-H3",
            "status": "succeeded",
            "content": {"url": video_url},
            "resolution": "768P",
            "duration": 4,
            "ratio": "16:9",
            "usage": {"total_seconds": 4}
        }
    })
}

fn body_of(request: &Request) -> Value {
    serde_json::from_slice(&request.body).unwrap()
}

#[tokio::test]
async fn video_is_created_queried_and_downloaded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/video_generation"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"task_id": TASK_ID})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v2/query/video_generation/{TASK_ID}")))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "task": {"id": TASK_ID, "status": "running"}
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v2/query/video_generation/{TASK_ID}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(succeeded_task("https://cdn.example.com/out.mp4")),
        )
        .mount(&server)
        .await;

    // A chat base URL with `/v1`: the media paths replace it.
    let model = provider(&format!("{}/v1", server.uri())).video("MiniMax-H3");
    let downloads = Arc::new(Mutex::new(Vec::new()));
    let mut options = video_options("一只猫在弹钢琴");
    options.duration = Some(6);
    options.aspect_ratio = Some("9:16".parse().unwrap());
    options.download = Some(recording_download(downloads.clone()));
    let result = generate_video(&model, options).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let create = requests
        .iter()
        .find(|r| r.url.path() == "/v2/video_generation")
        .unwrap();
    assert_eq!(
        body_of(create),
        json!({
            "model": "MiniMax-H3",
            "content": [{"type": "text", "text": "一只猫在弹钢琴"}],
            "resolution": "768P",
            "duration": 6,
            "ratio": "9:16",
            "response_format": "task",
        })
    );

    assert_eq!(
        *downloads.lock().unwrap(),
        ["https://cdn.example.com/out.mp4"]
    );
    match &result.videos[0] {
        VideoData::Binary { data, .. } => assert_eq!(data, VIDEO_BYTES),
        other => panic!("expected the downloaded video, got {other:?}"),
    }
    let metadata = &result.provider_metadata.as_ref().unwrap()["minimax"];
    assert_eq!(metadata["taskId"], TASK_ID);
    assert_eq!(metadata["ratio"], "16:9");
}

#[tokio::test]
async fn first_and_last_frames_are_role_tagged_with_an_adaptive_ratio() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/video_generation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"task_id": TASK_ID})))
        .mount(&server)
        .await;
    let model = provider(&server.uri()).video("MiniMax-H3-Max");
    let mut options = video_options("x");
    options.image = Some(VideoFile::File {
        media_type: "image/jpeg".into(),
        data: VideoFileData::Base64("/9j/AA==".into()),
    });
    options.provider_options.insert(
        "minimax".into(),
        serde_json::from_value(json!({
            "lastFrame": "https://example.com/last.png",
            "promptExpansionMode": "disabled"
        }))
        .unwrap(),
    );
    let start = model.do_start(&options).await.unwrap();
    assert_eq!(start.operation, json!({"taskId": TASK_ID}));

    let requests = server.received_requests().await.unwrap();
    let body = body_of(&requests[0]);
    assert_eq!(body["ratio"], "adaptive");
    assert_eq!(body["duration"], 5);
    assert_eq!(body["extra"], json!({"prompt_expansion_mode": "disabled"}));
    assert_eq!(
        body["content"][1],
        json!({"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,/9j/AA=="}, "role": "first_frame"})
    );
    assert_eq!(
        body["content"][2],
        json!({"type": "image_url", "image_url": {"url": "https://example.com/last.png"}, "role": "last_frame"})
    );
}

#[tokio::test]
async fn a_failed_create_is_not_sent_again() {
    for status in [500, 502] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/video_generation"))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({
                "type": "error",
                "error": {"type": "server_error", "message": "internal error (1000)", "http_code": "500"}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let model = provider(&server.uri()).video("MiniMax-H3");
        let error = generate_video(&model, video_options("x"))
            .await
            .unwrap_err();
        let AiMuxError::ApiCall(detail) = &error else {
            panic!("expected an API call error, got {error:?}");
        };
        assert!(!detail.is_retryable);
        assert_eq!(detail.message, "internal error (1000)");
        assert_eq!(detail.provider_code.as_deref(), Some("server_error"));
    }
}

#[tokio::test]
async fn a_create_without_task_id_is_not_sent_again() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/video_generation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    let model = provider(&server.uri()).video("MiniMax-H3");
    let error = generate_video(&model, video_options("x"))
        .await
        .unwrap_err();
    assert!(
        matches!(error, AiMuxError::InvalidResponseData { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_rate_limited_create_is_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/video_generation"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after-ms", "0")
                .set_body_json(json!({
                    "type": "error",
                    "error": {"type": "rate_limit_error", "message": "rate limit (1002)"}
                })),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v2/video_generation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"task_id": TASK_ID})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v2/query/video_generation/{TASK_ID}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(succeeded_task("https://cdn.example.com/out.mp4")),
        )
        .mount(&server)
        .await;
    let model = provider(&server.uri()).video("MiniMax-H3");
    let mut options = video_options("x");
    options.download = Some(recording_download(Arc::default()));
    generate_video(&model, options).await.unwrap();
    let creates = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/v2/video_generation")
        .count();
    assert_eq!(creates, 2);
}

#[tokio::test]
async fn a_transient_query_failure_is_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/video_generation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"task_id": TASK_ID})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v2/query/video_generation/{TASK_ID}")))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("retry-after-ms", "0")
                .set_body_json(json!({"type": "error", "error": {"message": "busy"}})),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v2/query/video_generation/{TASK_ID}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(succeeded_task("https://cdn.example.com/out.mp4")),
        )
        .mount(&server)
        .await;
    let model = provider(&server.uri()).video("MiniMax-H3");
    let mut options = video_options("x");
    options.download = Some(recording_download(Arc::default()));
    generate_video(&model, options).await.unwrap();
}

#[tokio::test]
async fn query_failures_and_mismatches_end_the_poll() {
    let server = MockServer::start().await;
    let model = provider(&server.uri()).video("MiniMax-H3");
    let options = video_options("x");
    let operation = json!({"taskId": TASK_ID});

    for (status, body) in [
        (
            "failed",
            json!({"task": {"id": TASK_ID, "status": "failed", "error": {"message": "sensitive content"}}}),
        ),
        (
            "expired",
            json!({"task": {"id": TASK_ID, "status": "expired"}}),
        ),
        ("cancelled", json!({"id": TASK_ID, "status": "cancelled"})),
    ] {
        server.reset().await;
        Mock::given(method("GET"))
            .and(path(format!("/v2/query/video_generation/{TASK_ID}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let error = model.do_status(&operation, &options).await.unwrap_err();
        let AiMuxError::ApiCall(detail) = &error else {
            panic!("{status}: {error:?}");
        };
        assert!(!detail.is_retryable, "{status}");
        assert_eq!(detail.provider_code.as_deref(), Some(status));
        if status == "failed" {
            assert!(detail.message.contains("sensitive content"));
        }
    }

    // The query must answer for the task that was asked about.
    server.reset().await;
    Mock::given(method("GET"))
        .and(path(format!("/v2/query/video_generation/{TASK_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "task": {"id": "other", "status": "succeeded", "content": {"url": "https://x/y.mp4"}}
        })))
        .mount(&server)
        .await;
    let error = model.do_status(&operation, &options).await.unwrap_err();
    assert!(
        matches!(error, AiMuxError::InvalidResponseData { .. }),
        "{error:?}"
    );

    // A success needs an http(s) video URL.
    server.reset().await;
    Mock::given(method("GET"))
        .and(path(format!("/v2/query/video_generation/{TASK_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(succeeded_task("ftp://x/y.mp4")))
        .mount(&server)
        .await;
    let error = model.do_status(&operation, &options).await.unwrap_err();
    assert!(
        matches!(error, AiMuxError::InvalidResponseData { .. }),
        "{error:?}"
    );

    // The flat response format is read too.
    server.reset().await;
    Mock::given(method("GET"))
        .and(path(format!("/v2/query/video_generation/{TASK_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": TASK_ID, "status": "queued"
        })))
        .mount(&server)
        .await;
    assert!(matches!(
        model.do_status(&operation, &options).await.unwrap(),
        VideoOperationStatus::Pending
    ));
}

#[tokio::test]
async fn invalid_video_options_send_nothing() {
    let server = MockServer::start().await;
    let model = provider(&server.uri()).video("MiniMax-H3-Lite");
    let mut options = video_options("x");
    options.duration = Some(3);
    let error = generate_video(&model, options).await.unwrap_err();
    assert!(
        matches!(error, AiMuxError::InvalidArgument { .. }),
        "{error:?}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

async fn mount_image(server: &MockServer, status: u16, body: Value) {
    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn image_is_generated_and_downloaded_without_the_key() {
    let server = MockServer::start().await;
    let image_url = format!("{}/files/out-0.jpg", server.uri());
    mount_image(
        &server,
        200,
        json!({
            "created": 1792308678,
            "data": [{"url": image_url, "size": "2720x1536", "revised_prompt": "暖光下的橘猫"}],
            "output_format": "jpeg",
            "usage": {"generated_images": 1, "total_tokens": 7254, "input_tokens": 3174, "output_tokens": 4080},
            "request_id": "req-1"
        }),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/files/out-0.jpg"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"jpeg bytes".to_vec()))
        .mount(&server)
        .await;

    // A chat base URL under `/anthropic/v1`: the media path replaces it.
    let model = provider(&format!("{}/anthropic/v1", server.uri())).image("MiniMax-H3-Image");
    let mut options = ImageCallOptions::new("橘猫");
    options.size = Some("2720x1536".parse().unwrap());
    let result = generate_image(&model, options).await.unwrap();

    match result.images {
        ImageOutputs::Binary(images) => assert_eq!(images, vec![b"jpeg bytes".to_vec()]),
        ImageOutputs::Base64(_) => panic!("expected binary"),
    }
    let usage = result.usage.unwrap();
    assert_eq!(usage.total_tokens, Some(7254));
    let metadata = &result.provider_metadata.as_ref().unwrap()["minimax"];
    assert_eq!(metadata["revisedPrompt"], "暖光下的橘猫");
    assert_eq!(metadata["outputFormat"], "jpeg");

    let requests = server.received_requests().await.unwrap();
    let generate = &requests[0];
    assert_eq!(generate.url.path(), "/v1/images/generations");
    assert_eq!(
        body_of(generate),
        json!({
            "model": "MiniMax-H3-Image",
            "prompt": "橘猫",
            "n": 1,
            "response_format": "url",
            "stream": false,
            "size": "2720x1536"
        })
    );
    let download = &requests[1];
    assert_eq!(download.url.path(), "/files/out-0.jpg");
    assert!(
        !download.headers.contains_key("authorization"),
        "the image download must not carry the key"
    );
}

#[tokio::test]
async fn a_failed_image_generation_is_not_sent_again() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "type": "error", "error": {"type": "server_error", "message": "internal error"}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let model = provider(&server.uri()).image("MiniMax-H3-Image");
    let error = generate_image(&model, ImageCallOptions::new("x"))
        .await
        .unwrap_err();
    assert!(!error.is_retryable(), "{error:?}");
}

#[tokio::test]
async fn a_failed_image_download_does_not_generate_again() {
    let server = MockServer::start().await;
    let image_url = format!("{}/files/gone.jpg", server.uri());
    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"url": image_url}]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/files/gone.jpg"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let model = provider(&server.uri()).image("MiniMax-H3-Image");
    let error = generate_image(&model, ImageCallOptions::new("x"))
        .await
        .unwrap_err();
    assert!(!error.is_retryable(), "{error:?}");
}

#[tokio::test]
async fn a_per_image_error_is_reported() {
    let server = MockServer::start().await;
    mount_image(
        &server,
        200,
        json!({"data": [{"error": {"code": "1027", "message": "output image failed content safety check"}}]}),
    )
    .await;
    let model = provider(&server.uri()).image("MiniMax-H3-Image-Highspeed");
    let error = model
        .do_generate(&ImageCallOptions::new("x"))
        .await
        .unwrap_err();
    let AiMuxError::ApiCall(detail) = &error else {
        panic!("{error:?}");
    };
    assert_eq!(detail.provider_code.as_deref(), Some("1027"));
    assert!(detail.message.contains("content safety"));
    assert!(!detail.is_retryable);
}

#[tokio::test]
async fn a_base_resp_failure_in_a_2xx_body_is_an_error() {
    let server = MockServer::start().await;
    mount_image(
        &server,
        200,
        json!({"base_resp": {"status_code": 1008, "status_msg": "insufficient balance"}}),
    )
    .await;
    let model = provider(&server.uri()).image("MiniMax-H3-Image");
    let error = model
        .do_generate(&ImageCallOptions::new("x"))
        .await
        .unwrap_err();
    let AiMuxError::ApiCall(detail) = &error else {
        panic!("{error:?}");
    };
    assert_eq!(detail.provider_code.as_deref(), Some("1008"));
    assert_eq!(detail.message, "insufficient balance");
}

#[tokio::test]
async fn invalid_image_options_send_nothing() {
    let server = MockServer::start().await;
    let model = provider(&server.uri()).image("MiniMax-H3-Image");
    let mut options = ImageCallOptions::new("x");
    options.size = Some("100x100".parse().unwrap());
    let error = model.do_generate(&options).await.unwrap_err();
    assert!(
        matches!(error, AiMuxError::InvalidArgument { .. }),
        "{error:?}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn the_provider_serves_video_and_image_models() {
    use aimux_core::provider::Provider;
    let provider = provider("https://api.minimaxi.com/v1");
    assert_eq!(
        provider
            .video_model("MiniMax-H3")
            .unwrap()
            .unwrap()
            .provider(),
        "minimax.video"
    );
    assert_eq!(
        provider.image_model("MiniMax-H3-Image").unwrap().provider(),
        "minimax.image"
    );
    assert!(provider.language_model("MiniMax-M2").is_err());
}
