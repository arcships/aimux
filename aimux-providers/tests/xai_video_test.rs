//! Rust translation of the xAI video model tests.
//! Source: `reference/aisdk-pinned/xai/src/xai-video-model.test.ts`
//!
//! The HTTP layer is mocked with wiremock. Requests are inspected through
//! `received_requests`; `do_start` / `do_status` are called directly.

use std::collections::HashMap;

use serde_json::{Map, Value, json};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{AspectRatio, Size, Warning};
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoFileData, VideoFrameImage, VideoFrameType,
    VideoModel, VideoOperationStart, VideoOperationStatus, VideoResult,
};
use aimux_providers::{XAIProviderSettings, XaiVideoModel, create_xai};

const PROMPT: &str = "A chicken flying into the sunset";
const SRC: &str = "https://example.com/source-video.mp4";
const VIDEO_URL: &str = "https://vidgen.x.ai/output/video-001.mp4";
const V15: &str = "grok-imagine-video-1.5";
const V1: &str = "grok-imagine-video";

// ---------------------------------------------------------------- helpers

fn done_status() -> Value {
    json!({
        "status": "done",
        "video": {"url": VIDEO_URL, "duration": 5, "respect_moderation": true},
        "model": V1,
        "progress": 100
    })
}

fn json_response(body: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

async fn server_with(post: Value, status: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(json_response(post))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(status)
        .mount(&server)
        .await;
    server
}

fn model_with(server: &MockServer, model_id: &str, headers: &[(&str, &str)]) -> XaiVideoModel {
    create_xai(XAIProviderSettings {
        api_key: Some("test-key".into()),
        base_url: Some(server.uri()),
        headers: (!headers.is_empty()).then(|| {
            headers
                .iter()
                .map(|(k, v)| ((*k).to_string(), Some((*v).to_string())))
                .collect()
        }),
        ..Default::default()
    })
    .unwrap()
    .video(model_id)
}

fn base() -> VideoCallOptions {
    VideoCallOptions::new(PROMPT)
}

/// Sets `providerOptions.xai`.
fn xai(mut options: VideoCallOptions, value: Value) -> VideoCallOptions {
    options
        .provider_options
        .insert("xai".into(), value.as_object().unwrap().clone());
    options
}

fn ar(s: &str) -> Option<AspectRatio> {
    Some(AspectRatio::parse(s).unwrap())
}

fn size(s: &str) -> Option<Size> {
    Some(Size::parse(s).unwrap())
}

fn url_file(url: &str, media_type: Option<&str>) -> VideoFile {
    VideoFile::Url {
        url: url.into(),
        media_type: media_type.map(str::to_owned),
    }
}

fn last_frame(file: VideoFile) -> Option<Vec<VideoFrameImage>> {
    Some(vec![VideoFrameImage {
        image: file,
        frame_type: VideoFrameType::LastFrame,
    }])
}

fn header(request: &Request, name: &str) -> String {
    request
        .headers
        .get(name)
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default()
}

struct Started {
    start: VideoOperationStart,
    method: String,
    path: String,
    body: Value,
}

async fn try_start(model_id: &str, options: &VideoCallOptions) -> Result<Started, AiMuxError> {
    let server = server_with(
        json!({"request_id": "req-123"}),
        json_response(done_status()),
    )
    .await;
    let start = model_with(&server, model_id, &[]).do_start(options).await?;
    let request = server.received_requests().await.unwrap().remove(0);
    Ok(Started {
        start,
        method: request.method.to_string(),
        path: request.url.path().to_owned(),
        body: serde_json::from_slice(&request.body).unwrap(),
    })
}

async fn start(model_id: &str, options: VideoCallOptions) -> Started {
    try_start(model_id, &options).await.unwrap()
}

/// The `details` of every `Unsupported` warning for `feature`.
fn details<'a>(warnings: &'a [Warning], feature: &str) -> Vec<&'a str> {
    warnings
        .iter()
        .filter_map(|w| match w {
            Warning::Unsupported {
                feature: f,
                details,
            } if f == feature => Some(details.as_deref().unwrap_or("")),
            _ => None,
        })
        .collect()
}

fn warned(warnings: &[Warning], feature: &str) -> bool {
    !details(warnings, feature).is_empty()
}

async fn poll_once(
    status: ResponseTemplate,
    request_id: &str,
) -> (Result<VideoOperationStatus, AiMuxError>, Vec<Request>) {
    let server = server_with(json!({"request_id": "req-123"}), status).await;
    let result = model_with(&server, V1, &[])
        .do_status(&json!({ "requestId": request_id }), &base())
        .await;
    (result, server.received_requests().await.unwrap())
}

fn completed(result: Result<VideoOperationStatus, AiMuxError>) -> VideoResult {
    match result {
        Ok(VideoOperationStatus::Completed(done)) => done,
        other => panic!("expected completed, got {other:?}"),
    }
}

fn xai_meta(result: &VideoResult) -> Map<String, Value> {
    result.provider_metadata.as_ref().unwrap()["xai"].clone()
}

fn assert_terminal(result: Result<VideoOperationStatus, AiMuxError>, message: &str) {
    match result {
        Err(AiMuxError::ApiCall(e)) => {
            assert_eq!(e.message, message);
            assert!(!e.is_retryable);
            assert_eq!(e.status_code, Some(200));
        }
        other => panic!("expected terminal api error, got {other:?}"),
    }
}

mod xai_video_model {
    use super::*;

    mod constructor {
        use super::*;

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › constructor › should send the grok-imagine-video-1.5 model id in the request body
        #[tokio::test]
        async fn sends_the_1_5_model_id_in_the_body() {
            let s = start(V15, base()).await;
            assert_eq!(s.body["model"], V15);
        }
    }

    mod do_start {
        use super::*;

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should map current xAI generation options
        #[tokio::test]
        async fn maps_current_generation_options() {
            let mut o = xai(
                base(),
                json!({
                    "storageOptions": {
                        "filename": "result.mp4",
                        "expiresAfter": 86_400,
                        "publicUrl": {"expiresAfter": 3_600}
                    },
                    "keyframes": [{"imageUrl": "https://example.com/middle.png", "timestampSeconds": 2.5}]
                }),
            );
            o.generate_audio = Some(false);
            o.frame_images = last_frame(url_file("https://example.com/end.png", None));
            let s = start(V15, o).await;
            assert_eq!(s.body["generate_audio"], false);
            assert_eq!(
                s.body["last_frame"],
                json!({"url": "https://example.com/end.png"})
            );
            assert_eq!(
                s.body["storage_options"],
                json!({
                    "filename": "result.mp4",
                    "expires_after": 86_400,
                    "public_url": {"expires_after": 3_600}
                })
            );
            assert_eq!(
                s.body["keyframes"],
                json!([{"image": {"url": "https://example.com/middle.png"}, "timestamp_s": 2.5}])
            );
            assert_eq!(s.start.operation, json!({"requestId": "req-123"}));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should warn and omit last_frame for grok-imagine-video
        #[tokio::test]
        async fn warns_and_omits_last_frame_for_grok_imagine_video() {
            let mut o = base();
            o.frame_images = last_frame(url_file("https://example.com/end.png", None));
            let s = start(V1, o).await;
            assert!(s.body.get("last_frame").is_none());
            assert_eq!(
                details(&s.start.warnings, "frameImages"),
                [
                    "xAI only supports last_frame with \"grok-imagine-video-1.5\". \
                     The last frame was ignored."
                ]
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should pass headers
        #[tokio::test]
        async fn do_start_passes_headers() {
            let server = server_with(
                json!({"request_id": "req-123"}),
                json_response(done_status()),
            )
            .await;
            let model = model_with(&server, V1, &[("X-Custom", "value")]);
            let mut o = base();
            o.headers = Some(HashMap::from([(
                "X-Request-Header".to_string(),
                "request-value".to_string(),
            )]));
            model.do_start(&o).await.unwrap();
            let request = server.received_requests().await.unwrap().remove(0);
            assert_eq!(header(&request, "authorization"), "Bearer test-key");
            assert_eq!(header(&request, "x-custom"), "value");
            assert_eq!(header(&request, "x-request-header"), "request-value");
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should throw when no request_id returned
        #[tokio::test]
        async fn do_start_fails_without_request_id() {
            let server = server_with(json!({}), json_response(done_status())).await;
            let err = model_with(&server, V1, &[])
                .do_start(&base())
                .await
                .unwrap_err();
            match err {
                AiMuxError::Other { message, .. } => {
                    assert_eq!(message, "No request_id returned from xAI API.")
                }
                other => panic!("unexpected error: {other:?}"),
            }
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should return warnings for unsupported features
        #[tokio::test]
        async fn warns_for_fps_and_seed() {
            let mut o = base();
            o.fps = Some(30);
            o.seed = Some(42);
            let s = start(V1, o).await;
            assert!(warned(&s.start.warnings, "fps"));
            assert!(warned(&s.start.warnings, "seed"));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should warn when explicit R2V has no references at all
        #[tokio::test]
        async fn warns_when_reference_to_video_has_no_references() {
            let s = start(V15, xai(base(), json!({"mode": "reference-to-video"}))).await;
            assert!(s.body.get("reference_images").is_none());
            assert!(
                details(&s.start.warnings, "referenceImages")[0]
                    .contains("without reference images")
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should allow duration and aspectRatio with reference images
        #[tokio::test]
        async fn allows_duration_and_aspect_ratio_with_reference_images() {
            let mut o = xai(
                base(),
                json!({"referenceImageUrls": ["https://example.com/ref.jpg"]}),
            );
            o.duration = Some(8);
            o.aspect_ratio = ar("16:9");
            let s = start(V1, o).await;
            assert_eq!(s.body["duration"], 8);
            assert_eq!(s.body["aspect_ratio"], "16:9");
            assert!(!warned(&s.start.warnings, "duration"));
            assert!(!warned(&s.start.warnings, "aspectRatio"));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should omit reference_audios for an empty referenceVoiceIds array
        #[tokio::test]
        async fn omits_reference_audios_for_empty_voice_ids() {
            let o = xai(
                base(),
                json!({
                    "mode": "reference-to-video",
                    "referenceImageUrls": ["https://example.com/ref1.jpg"],
                    "referenceVoiceIds": []
                }),
            );
            let s = start(V15, o).await;
            assert!(s.body.get("reference_audios").is_none());
            assert!(!warned(&s.start.warnings, "referenceVoiceIds"));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should warn and omit referenceVoiceIds outside reference-to-video
        #[tokio::test]
        async fn warns_and_omits_voice_ids_outside_reference_to_video() {
            let s = start(V15, xai(base(), json!({"referenceVoiceIds": ["eve"]}))).await;
            assert!(s.body.get("reference_audios").is_none());
            assert!(warned(&s.start.warnings, "referenceVoiceIds"));
        }
    }

    mod do_status {
        use super::*;

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStatus › should return pending when status is pending
        #[tokio::test]
        async fn do_status_returns_pending() {
            for template in [
                json_response(json!({"status": "pending"})),
                // xAI answers 202 while running, sometimes with an empty body.
                ResponseTemplate::new(202),
                ResponseTemplate::new(202)
                    .set_body_json(json!({"status": "pending", "progress": 40})),
            ] {
                let (result, _) = poll_once(template, "req-123").await;
                assert!(
                    matches!(result, Ok(VideoOperationStatus::Pending)),
                    "{result:?}"
                );
            }
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStatus › should pass headers to request
        #[tokio::test]
        async fn do_status_passes_headers() {
            let server = server_with(json!({}), json_response(done_status())).await;
            let model = model_with(&server, V1, &[("X-Custom", "value")]);
            let mut o = base();
            o.headers = Some(HashMap::from([(
                "X-Request-Header".to_string(),
                "request-value".to_string(),
            )]));
            model
                .do_status(&json!({"requestId": "req-123"}), &o)
                .await
                .unwrap();
            let request = server.received_requests().await.unwrap().remove(0);
            assert_eq!(header(&request, "authorization"), "Bearer test-key");
            assert_eq!(header(&request, "x-custom"), "value");
            assert_eq!(header(&request, "x-request-header"), "request-value");
        }
    }
}

/// Checks the provider id, model id and maxVideosPerCall; specificationVersion is not checked. partial: should expose correct provider and model information; full port in src/xai/tests/
#[tokio::test]
async fn constructor_exposes_provider_and_model_information() {
    let server = MockServer::start().await;
    let model = model_with(&server, V1, &[]);
    assert_eq!(model.provider(), "xai.video");
    assert_eq!(model.model_id(), V1);
    assert_eq!(model.max_videos_per_call(), Some(1));
}

/// Checks the requestId operation and the response model id, timestamp and headers. partial: should return operation with requestId; should include response metadata; full port in src/xai/tests/
#[tokio::test]
async fn do_start_returns_request_id_and_response_metadata() {
    let s = start(V1, base()).await;
    assert_eq!(s.start.operation, json!({"requestId": "req-123"}));
    let response = &s.start.response;
    assert_eq!(response.model_id.as_deref(), Some(V1));
    assert!(response.timestamp.is_some());
    assert!(response.headers.is_some());
}

/// Checks the POST to /videos/generations with the minimal body and no warnings. partial: should pass correct request body; should return empty warnings for supported features; full port in src/xai/tests/
#[tokio::test]
async fn do_start_posts_minimal_generation_body() {
    let s = start(V1, base()).await;
    assert_eq!(s.method, "POST");
    assert_eq!(s.path, "/videos/generations");
    assert_eq!(s.body, json!({"model": V1, "prompt": PROMPT}));
    assert!(s.start.warnings.is_empty());
}

/// Checks the POST path and video field for legacy, edit and extend modes. partial: should use edits endpoint for video editing (legacy videoUrl without mode); should use edits endpoint for video editing with explicit mode; should use extensions endpoint for extend-video mode; full port in src/xai/tests/
#[tokio::test]
async fn selects_endpoint_by_mode() {
    let cases = [
        (json!({"videoUrl": SRC}), "/videos/edits"),
        (
            json!({"mode": "edit-video", "videoUrl": SRC}),
            "/videos/edits",
        ),
        (
            json!({"mode": "extend-video", "videoUrl": SRC}),
            "/videos/extensions",
        ),
    ];
    for (options, path) in cases {
        let s = start(V1, xai(base(), options)).await;
        assert_eq!(s.method, "POST");
        assert_eq!(s.path, path);
        assert_eq!(s.body["video"], json!({"url": SRC}));
    }
}

/// Checks the n warning for n=3 and its absence for n=1. partial: should warn when n > 1; should not warn when n is 1; full port in src/xai/tests/
#[tokio::test]
async fn warns_only_when_n_exceeds_one() {
    for (n, expected) in [(3, true), (1, false)] {
        let mut o = base();
        o.n = n;
        let s = start(V1, o).await;
        assert_eq!(warned(&s.start.warnings, "n"), expected, "n = {n}");
    }
}

/// Checks that duration and aspect_ratio are sent in the body. partial: should send duration in request body; should send aspect_ratio in request body; full port in src/xai/tests/
#[tokio::test]
async fn sends_duration_and_aspect_ratio() {
    let mut o = base();
    o.duration = Some(10);
    o.aspect_ratio = ar("9:16");
    let s = start(V1, o).await;
    assert_eq!(s.body["duration"], 10);
    assert_eq!(s.body["aspect_ratio"], "9:16");
}

/// Checks that SDK sizes map to 720p, 480p and 1080p. partial: should map SDK resolution 1280x720 to 720p; should map SDK resolution 854x480 to 480p; should map SDK resolution 640x480 to 480p; should map SDK resolution 1920x1080 to 1080p; full port in src/xai/tests/
#[tokio::test]
async fn maps_sdk_resolution_to_xai_resolution() {
    let cases = [
        (V1, "1280x720", "720p"),
        (V1, "854x480", "480p"),
        (V1, "640x480", "480p"),
        (V15, "1920x1080", "1080p"),
    ];
    for (model, resolution, expected) in cases {
        let mut o = base();
        o.resolution = size(resolution);
        let s = start(model, o).await;
        assert_eq!(s.body["resolution"], expected, "{resolution}");
    }
}

/// Checks that a provider resolution is sent and wins over the SDK size. partial: should pass through provider option resolution 1080p; should prefer provider option resolution over SDK resolution; full port in src/xai/tests/
#[tokio::test]
async fn provider_option_resolution_wins() {
    let s = start(V15, xai(base(), json!({"resolution": "1080p"}))).await;
    assert_eq!(s.body["resolution"], "1080p");

    let mut o = xai(base(), json!({"resolution": "480p"}));
    o.resolution = size("1280x720");
    let s = start(V1, o).await;
    assert_eq!(s.body["resolution"], "480p");
}

/// Checks the 1080p warning on grok-imagine-video (SDK and provider sources) and none on 1.5. partial: should warn when SDK resolution 1920x1080 is used with grok-imagine-video; should warn when provider resolution 1080p is used with grok-imagine-video; should not warn about 1080p with grok-imagine-video-1.5; full port in src/xai/tests/
#[tokio::test]
async fn warns_about_1080p_only_on_grok_imagine_video() {
    let mut sdk = base();
    sdk.resolution = size("1920x1080");
    let provider = xai(base(), json!({"resolution": "1080p"}));
    for options in [sdk, provider] {
        let s = start(V1, options).await;
        assert_eq!(s.body["resolution"], "1080p");
        assert!(
            details(&s.start.warnings, "resolution")[0].contains("does not support 1080p"),
            "{:?}",
            s.start.warnings
        );
    }
    let mut o = base();
    o.resolution = size("1920x1080");
    assert!(start(V15, o).await.start.warnings.is_empty());
}

/// Checks that unrecognized resolutions are left out of the body and warned about. partial: should warn for unrecognized resolution format; should warn and omit body resolution for completely unknown format; full port in src/xai/tests/
#[tokio::test]
async fn warns_and_omits_unrecognized_resolution() {
    for resolution in ["2560x1440", "3840x2160"] {
        let mut o = base();
        o.resolution = size(resolution);
        let s = start(V1, o).await;
        assert!(s.body.get("resolution").is_none());
        assert!(warned(&s.start.warnings, "resolution"), "{resolution}");
    }
}

/// Checks that URL, binary and base64 image inputs are sent as image.url. partial: should send image object from URL-based image input; should send image object with data URI from file data; should send image object with data URI from base64 string; full port in src/xai/tests/
#[tokio::test]
async fn sends_image_object_from_url_and_file_inputs() {
    let cases = [
        (
            url_file("https://example.com/image.png", None),
            "https://example.com/image.png",
        ),
        (
            VideoFile::File {
                media_type: "image/png".into(),
                data: VideoFileData::Binary(vec![137, 80, 78, 71]),
            },
            "data:image/png;base64,iVBORw==",
        ),
        (
            VideoFile::File {
                media_type: "image/jpeg".into(),
                data: VideoFileData::Base64("aGVsbG8=".into()),
            },
            "data:image/jpeg;base64,aGVsbG8=",
        ),
    ];
    for (image, expected) in cases {
        let mut o = base();
        o.image = Some(image);
        let s = start(V1, o).await;
        assert_eq!(s.body["image"], json!({"url": expected}));
    }
}

/// Checks which options warn in default, edit and extend modes. partial: should warn about duration in edit mode; should warn about aspectRatio in edit mode; should warn about resolution in edit mode; should not warn about duration outside edit mode; should not warn about aspectRatio outside edit mode; should not warn about resolution outside edit mode; should warn about aspectRatio in extension mode; should warn about resolution in extension mode; should not warn about duration in extension mode; full port in src/xai/tests/
#[tokio::test]
async fn warns_about_unsupported_options_per_mode() {
    type Setter = fn(&mut VideoCallOptions);
    let setters: [(&str, Setter); 3] = [
        ("duration", |o| o.duration = Some(10)),
        ("aspectRatio", |o| o.aspect_ratio = ar("16:9")),
        ("resolution", |o| o.resolution = size("1280x720")),
    ];
    for mode in [None, Some("edit-video"), Some("extend-video")] {
        for (feature, set) in setters {
            let mut o = base();
            set(&mut o);
            if let Some(mode) = mode {
                o = xai(o, json!({"mode": mode, "videoUrl": SRC}));
            }
            let expected = matches!(
                (mode, feature),
                (Some("edit-video"), _) | (Some("extend-video"), "aspectRatio" | "resolution")
            );
            let s = start(V1, o).await;
            assert_eq!(
                warned(&s.start.warnings, feature),
                expected,
                "{mode:?} {feature}"
            );
        }
    }
}

/// Checks that unsupported fields are left out of edit and extend bodies. partial: should omit duration, aspect_ratio, and resolution from body in edit mode; should allow duration in extension mode; should omit aspect_ratio and resolution from body in extension mode; should warn about provider-level resolution in extension mode; full port in src/xai/tests/
#[tokio::test]
async fn omits_unsupported_fields_from_edit_and_extension_bodies() {
    let mut edit = xai(base(), json!({"mode": "edit-video", "videoUrl": SRC}));
    edit.duration = Some(10);
    edit.aspect_ratio = ar("16:9");
    edit.resolution = size("1280x720");
    let s = start(V1, edit).await;
    for key in ["duration", "aspect_ratio", "resolution"] {
        assert!(s.body.get(key).is_none(), "edit body has {key}");
    }

    let mut extend = xai(base(), json!({"mode": "extend-video", "videoUrl": SRC}));
    extend.duration = Some(6);
    extend.aspect_ratio = ar("16:9");
    extend.resolution = size("1280x720");
    let s = start(V1, extend).await;
    assert_eq!(s.body["duration"], 6);
    assert!(s.body.get("aspect_ratio").is_none());
    assert!(s.body.get("resolution").is_none());

    let provider_resolution = xai(
        base(),
        json!({"mode": "extend-video", "videoUrl": SRC, "resolution": "720p"}),
    );
    let s = start(V1, provider_resolution).await;
    assert!(s.body.get("resolution").is_none());
    assert!(warned(&s.start.warnings, "resolution"));
}

/// Checks reference_images for explicit and implicit reference-to-video. partial: should send reference_images array to /videos/generations with explicit mode; should fallback to reference-to-video mode when referenceImageUrls is set without mode; full port in src/xai/tests/
#[tokio::test]
async fn sends_reference_images_for_reference_to_video() {
    let urls = [
        "https://example.com/ref1.jpg",
        "https://example.com/ref2.jpg",
    ];
    let explicit = xai(
        base(),
        json!({"mode": "reference-to-video", "referenceImageUrls": urls}),
    );
    let implicit = xai(base(), json!({"referenceImageUrls": urls}));
    for options in [explicit, implicit] {
        let s = start(V1, options).await;
        assert_eq!(s.method, "POST");
        assert_eq!(s.path, "/videos/generations");
        assert_eq!(
            s.body["reference_images"],
            json!([{"url": urls[0]}, {"url": urls[1]}])
        );
    }
}

/// Checks that 1080p is downgraded to 720p in reference-to-video with a warning. partial: should downgrade R2V 1080p to 720p with a warning; should downgrade R2V 1920x1080 from the SDK resolution to 720p; full port in src/xai/tests/
#[tokio::test]
async fn downgrades_reference_to_video_1080p() {
    let refs = json!(["https://example.com/ref1.jpg"]);
    let provider = xai(
        base(),
        json!({"mode": "reference-to-video", "referenceImageUrls": refs, "resolution": "1080p"}),
    );
    let mut sdk = xai(
        base(),
        json!({"mode": "reference-to-video", "referenceImageUrls": refs}),
    );
    sdk.resolution = size("1920x1080");
    for options in [provider, sdk] {
        let s = start(V15, options).await;
        assert_eq!(s.body["resolution"], "720p");
        assert!(warned(&s.start.warnings, "resolution"));
    }
}

/// Checks that inputReferences split into reference_images and reference_audios by media type. partial: should separate image and audio inputReferences; should support audio-only reference-to-video; should not send an empty reference_images array for video-only inputReferences; should combine a pinned first frame with an audio reference; should support explicit audio-only R2V without reference_images; full port in src/xai/tests/
#[tokio::test]
async fn splits_input_references_by_media_type() {
    let image = url_file("https://example.com/ref1.jpg", None);
    let audio = url_file("https://example.com/voice.mp3", Some("audio/mpeg"));
    let video = url_file("https://example.com/clip.mp4", Some("video/mp4"));
    let start_image = url_file("https://example.com/start.jpg", Some("image/jpeg"));
    let voice = json!([{"url": "https://example.com/voice.mp3"}]);

    let mut o = base();
    o.input_references = Some(vec![image, audio.clone()]);
    let s = start(V15, o).await;
    assert_eq!(
        s.body["reference_images"],
        json!([{"url": "https://example.com/ref1.jpg"}])
    );
    assert_eq!(s.body["reference_audios"], voice);
    assert!(s.start.warnings.is_empty());

    let mut o = base();
    o.input_references = Some(vec![audio.clone()]);
    let s = start(V15, o).await;
    assert_eq!(s.body["reference_audios"], voice);
    assert!(s.body.get("reference_images").is_none());
    assert!(s.start.warnings.is_empty());

    let mut o = base();
    o.input_references = Some(vec![video]);
    let s = start(V15, o).await;
    assert!(s.body.get("reference_images").is_none());

    let mut o = base();
    o.image = Some(start_image);
    o.input_references = Some(vec![audio.clone()]);
    let s = start(V15, o).await;
    assert_eq!(
        s.body["image"],
        json!({"url": "https://example.com/start.jpg"})
    );
    assert_eq!(s.body["reference_audios"], voice);
    assert!(s.body.get("reference_images").is_none());
    assert!(s.start.warnings.is_empty());

    let mut o = xai(base(), json!({"mode": "reference-to-video"}));
    o.input_references = Some(vec![audio]);
    let s = start(V15, o).await;
    assert_eq!(s.body["reference_audios"], voice);
    assert!(s.body.get("reference_images").is_none());
    assert!(s.start.warnings.is_empty());
}

/// Checks that referenceVoiceIds become reference_audios in order and are never sent raw. partial: should send reference_audios for preset reference voices; should send up to 3 preset reference voices in order; should never forward referenceVoiceIds as a raw body key; full port in src/xai/tests/
#[tokio::test]
async fn sends_reference_voice_ids_as_reference_audios() {
    let cases = [
        (json!(["eve"]), json!([{"voice_id": "eve"}])),
        (
            json!(["eve", "leo", "rex"]),
            json!([{"voice_id": "eve"}, {"voice_id": "leo"}, {"voice_id": "rex"}]),
        ),
    ];
    for (ids, expected) in cases {
        let o = xai(
            base(),
            json!({
                "mode": "reference-to-video",
                "referenceImageUrls": ["https://example.com/ref1.jpg"],
                "referenceVoiceIds": ids
            }),
        );
        let s = start(V15, o).await;
        assert_eq!(s.path, "/videos/generations");
        assert_eq!(
            s.body["reference_images"],
            json!([{"url": "https://example.com/ref1.jpg"}])
        );
        assert_eq!(s.body["reference_audios"], expected);
        assert!(s.body.get("referenceVoiceIds").is_none());
    }
}

/// Checks that more than 3 voices, or an empty voice id, gives InvalidArgument. partial: should reject more than 3 preset reference voices; should reject empty-string preset reference voice ids; full port in src/xai/tests/
#[tokio::test]
async fn rejects_invalid_reference_voice_ids() {
    for ids in [json!(["ara", "eve", "leo", "rex"]), json!([""])] {
        let o = xai(
            base(),
            json!({
                "mode": "reference-to-video",
                "referenceImageUrls": ["https://example.com/ref1.jpg"],
                "referenceVoiceIds": ids
            }),
        );
        match try_start(V15, &o).await {
            Err(AiMuxError::InvalidArgument { .. }) => {}
            Err(other) => panic!("unexpected error: {other:?}"),
            Ok(_) => panic!("expected InvalidArgument for {ids}"),
        }
    }
}

/// Checks that the requestId is encoded as one URL path segment, dot segments included. partial: should encode the request ID as a single URL path segment; should preserve the $requestId request ID as a URL path segment; full port in src/xai/tests/
#[tokio::test]
async fn do_status_encodes_request_id_as_one_path_segment() {
    let cases = [
        ("abc/../../internal", "/videos/abc%2F..%2F..%2Finternal"),
        (".", "/videos/%252E"),
        ("..", "/videos/%252E%252E"),
    ];
    for (request_id, expected) in cases {
        let (_, requests) = poll_once(json_response(done_status()), request_id).await;
        assert_eq!(requests[0].method.as_str(), "GET");
        assert_eq!(requests[0].url.path(), expected);
    }
}

/// Checks the completed video, its xai metadata and the response. partial: should return completed with video data when done; should include response metadata; full port in src/xai/tests/
#[tokio::test]
async fn do_status_returns_completed_video_and_metadata() {
    let (result, _) = poll_once(json_response(done_status()), "req-123").await;
    let done = completed(result);
    assert_eq!(done.videos.len(), 1);
    match &done.videos[0] {
        VideoData::Url { url, media_type } => {
            assert_eq!(url, VIDEO_URL);
            assert_eq!(media_type, "video/mp4");
        }
        other => panic!("expected url, got {other:?}"),
    }
    let meta = xai_meta(&done);
    assert_eq!(meta.len(), 4);
    assert_eq!(meta["requestId"], "req-123");
    assert_eq!(meta["videoUrl"], VIDEO_URL);
    assert_eq!(meta["duration"].as_f64(), Some(5.0));
    assert_eq!(meta["progress"].as_f64(), Some(100.0));
    assert_eq!(done.response.model_id.as_deref(), Some(V1));
    assert!(done.response.timestamp.is_some());
    assert!(done.response.headers.is_some());
}

/// Checks that expired, failed, missing-URL and blocked statuses give non-retryable errors. partial: should return error status on expired; should return error status on failed; should report an error status when video URL missing on done; should report an error status when respect_moderation is false; full port in src/xai/tests/
#[tokio::test]
async fn do_status_reports_terminal_failures_as_non_retryable_errors() {
    let cases = [
        (
            json!({"status": "expired", "model": V1}),
            "Video generation request expired.",
        ),
        (
            json!({"status": "failed", "model": V1, "progress": 0,
                   "error": {"message": "Content policy violation"}}),
            "Video generation failed: Content policy violation",
        ),
        (
            json!({"status": "done", "video": null, "model": V1}),
            "Video generation completed but no video URL was returned.",
        ),
        (
            json!({"status": "done", "video": {"url": "", "respect_moderation": false}, "model": V1}),
            "Video generation was blocked due to a content policy violation.",
        ),
    ];
    for (body, message) in cases {
        let (result, _) = poll_once(json_response(body), "req-123").await;
        assert_terminal(result, message);
    }
}

/// Checks that costInUsdTicks is added and that duration and progress are omitted when absent. partial: should include costInUsdTicks when returned in usage; should omit duration from metadata when absent in response; should omit progress from metadata when absent in response; full port in src/xai/tests/
#[tokio::test]
async fn do_status_metadata_includes_optional_fields_only_when_present() {
    let mut with_usage = done_status();
    with_usage["usage"] = json!({"cost_in_usd_ticks": 4_000_000_000_u64});
    let mut no_duration = done_status();
    no_duration["video"]
        .as_object_mut()
        .unwrap()
        .remove("duration");
    let mut no_progress = done_status();
    no_progress.as_object_mut().unwrap().remove("progress");

    let meta = xai_meta(&completed(
        poll_once(json_response(with_usage), "req-123").await.0,
    ));
    assert_eq!(meta.len(), 5);
    assert_eq!(meta["costInUsdTicks"].as_f64(), Some(4_000_000_000.0));
    let meta = xai_meta(&completed(
        poll_once(json_response(no_duration), "req-123").await.0,
    ));
    assert!(!meta.contains_key("duration"));
    let meta = xai_meta(&completed(
        poll_once(json_response(no_progress), "req-123").await.0,
    ));
    assert!(!meta.contains_key("progress"));
}
