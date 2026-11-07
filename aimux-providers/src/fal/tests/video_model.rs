//! Port of `@ai-sdk/fal@3.0.54` `src/fal-video-model.test.ts`.
//!
//! Upstream's `createTestServer` answers the hard-coded
//! `https://queue.fal.run`; here every test starts its own wiremock server and
//! passes it as `base_url`, the only URL knob a video model has.
//!
//! Two deliberate deviations are visible in the bodies rather than in what
//! they assert. First, `do_status` also needs `submitUrl` in the operation —
//! it is the trusted origin of the status poll, so the status URL (which comes
//! from the queue response) passes the SSRF guard — while upstream's
//! `doStatus` tests hand it only `responseUrl`; the helper [`operation`]
//! builds the pair. Second, each status response is mounted per test instead
//! of living in one shared route table. The `_internal.currentDate` hook has
//! no Rust counterpart, so a test that pins the timestamp cannot be ported
//! literally; no case here pins one.

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{AspectRatio, SharedProviderOptions};
use aimux_core::video_model::{
    VideoCallOptions, VideoFile, VideoFileData, VideoModel, VideoOperationStatus,
};
use aimux_provider_utils::Resolvable;

use crate::fal::{FalProviderSettings, FalVideoModel, create_fal};

const PROMPT: &str = "A futuristic city with flying cars";
const MODEL_ID: &str = "luma-dream-machine";
/// The queue endpoint of [`MODEL_ID`] on a mock server
/// (`https://queue.fal.run/fal-ai/luma-dream-machine` upstream).
const SUBMIT_PATH: &str = "/fal-ai/luma-dream-machine";
/// The status endpoint of the default queued request.
const STATUS_PATH: &str = "/fal-ai/luma-dream-machine/requests/test-request-id-123";
const VIDEO_URL: &str = "https://fal.media/files/video-output.mp4";

/// `new FalVideoModel(modelId, { provider: 'fal.video', headers: { 'api-key':
/// 'test-key' } })` at `base_url`: the key goes out as fal's
/// `Authorization: Key …`.
fn video_model(base_url: &str, model_id: &str) -> FalVideoModel {
    create_fal(FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(base_url.to_string()),
        ..Default::default()
    })
    .unwrap()
    .video(model_id)
}

/// `createBasicModel()`.
fn create_basic_model(server: &MockServer) -> FalVideoModel {
    video_model(&server.uri(), MODEL_ID)
}

/// `defaultOptions`: the prompt every upstream call starts from.
fn call_options(prompt: &str) -> VideoCallOptions {
    VideoCallOptions::new(prompt)
}

fn fal_options(value: Value) -> SharedProviderOptions {
    let Value::Object(map) = value else {
        panic!("options must be an object")
    };
    [("fal".to_string(), map)].into_iter().collect()
}

/// The queue answer for a submitted request.
fn queued(server: &MockServer, model_id: &str, request_id: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "request_id": request_id,
        "response_url": format!(
            "{}/fal-ai/{model_id}/requests/{request_id}",
            server.uri()
        ),
    }))
}

/// The status answer of a finished job: the `video` object upstream's
/// `falVideoResponseSchema` accepts, plus the `seed` and `timings` only the
/// provider metadata uses.
fn finished_video(url: &str) -> Value {
    json!({
        "video": {
            "url": url,
            "width": 1920,
            "height": 1080,
            "duration": 5.0,
            "fps": 24,
            "content_type": "video/mp4",
        },
        "seed": 12345,
        "timings": { "inference": 45.5 },
    })
}

/// `server.urls['…/<model_id>'].response = …`.
async fn respond_to_post(server: &MockServer, model_id: &str, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path(format!("/fal-ai/{model_id}")))
        .respond_with(response)
        .with_priority(1)
        .mount(server)
        .await;
}

/// A `GET` route of the mock server.
async fn respond_to_get(server: &MockServer, route: &str, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(route.to_string()))
        .respond_with(response)
        .with_priority(1)
        .mount(server)
        .await;
}

/// `createBasicModel()` plus the `createTestServer` routes of the
/// `luma-dream-machine` queue and of its one queued request.
async fn default_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SUBMIT_PATH))
        .respond_with(queued(&server, MODEL_ID, "test-request-id-123"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(STATUS_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(finished_video(VIDEO_URL)))
        .mount(&server)
        .await;
    server
}

/// The operation reference `doStart` returns; upstream passes only
/// `responseUrl` here.
fn operation(server: &MockServer, request_id: &str) -> Value {
    json!({
        "responseUrl": format!("{}{SUBMIT_PATH}/requests/{request_id}", server.uri()),
        "submitUrl": format!("{}{SUBMIT_PATH}", server.uri()),
    })
}

/// `await server.calls[0].requestBodyJson`: the body of the submit call.
async fn request_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    let post = requests
        .iter()
        .find(|request| request.method.as_str() == "POST")
        .expect("the queue endpoint was called");
    serde_json::from_slice(&post.body).unwrap()
}

/// The videos of a completed status, as JSON.
fn videos(status: &VideoOperationStatus) -> Value {
    let VideoOperationStatus::Completed(result) = status else {
        panic!("expected a completed status, got {status:?}")
    };
    serde_json::to_value(&result.videos).unwrap()
}

/// The PNG magic bytes upstream uses as the input image.
const PNG_MAGIC: [u8; 4] = [137, 80, 78, 71];

mod fal_video_model {
    use super::*;

    mod constructor {
        use super::*;

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › constructor › should expose correct provider and model information
        #[test]
        fn should_expose_correct_provider_and_model_information() {
            let model = video_model("https://queue.fal.run", MODEL_ID);

            assert_eq!(model.provider(), "fal.video");
            assert_eq!(model.model_id(), MODEL_ID);
            assert_eq!(model.specification_version(), "v4");
            assert_eq!(model.max_videos_per_call(), Some(1));
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › constructor › should support different model IDs
        #[test]
        fn should_support_different_model_ids() {
            let model = video_model("https://queue.fal.run", "luma-ray-2");

            assert_eq!(model.model_id(), "luma-ray-2");
        }
    }

    mod do_start {
        use super::*;

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should submit to queue and return operation with responseUrl
        #[tokio::test]
        async fn should_submit_to_queue_and_return_operation_with_response_url() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let result = model.do_start(&call_options(PROMPT)).await.unwrap();

            assert_eq!(
                result.operation,
                json!({
                    "responseUrl": format!("{}{STATUS_PATH}", server.uri()),
                    "submitUrl": format!("{}{SUBMIT_PATH}", server.uri()),
                })
            );
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
            assert_eq!(result.response.model_id.as_deref(), Some(MODEL_ID));
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should pass the correct request body
        #[tokio::test]
        async fn should_pass_the_correct_request_body() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.seed = Some(42);
            options.aspect_ratio = Some(AspectRatio::new(16, 9));
            model.do_start(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({
                    "prompt": PROMPT,
                    "seed": 42,
                    "aspect_ratio": "16:9",
                })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should pass the correct parameters including prompt
        #[tokio::test]
        async fn should_pass_the_correct_parameters_including_prompt() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            model.do_start(&call_options(PROMPT)).await.unwrap();

            assert_eq!(request_body(&server).await, json!({ "prompt": PROMPT }));
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should pass seed when provided
        #[tokio::test]
        async fn should_pass_seed_when_provided() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.seed = Some(42);
            model.do_start(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({ "prompt": PROMPT, "seed": 42 })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should pass aspect ratio when provided
        #[tokio::test]
        async fn should_pass_aspect_ratio_when_provided() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.aspect_ratio = Some(AspectRatio::new(16, 9));
            model.do_start(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({ "prompt": PROMPT, "aspect_ratio": "16:9" })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should convert duration to string format
        #[tokio::test]
        async fn should_convert_duration_to_string_format() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.duration = Some(5);
            model.do_start(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({ "prompt": PROMPT, "duration": "5s" })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let server = default_server().await;
            // Upstream's model sends no `api-key` at all for this case; the
            // credential header is removed here instead.
            let model_with_headers = create_fal(FalProviderSettings {
                api_key: Some(Resolvable::Value("test-key".to_string())),
                base_url: Some(server.uri()),
                headers: Some(
                    [
                        ("Authorization".to_string(), None),
                        (
                            "Custom-Provider-Header".to_string(),
                            Some("provider-header-value".to_string()),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
                ..Default::default()
            })
            .unwrap()
            .video(MODEL_ID);

            let mut options = call_options(PROMPT);
            options.headers = Some(
                [(
                    "Custom-Request-Header".to_string(),
                    "request-header-value".to_string(),
                )]
                .into_iter()
                .collect(),
            );
            model_with_headers.do_start(&options).await.unwrap();

            let requests = server.received_requests().await.unwrap();
            let post = requests
                .iter()
                .find(|request| request.method.as_str() == "POST")
                .unwrap();
            // The test server reports neither the user agent nor the headers
            // the HTTP client adds on its own.
            let headers: serde_json::Map<String, Value> = post
                .headers
                .iter()
                .filter(|(name, _)| {
                    !matches!(
                        name.as_str(),
                        "user-agent" | "host" | "content-length" | "accept" | "accept-encoding"
                    )
                })
                .map(|(name, value)| (name.to_string(), json!(value.to_str().unwrap())))
                .collect();
            assert_eq!(
                Value::Object(headers),
                json!({
                    "content-type": "application/json",
                    "custom-provider-header": "provider-header-value",
                    "custom-request-header": "request-header-value",
                })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should return warnings array
        #[tokio::test]
        async fn should_return_warnings_array() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let result = model.do_start(&call_options(PROMPT)).await.unwrap();

            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should throw error when no response URL returned
        #[tokio::test]
        async fn should_throw_error_when_no_response_url_returned() {
            let server = default_server().await;
            respond_to_post(
                &server,
                MODEL_ID,
                ResponseTemplate::new(200).set_body_json(json!({})),
            )
            .await;
            let model = create_basic_model(&server);

            let error = model.do_start(&call_options(PROMPT)).await.unwrap_err();

            let AiMuxError::InvalidResponseData { message, .. } = error else {
                panic!("expected an invalid response data error, got {error:?}")
            };
            assert_eq!(message, "No response URL returned from queue endpoint");
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should handle API errors from queue endpoint
        #[tokio::test]
        async fn should_handle_api_errors_from_queue_endpoint() {
            let server = default_server().await;
            respond_to_post(
                &server,
                MODEL_ID,
                ResponseTemplate::new(400).set_body_string(
                    json!({ "error": { "message": "Invalid prompt" } }).to_string(),
                ),
            )
            .await;
            let model = create_basic_model(&server);

            let error = model.do_start(&call_options(PROMPT)).await.unwrap_err();

            let AiMuxError::ApiCall(error) = error else {
                panic!("expected an API call error, got {error:?}")
            };
            assert_eq!(error.status_code, Some(400));
            assert_eq!(error.message, "Invalid prompt");
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should send image_url with file data
        #[tokio::test]
        async fn should_send_image_url_with_file_data() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.image = Some(VideoFile::File {
                media_type: "image/png".to_string(),
                data: VideoFileData::Binary(PNG_MAGIC.to_vec()),
            });
            model.do_start(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({
                    "prompt": PROMPT,
                    "image_url": "data:image/png;base64,iVBORw==",
                })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should send image_url with URL-based image
        #[tokio::test]
        async fn should_send_image_url_with_url_based_image() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.image = Some(VideoFile::Url {
                url: "https://example.com/input-image.png".to_string(),
                media_type: None,
            });
            model.do_start(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({
                    "prompt": PROMPT,
                    "image_url": "https://example.com/input-image.png",
                })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should pass loop option
        #[tokio::test]
        async fn should_pass_loop_option() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.provider_options = fal_options(json!({ "loop": true }));
            model.do_start(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({ "prompt": PROMPT, "loop": true })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should pass motionStrength as motion_strength
        #[tokio::test]
        async fn should_pass_motion_strength_as_motion_strength() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.provider_options = fal_options(json!({ "motionStrength": 0.8 }));
            model.do_start(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({ "prompt": PROMPT, "motion_strength": 0.8 })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should pass resolution option
        #[tokio::test]
        async fn should_pass_resolution_option() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/fal-ai/luma-ray-2"))
                .respond_with(queued(&server, "luma-ray-2", "ray2-request-id"))
                .mount(&server)
                .await;
            let model = video_model(&server.uri(), "luma-ray-2");

            let mut options = call_options(PROMPT);
            options.provider_options = fal_options(json!({ "resolution": "1080p" }));
            model.do_start(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({ "prompt": PROMPT, "resolution": "1080p" })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should pass negativePrompt as negative_prompt
        #[tokio::test]
        async fn should_pass_negative_prompt_as_negative_prompt() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.provider_options =
                fal_options(json!({ "negativePrompt": "blurry, low quality" }));
            model.do_start(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({ "prompt": PROMPT, "negative_prompt": "blurry, low quality" })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should pass promptOptimizer as prompt_optimizer
        #[tokio::test]
        async fn should_pass_prompt_optimizer_as_prompt_optimizer() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.provider_options = fal_options(json!({ "promptOptimizer": true }));
            model.do_start(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({ "prompt": PROMPT, "prompt_optimizer": true })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStart › should pass through additional options
        #[tokio::test]
        async fn should_pass_through_additional_options() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.provider_options = fal_options(json!({
                "custom_param": "custom_value",
                "another_param": 123,
            }));
            model.do_start(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({
                    "prompt": PROMPT,
                    "custom_param": "custom_value",
                    "another_param": 123,
                })
            );
        }
    }

    mod do_status {
        use super::*;

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStatus › should return completed status with video data
        ///
        /// The case also asserts `result.providerMetadata?.fal` is defined;
        /// that half has its own case, `should_include_provider_metadata_in_
        /// completed_status`.
        #[tokio::test]
        async fn should_return_completed_status_with_video_data() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let status = model
                .do_status(
                    &operation(&server, "test-request-id-123"),
                    &call_options(PROMPT),
                )
                .await
                .unwrap();

            assert!(
                matches!(status, VideoOperationStatus::Completed(_)),
                "{status:?}"
            );
            assert_eq!(
                videos(&status),
                json!([{ "type": "url", "url": VIDEO_URL, "mediaType": "video/mp4" }])
            );
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStatus › should return pending status when request is still in progress
        #[tokio::test]
        async fn should_return_pending_status_when_request_is_still_in_progress() {
            let server = MockServer::start().await;
            let status_path = "/fal-ai/luma-dream-machine/requests/test-id";
            respond_to_get(
                &server,
                status_path,
                ResponseTemplate::new(500).set_body_string(
                    json!({ "detail": "Request is still in progress" }).to_string(),
                ),
            )
            .await;
            let model = create_basic_model(&server);

            let status = model
                .do_status(&operation(&server, "test-id"), &call_options(PROMPT))
                .await
                .unwrap();

            let requests = server.received_requests().await.unwrap();
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request.method.as_str() == "GET")
                    .count(),
                1
            );
            assert!(
                matches!(status, VideoOperationStatus::Pending),
                "{status:?}"
            );
        }

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStatus › should include provider metadata in completed status
        #[tokio::test]
        async fn should_include_provider_metadata_in_completed_status() {
            let server = default_server().await;
            let model = create_basic_model(&server);

            let status = model
                .do_status(
                    &operation(&server, "test-request-id-123"),
                    &call_options(PROMPT),
                )
                .await
                .unwrap();
            let VideoOperationStatus::Completed(result) = status else {
                panic!("expected a completed status, got {status:?}")
            };

            assert_eq!(
                serde_json::to_value(result.provider_metadata.as_ref().unwrap()).unwrap(),
                json!({
                    "fal": {
                        "videos": [{
                            "url": VIDEO_URL,
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

        /// upstream: @ai-sdk/fal src/fal-video-model.test.ts › FalVideoModel › doStatus › should throw error when video URL is missing in completed response
        #[tokio::test]
        async fn should_throw_error_when_video_url_is_missing_in_completed_response() {
            let server = default_server().await;
            respond_to_get(
                &server,
                STATUS_PATH,
                ResponseTemplate::new(200).set_body_json(json!({})),
            )
            .await;
            let model = create_basic_model(&server);

            let error = model
                .do_status(
                    &operation(&server, "test-request-id-123"),
                    &call_options(PROMPT),
                )
                .await
                .unwrap_err();

            let AiMuxError::InvalidResponseData { message, .. } = error else {
                panic!("expected an invalid response data error, got {error:?}")
            };
            assert_eq!(message, "No video URL in response");
        }
    }
}

// ── Status poll origins (#441; not upstream: no fetchStatus test checks them) ──

/// The `Key` credential is gated on the submit origin, so a `response_url` on
/// another origin is polled without it. The mock server cannot show this: the
/// SSRF guard rejects an untrusted loopback target before it connects. The
/// gate is `HttpRequest::credentialed_origin`, and that it drops the caller
/// headers is tested in `aimux-provider-utils` (`http.rs`).
#[test]
fn a_status_poll_is_credentialed_and_trusted_only_on_the_submit_origin() {
    let submit_url = "https://queue.fal.run/fal-ai/luma-dream-machine";
    let request = crate::fal::video_model::status_request(
        aimux_provider_utils::HttpRequest {
            url: "https://elsewhere.example/requests/1/status".into(),
            headers: vec![("authorization".into(), "Key test-api-key".into())],
            ..aimux_provider_utils::HttpRequest::default()
        },
        submit_url,
    );

    assert!(request.validate_url);
    assert_eq!(request.credentialed_origin.as_deref(), Some(submit_url));
    assert_eq!(request.trusted_origin.as_deref(), Some(submit_url));
}
