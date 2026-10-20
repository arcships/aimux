//! Ports of `src/xai-video-model.test.ts` from `@ai-sdk/xai@5.0.12`: the
//! `doStart` and `doStatus` cases that `tests/xai_video_test.rs` does not cover.
//!
//! Harness differences from the TypeScript (expectations are unchanged):
//! - `createTestServer` is the in-crate [`MockFetch`]: it answers requests in
//!   order and records each one. `server.calls[0]` is the first recorded
//!   request, `requestBodyJson` is [`Seen::json_body`].
//! - `toStrictEqual` / `toEqual` compare JSON with [`assert_strict_eq`], and
//!   `toMatchObject` is [`assert_matches`]. Both compare every number as an
//!   `f64`, because JavaScript has one number type (`5` and `5.0` are equal).
//! - `_internal.currentDate` cannot be injected: `XaiVideoModel` reads the clock
//!   itself. The two response-metadata cases check that the timestamp lies
//!   between two clock reads, instead of `2024-01-01T00:00:00Z`.
//! - A terminal status (`status: 'error'` upstream) is `Err(AiMuxError::ApiCall)`
//!   here, non-retryable. Its message is compared. The error arm carries no
//!   `VideoResponse`, so the upstream `result.response` check of the expired
//!   case is not ported.
//! - Upstream's `if (result.status === 'completed')` guards in the metadata
//!   cases are asserted directly, so a result that is not completed fails.
//! - `createModel`'s default `api-key` header is not used: no case here checks a
//!   header, and the Rust provider sends `Authorization: Bearer`.

use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::Timestamp;
use aimux_core::error::AiMuxError;
use aimux_core::shared::{AspectRatio, Size, Warning, provider_namespace};
use aimux_core::video_model::{
    VideoCallOptions, VideoFile, VideoFileData, VideoModel, VideoOperationStart,
    VideoOperationStatus, VideoResult,
};

use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};
use crate::xai::options::NAMESPACE;
use crate::xai::{XAIProviderSettings, XaiVideoModel, create_xai};

// ---------------------------------------------------------------- helpers

const BASE_URL: &str = "https://api.example.com";
const PROMPT: &str = "A chicken flying into the sunset";
const SRC: &str = "https://example.com/source-video.mp4";
const VIDEO_URL: &str = "https://vidgen.x.ai/output/video-001.mp4";
const V1: &str = "grok-imagine-video";
const V15: &str = "grok-imagine-video-1.5";

/// `createVideoResponse`.
fn create_video_response() -> Value {
    json!({ "request_id": "req-123" })
}

/// `doneStatusResponse`.
fn done_status_response() -> Value {
    json!({
        "status": "done",
        "video": { "url": VIDEO_URL, "duration": 5, "respect_moderation": true },
        "model": V1,
        "progress": 100
    })
}

/// `createModel({ modelId })`, on the mock transport.
fn model(fetch: &Arc<MockFetch>, model_id: &str) -> XaiVideoModel {
    create_xai(XAIProviderSettings {
        api_key: Some("test-key".into()),
        base_url: Some(BASE_URL.into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .expect("xai provider should build")
    .video(model_id)
}

/// `defaultOptions`: the prompt and `n: 1`, nothing else.
fn default_options() -> VideoCallOptions {
    VideoCallOptions::new(PROMPT)
}

/// `providerOptions: { xai: value }`.
fn with_xai(mut options: VideoCallOptions, value: Value) -> VideoCallOptions {
    options.provider_options =
        provider_namespace(NAMESPACE, value).expect("xai options are an object");
    options
}

/// `doStatus` takes no prompt upstream.
fn status_options() -> VideoCallOptions {
    VideoCallOptions {
        prompt: None,
        ..VideoCallOptions::new(PROMPT)
    }
}

fn url_file(url: &str, media_type: Option<&str>) -> VideoFile {
    VideoFile::Url {
        url: url.into(),
        media_type: media_type.map(str::to_owned),
    }
}

fn ratio(s: &str) -> Option<AspectRatio> {
    Some(AspectRatio::parse(s).unwrap())
}

fn size(s: &str) -> Option<Size> {
    Some(Size::parse(s).unwrap())
}

/// `doStart` against a mock that answers the generation request with `answer`.
/// Returns the result and every request the mock saw.
async fn do_start_answered(
    model_id: &str,
    options: &VideoCallOptions,
    answer: Canned,
) -> (Result<VideoOperationStart, AiMuxError>, Vec<Seen>) {
    let fetch = MockFetch::new(vec![answer]);
    let result = model(&fetch, model_id).do_start(options).await;
    (result, fetch.seen())
}

/// `doStart` against a server that returns `request_id: "req-123"`, with the
/// one request it sent.
async fn start(model_id: &str, options: &VideoCallOptions) -> (VideoOperationStart, Seen) {
    let (result, mut seen) =
        do_start_answered(model_id, options, Canned::json(&create_video_response())).await;
    (result.expect("doStart should succeed"), seen.remove(0))
}

/// `doStatus` against a mock that answers with `answer`. Returns the result and
/// every request the mock saw.
async fn do_status_answered(
    request_id: &str,
    answer: Canned,
) -> (Result<VideoOperationStatus, AiMuxError>, Vec<Seen>) {
    let fetch = MockFetch::new(vec![answer]);
    let result = model(&fetch, V1)
        .do_status(&json!({ "requestId": request_id }), &status_options())
        .await;
    (result, fetch.seen())
}

/// The `completed` arm of a status.
fn completed(result: Result<VideoOperationStatus, AiMuxError>) -> VideoResult {
    match result {
        Ok(VideoOperationStatus::Completed(done)) => done,
        other => panic!("expected a completed status, got {other:?}"),
    }
}

/// The message of an error status. Upstream `status: 'error'` is `Err` here.
fn error_message(result: Result<VideoOperationStatus, AiMuxError>) -> String {
    match result {
        Err(AiMuxError::ApiCall(error)) => error.message,
        other => panic!("expected an error status, got {other:?}"),
    }
}

/// `providerMetadata`, as JSON.
fn metadata_json(done: &VideoResult) -> Value {
    serde_json::to_value(&done.provider_metadata).expect("metadata serializes")
}

/// Every number as an `f64`: JavaScript has one number type.
fn js(value: &Value) -> Value {
    match value {
        Value::Number(n) => n.as_f64().map_or(Value::Null, |f| json!(f)),
        Value::Array(items) => Value::Array(items.iter().map(js).collect()),
        Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), js(v))).collect()),
        other => other.clone(),
    }
}

/// `toStrictEqual` / `toEqual` on JSON values.
fn assert_strict_eq(actual: &Value, expected: &Value) {
    assert_eq!(js(actual), js(expected), "actual: {actual}");
}

/// `toMatchObject`: every key of `expected` is in `actual` and matches. Arrays
/// have equal length and match element by element.
fn assert_matches(actual: &Value, expected: &Value) {
    match (actual, expected) {
        (Value::Object(actual_map), Value::Object(expected_map)) => {
            for (key, want) in expected_map {
                let got = actual_map
                    .get(key)
                    .unwrap_or_else(|| panic!("missing key `{key}` in {actual}"));
                assert_matches(got, want);
            }
        }
        (Value::Array(actual_items), Value::Array(expected_items)) => {
            assert_eq!(actual_items.len(), expected_items.len(), "in {actual}");
            for (got, want) in actual_items.iter().zip(expected_items) {
                assert_matches(got, want);
            }
        }
        _ => assert_eq!(js(actual), js(expected)),
    }
}

/// The `details` of each `unsupported` warning on `feature`, `""` when absent.
fn unsupported_details<'a>(warnings: &'a [Warning], feature: &str) -> Vec<&'a str> {
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

/// `toContainEqual(objectContaining({ type: 'unsupported', feature }))`.
fn has_unsupported(warnings: &[Warning], feature: &str) -> bool {
    !unsupported_details(warnings, feature).is_empty()
}

/// `toContainEqual(objectContaining({ type: 'unsupported', feature, details:
/// stringContaining(needle) }))`.
fn has_unsupported_containing(warnings: &[Warning], feature: &str, needle: &str) -> bool {
    unsupported_details(warnings, feature)
        .iter()
        .any(|details| details.contains(needle))
}

/// `not.toContainEqual(objectContaining({ feature }))`: no warning of any type
/// names `feature`.
fn names_feature(warnings: &[Warning], feature: &str) -> bool {
    warnings.iter().any(|w| {
        matches!(
            w,
            Warning::Unsupported { feature: f, .. } | Warning::Compatibility { feature: f, .. }
                if f == feature
        )
    })
}

mod xai_video_model {
    use super::*;

    mod constructor {
        use super::*;

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › constructor › should expose correct provider and model information
        #[tokio::test]
        async fn should_expose_correct_provider_and_model_information() {
            let fetch = MockFetch::new(vec![]);
            let video = model(&fetch, V1);
            assert_eq!(video.provider(), "xai.video");
            assert_eq!(video.model_id(), V1);
            assert_eq!(video.specification_version(), "v4");
            assert_eq!(video.max_videos_per_call(), Some(1));
        }
    }

    mod do_start {
        use super::*;

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should return operation with requestId
        #[tokio::test]
        async fn should_return_operation_with_request_id() {
            let (result, _) = start(V1, &default_options()).await;
            assert_strict_eq(&result.operation, &json!({ "requestId": "req-123" }));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should pass correct request body
        #[tokio::test]
        async fn should_pass_correct_request_body() {
            let (_, request) = start(V1, &default_options()).await;
            assert_eq!(request.method, "POST");
            assert_eq!(request.url, format!("{BASE_URL}/videos/generations"));
            assert_strict_eq(
                &request.json_body(),
                &json!({ "model": V1, "prompt": PROMPT }),
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should use edits endpoint for video editing (legacy videoUrl without mode)
        #[tokio::test]
        async fn should_use_edits_endpoint_for_video_editing_legacy_video_url_without_mode() {
            let options = with_xai(default_options(), json!({ "videoUrl": SRC }));
            let (_, request) = start(V1, &options).await;
            assert_eq!(request.method, "POST");
            assert_eq!(request.url, format!("{BASE_URL}/videos/edits"));
            assert_matches(&request.json_body(), &json!({ "video": { "url": SRC } }));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should use edits endpoint for video editing with explicit mode
        #[tokio::test]
        async fn should_use_edits_endpoint_for_video_editing_with_explicit_mode() {
            let options = with_xai(
                default_options(),
                json!({ "mode": "edit-video", "videoUrl": SRC }),
            );
            let (_, request) = start(V1, &options).await;
            assert_eq!(request.method, "POST");
            assert_eq!(request.url, format!("{BASE_URL}/videos/edits"));
            assert_matches(&request.json_body(), &json!({ "video": { "url": SRC } }));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should use extensions endpoint for extend-video mode
        #[tokio::test]
        async fn should_use_extensions_endpoint_for_extend_video_mode() {
            let options = with_xai(
                default_options(),
                json!({ "mode": "extend-video", "videoUrl": SRC }),
            );
            let (_, request) = start(V1, &options).await;
            assert_eq!(request.method, "POST");
            assert_eq!(request.url, format!("{BASE_URL}/videos/extensions"));
            assert_matches(&request.json_body(), &json!({ "video": { "url": SRC } }));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should include response metadata
        #[tokio::test]
        async fn should_include_response_metadata() {
            // Upstream injects `currentDate` and expects `timestamp: testDate`.
            // The clock is read inside the model here, so the timestamp is
            // checked to lie between two reads.
            let before = Timestamp::now();
            let (result, _) = start(V1, &default_options()).await;
            let after = Timestamp::now();
            let timestamp = result.response.timestamp.expect("timestamp is set");
            assert!(before <= timestamp && timestamp <= after);
            assert_eq!(result.response.model_id.as_deref(), Some(V1));
            assert!(result.response.headers.is_some());
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should send duration in request body
        #[tokio::test]
        async fn should_send_duration_in_request_body() {
            let mut options = default_options();
            options.duration = Some(10);
            let (_, request) = start(V1, &options).await;
            assert_matches(&request.json_body(), &json!({ "duration": 10 }));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should send aspect_ratio in request body
        #[tokio::test]
        async fn should_send_aspect_ratio_in_request_body() {
            let mut options = default_options();
            options.aspect_ratio = ratio("9:16");
            let (_, request) = start(V1, &options).await;
            assert_matches(&request.json_body(), &json!({ "aspect_ratio": "9:16" }));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should map SDK resolution 1280x720 to 720p
        #[tokio::test]
        async fn should_map_sdk_resolution_1280x720_to_720p() {
            let mut options = default_options();
            options.resolution = size("1280x720");
            let (_, request) = start(V1, &options).await;
            assert_matches(&request.json_body(), &json!({ "resolution": "720p" }));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should map SDK resolution 854x480 to 480p
        #[tokio::test]
        async fn should_map_sdk_resolution_854x480_to_480p() {
            let mut options = default_options();
            options.resolution = size("854x480");
            let (_, request) = start(V1, &options).await;
            assert_matches(&request.json_body(), &json!({ "resolution": "480p" }));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should map SDK resolution 640x480 to 480p
        #[tokio::test]
        async fn should_map_sdk_resolution_640x480_to_480p() {
            let mut options = default_options();
            options.resolution = size("640x480");
            let (_, request) = start(V1, &options).await;
            assert_matches(&request.json_body(), &json!({ "resolution": "480p" }));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should map SDK resolution 1920x1080 to 1080p
        #[tokio::test]
        async fn should_map_sdk_resolution_1920x1080_to_1080p() {
            let mut options = default_options();
            options.resolution = size("1920x1080");
            let (_, request) = start(V15, &options).await;
            assert_matches(&request.json_body(), &json!({ "resolution": "1080p" }));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should pass through provider option resolution 1080p
        #[tokio::test]
        async fn should_pass_through_provider_option_resolution_1080p() {
            let options = with_xai(default_options(), json!({ "resolution": "1080p" }));
            let (_, request) = start(V15, &options).await;
            assert_matches(&request.json_body(), &json!({ "resolution": "1080p" }));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should warn when SDK resolution 1920x1080 is used with grok-imagine-video
        #[tokio::test]
        async fn should_warn_when_sdk_resolution_1920x1080_is_used_with_grok_imagine_video() {
            let mut options = default_options();
            options.resolution = size("1920x1080");
            let (result, request) = start(V1, &options).await;
            assert_matches(&request.json_body(), &json!({ "resolution": "1080p" }));
            assert!(
                has_unsupported_containing(
                    &result.warnings,
                    "resolution",
                    "does not support 1080p"
                ),
                "{:?}",
                result.warnings
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should warn when provider resolution 1080p is used with grok-imagine-video
        #[tokio::test]
        async fn should_warn_when_provider_resolution_1080p_is_used_with_grok_imagine_video() {
            let options = with_xai(default_options(), json!({ "resolution": "1080p" }));
            let (result, request) = start(V1, &options).await;
            assert_matches(&request.json_body(), &json!({ "resolution": "1080p" }));
            assert!(
                has_unsupported_containing(
                    &result.warnings,
                    "resolution",
                    "does not support 1080p"
                ),
                "{:?}",
                result.warnings
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should not warn about 1080p with grok-imagine-video-1.5
        #[tokio::test]
        async fn should_not_warn_about_1080p_with_grok_imagine_video_1_5() {
            let mut options = default_options();
            options.resolution = size("1920x1080");
            let (result, _) = start(V15, &options).await;
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should prefer provider option resolution over SDK resolution
        #[tokio::test]
        async fn should_prefer_provider_option_resolution_over_sdk_resolution() {
            let mut options = with_xai(default_options(), json!({ "resolution": "480p" }));
            options.resolution = size("1280x720");
            let (_, request) = start(V1, &options).await;
            assert_matches(&request.json_body(), &json!({ "resolution": "480p" }));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should warn for unrecognized resolution format
        #[tokio::test]
        async fn should_warn_for_unrecognized_resolution_format() {
            let mut options = default_options();
            options.resolution = size("2560x1440");
            let (result, _) = start(V1, &options).await;
            assert!(
                has_unsupported(&result.warnings, "resolution"),
                "{:?}",
                result.warnings
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should warn and omit body resolution for completely unknown format
        #[tokio::test]
        async fn should_warn_and_omit_body_resolution_for_completely_unknown_format() {
            let mut options = default_options();
            options.resolution = size("3840x2160");
            let (result, request) = start(V1, &options).await;
            assert!(request.json_body().get("resolution").is_none());
            assert!(
                has_unsupported(&result.warnings, "resolution"),
                "{:?}",
                result.warnings
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should send image object from URL-based image input
        #[tokio::test]
        async fn should_send_image_object_from_url_based_image_input() {
            let mut options = default_options();
            options.image = Some(url_file("https://example.com/image.png", None));
            let (_, request) = start(V1, &options).await;
            assert_matches(
                &request.json_body(),
                &json!({ "image": { "url": "https://example.com/image.png" } }),
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should send image object with data URI from file data
        #[tokio::test]
        async fn should_send_image_object_with_data_uri_from_file_data() {
            let mut options = default_options();
            options.image = Some(VideoFile::File {
                media_type: "image/png".into(),
                // PNG magic bytes.
                data: VideoFileData::Binary(vec![137, 80, 78, 71]),
            });
            let (_, request) = start(V1, &options).await;
            assert_matches(
                &request.json_body(),
                &json!({ "image": { "url": "data:image/png;base64,iVBORw==" } }),
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should send image object with data URI from base64 string
        #[tokio::test]
        async fn should_send_image_object_with_data_uri_from_base64_string() {
            let mut options = default_options();
            options.image = Some(VideoFile::File {
                media_type: "image/jpeg".into(),
                data: VideoFileData::Base64("aGVsbG8=".into()),
            });
            let (_, request) = start(V1, &options).await;
            assert_matches(
                &request.json_body(),
                &json!({ "image": { "url": "data:image/jpeg;base64,aGVsbG8=" } }),
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should warn about duration in edit mode
        #[tokio::test]
        async fn should_warn_about_duration_in_edit_mode() {
            let mut options = with_xai(
                default_options(),
                json!({ "mode": "edit-video", "videoUrl": SRC }),
            );
            options.duration = Some(10);
            let (result, _) = start(V1, &options).await;
            assert!(
                has_unsupported(&result.warnings, "duration"),
                "{:?}",
                result.warnings
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should warn about aspectRatio in edit mode
        #[tokio::test]
        async fn should_warn_about_aspect_ratio_in_edit_mode() {
            let mut options = with_xai(
                default_options(),
                json!({ "mode": "edit-video", "videoUrl": SRC }),
            );
            options.aspect_ratio = ratio("16:9");
            let (result, _) = start(V1, &options).await;
            assert!(
                has_unsupported(&result.warnings, "aspectRatio"),
                "{:?}",
                result.warnings
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should warn about resolution in edit mode
        #[tokio::test]
        async fn should_warn_about_resolution_in_edit_mode() {
            let mut options = with_xai(
                default_options(),
                json!({ "mode": "edit-video", "videoUrl": SRC }),
            );
            options.resolution = size("1280x720");
            let (result, _) = start(V1, &options).await;
            assert!(
                has_unsupported(&result.warnings, "resolution"),
                "{:?}",
                result.warnings
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should not warn about duration outside edit mode
        #[tokio::test]
        async fn should_not_warn_about_duration_outside_edit_mode() {
            let mut options = default_options();
            options.duration = Some(10);
            let (result, _) = start(V1, &options).await;
            assert!(!names_feature(&result.warnings, "duration"));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should not warn about aspectRatio outside edit mode
        #[tokio::test]
        async fn should_not_warn_about_aspect_ratio_outside_edit_mode() {
            let mut options = default_options();
            options.aspect_ratio = ratio("16:9");
            let (result, _) = start(V1, &options).await;
            assert!(!names_feature(&result.warnings, "aspectRatio"));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should not warn about resolution outside edit mode
        #[tokio::test]
        async fn should_not_warn_about_resolution_outside_edit_mode() {
            let mut options = default_options();
            options.resolution = size("1280x720");
            let (result, _) = start(V1, &options).await;
            assert!(!names_feature(&result.warnings, "resolution"));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should omit duration, aspect_ratio, and resolution from body in edit mode
        #[tokio::test]
        async fn should_omit_duration_aspect_ratio_and_resolution_from_body_in_edit_mode() {
            let mut options = with_xai(
                default_options(),
                json!({ "mode": "edit-video", "videoUrl": SRC }),
            );
            options.duration = Some(10);
            options.aspect_ratio = ratio("16:9");
            options.resolution = size("1280x720");
            let (_, request) = start(V1, &options).await;
            let body = request.json_body();
            assert!(body.get("duration").is_none());
            assert!(body.get("aspect_ratio").is_none());
            assert!(body.get("resolution").is_none());
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should return empty warnings for supported features
        #[tokio::test]
        async fn should_return_empty_warnings_for_supported_features() {
            let (result, _) = start(V1, &default_options()).await;
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should warn when n > 1
        #[tokio::test]
        async fn should_warn_when_n_greater_than_1() {
            let mut options = default_options();
            options.n = 3;
            let (result, _) = start(V1, &options).await;
            assert!(
                has_unsupported(&result.warnings, "n"),
                "{:?}",
                result.warnings
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should not warn when n is 1
        #[tokio::test]
        async fn should_not_warn_when_n_is_1() {
            let mut options = default_options();
            options.n = 1;
            let (result, _) = start(V1, &options).await;
            assert!(!names_feature(&result.warnings, "n"));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should allow duration in extension mode
        #[tokio::test]
        async fn should_allow_duration_in_extension_mode() {
            let mut options = with_xai(
                default_options(),
                json!({ "mode": "extend-video", "videoUrl": SRC }),
            );
            options.duration = Some(6);
            let (_, request) = start(V1, &options).await;
            assert_matches(&request.json_body(), &json!({ "duration": 6 }));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should warn about aspectRatio in extension mode
        #[tokio::test]
        async fn should_warn_about_aspect_ratio_in_extension_mode() {
            let mut options = with_xai(
                default_options(),
                json!({ "mode": "extend-video", "videoUrl": SRC }),
            );
            options.aspect_ratio = ratio("16:9");
            let (result, _) = start(V1, &options).await;
            assert!(
                has_unsupported(&result.warnings, "aspectRatio"),
                "{:?}",
                result.warnings
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should warn about resolution in extension mode
        #[tokio::test]
        async fn should_warn_about_resolution_in_extension_mode() {
            let mut options = with_xai(
                default_options(),
                json!({ "mode": "extend-video", "videoUrl": SRC }),
            );
            options.resolution = size("1280x720");
            let (result, _) = start(V1, &options).await;
            assert!(
                has_unsupported(&result.warnings, "resolution"),
                "{:?}",
                result.warnings
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should omit aspect_ratio and resolution from body in extension mode
        #[tokio::test]
        async fn should_omit_aspect_ratio_and_resolution_from_body_in_extension_mode() {
            let mut options = with_xai(
                default_options(),
                json!({ "mode": "extend-video", "videoUrl": SRC }),
            );
            options.aspect_ratio = ratio("16:9");
            options.resolution = size("1280x720");
            let (_, request) = start(V1, &options).await;
            let body = request.json_body();
            assert!(body.get("aspect_ratio").is_none());
            assert!(body.get("resolution").is_none());
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should not warn about duration in extension mode
        #[tokio::test]
        async fn should_not_warn_about_duration_in_extension_mode() {
            let mut options = with_xai(
                default_options(),
                json!({ "mode": "extend-video", "videoUrl": SRC }),
            );
            options.duration = Some(6);
            let (result, _) = start(V1, &options).await;
            assert!(!names_feature(&result.warnings, "duration"));
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should warn about provider-level resolution in extension mode
        #[tokio::test]
        async fn should_warn_about_provider_level_resolution_in_extension_mode() {
            let options = with_xai(
                default_options(),
                json!({ "mode": "extend-video", "videoUrl": SRC, "resolution": "720p" }),
            );
            let (result, request) = start(V1, &options).await;
            assert!(request.json_body().get("resolution").is_none());
            assert!(
                has_unsupported(&result.warnings, "resolution"),
                "{:?}",
                result.warnings
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should send reference_images array to /videos/generations with explicit mode
        #[tokio::test]
        async fn should_send_reference_images_array_to_videos_generations_with_explicit_mode() {
            let options = with_xai(
                default_options(),
                json!({
                    "mode": "reference-to-video",
                    "referenceImageUrls": [
                        "https://example.com/ref1.jpg",
                        "https://example.com/ref2.jpg"
                    ]
                }),
            );
            let (_, request) = start(V1, &options).await;
            assert_eq!(request.method, "POST");
            assert_eq!(request.url, format!("{BASE_URL}/videos/generations"));
            assert_matches(
                &request.json_body(),
                &json!({
                    "reference_images": [
                        { "url": "https://example.com/ref1.jpg" },
                        { "url": "https://example.com/ref2.jpg" }
                    ]
                }),
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should downgrade R2V 1080p to 720p with a warning
        #[tokio::test]
        async fn should_downgrade_r2v_1080p_to_720p_with_a_warning() {
            let options = with_xai(
                default_options(),
                json!({
                    "mode": "reference-to-video",
                    "referenceImageUrls": ["https://example.com/ref1.jpg"],
                    "resolution": "1080p"
                }),
            );
            let (result, request) = start(V15, &options).await;
            assert_matches(&request.json_body(), &json!({ "resolution": "720p" }));
            assert!(
                has_unsupported(&result.warnings, "resolution"),
                "{:?}",
                result.warnings
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should downgrade R2V 1920x1080 from the SDK resolution to 720p
        #[tokio::test]
        async fn should_downgrade_r2v_1920x1080_from_the_sdk_resolution_to_720p() {
            let mut options = with_xai(
                default_options(),
                json!({
                    "mode": "reference-to-video",
                    "referenceImageUrls": ["https://example.com/ref1.jpg"]
                }),
            );
            options.resolution = size("1920x1080");
            let (result, request) = start(V15, &options).await;
            assert_matches(&request.json_body(), &json!({ "resolution": "720p" }));
            assert!(
                has_unsupported(&result.warnings, "resolution"),
                "{:?}",
                result.warnings
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should separate image and audio inputReferences
        #[tokio::test]
        async fn should_separate_image_and_audio_input_references() {
            let mut options = default_options();
            options.input_references = Some(vec![
                url_file("https://example.com/ref1.jpg", None),
                url_file("https://example.com/voice.mp3", Some("audio/mpeg")),
            ]);
            let (result, request) = start(V15, &options).await;
            let body = request.json_body();
            assert_matches(
                &body,
                &json!({
                    "reference_images": [{ "url": "https://example.com/ref1.jpg" }],
                    "reference_audios": [{ "url": "https://example.com/voice.mp3" }]
                }),
            );
            assert_eq!(body["reference_images"].as_array().map(Vec::len), Some(1));
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should support audio-only reference-to-video
        #[tokio::test]
        async fn should_support_audio_only_reference_to_video() {
            let mut options = default_options();
            options.input_references = Some(vec![url_file(
                "https://example.com/voice.mp3",
                Some("audio/mpeg"),
            )]);
            let (result, request) = start(V15, &options).await;
            let body = request.json_body();
            assert_matches(
                &body,
                &json!({ "reference_audios": [{ "url": "https://example.com/voice.mp3" }] }),
            );
            assert!(body.get("reference_images").is_none());
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should not send an empty reference_images array for video-only inputReferences
        #[tokio::test]
        async fn should_not_send_an_empty_reference_images_array_for_video_only_input_references() {
            let mut options = default_options();
            options.input_references = Some(vec![url_file(
                "https://example.com/clip.mp4",
                Some("video/mp4"),
            )]);
            let (_, request) = start(V15, &options).await;
            assert!(request.json_body().get("reference_images").is_none());
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should combine a pinned first frame with an audio reference
        #[tokio::test]
        async fn should_combine_a_pinned_first_frame_with_an_audio_reference() {
            let mut options = default_options();
            options.image = Some(url_file(
                "https://example.com/start.jpg",
                Some("image/jpeg"),
            ));
            options.input_references = Some(vec![url_file(
                "https://example.com/voice.mp3",
                Some("audio/mpeg"),
            )]);
            let (result, request) = start(V15, &options).await;
            let body = request.json_body();
            assert_matches(
                &body,
                &json!({
                    "image": { "url": "https://example.com/start.jpg" },
                    "reference_audios": [{ "url": "https://example.com/voice.mp3" }]
                }),
            );
            assert!(body.get("reference_images").is_none());
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should support explicit audio-only R2V without reference_images
        #[tokio::test]
        async fn should_support_explicit_audio_only_r2v_without_reference_images() {
            let mut options = with_xai(default_options(), json!({ "mode": "reference-to-video" }));
            options.input_references = Some(vec![url_file(
                "https://example.com/voice.mp3",
                Some("audio/mpeg"),
            )]);
            let (result, request) = start(V15, &options).await;
            let body = request.json_body();
            assert!(body.get("reference_images").is_none());
            assert_matches(
                &body,
                &json!({ "reference_audios": [{ "url": "https://example.com/voice.mp3" }] }),
            );
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should fallback to reference-to-video mode when referenceImageUrls is set without mode
        #[tokio::test]
        async fn should_fallback_to_r2v_when_reference_image_urls_set_without_mode() {
            let options = with_xai(
                default_options(),
                json!({ "referenceImageUrls": ["https://example.com/ref1.jpg"] }),
            );
            let (_, request) = start(V1, &options).await;
            assert_eq!(request.url, format!("{BASE_URL}/videos/generations"));
            assert_matches(
                &request.json_body(),
                &json!({ "reference_images": [{ "url": "https://example.com/ref1.jpg" }] }),
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should send reference_audios for preset reference voices
        #[tokio::test]
        async fn should_send_reference_audios_for_preset_reference_voices() {
            let options = with_xai(
                default_options(),
                json!({
                    "mode": "reference-to-video",
                    "referenceImageUrls": ["https://example.com/ref1.jpg"],
                    "referenceVoiceIds": ["eve"]
                }),
            );
            let (_, request) = start(V15, &options).await;
            assert_eq!(request.url, format!("{BASE_URL}/videos/generations"));
            assert_matches(
                &request.json_body(),
                &json!({
                    "reference_images": [{ "url": "https://example.com/ref1.jpg" }],
                    "reference_audios": [{ "voice_id": "eve" }]
                }),
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should send up to 3 preset reference voices in order
        #[tokio::test]
        async fn should_send_up_to_3_preset_reference_voices_in_order() {
            let options = with_xai(
                default_options(),
                json!({
                    "mode": "reference-to-video",
                    "referenceImageUrls": ["https://example.com/ref1.jpg"],
                    "referenceVoiceIds": ["eve", "leo", "rex"]
                }),
            );
            let (_, request) = start(V15, &options).await;
            assert_matches(
                &request.json_body(),
                &json!({
                    "reference_audios": [
                        { "voice_id": "eve" },
                        { "voice_id": "leo" },
                        { "voice_id": "rex" }
                    ]
                }),
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should reject more than 3 preset reference voices
        #[tokio::test]
        async fn should_reject_more_than_3_preset_reference_voices() {
            let options = with_xai(
                default_options(),
                json!({
                    "mode": "reference-to-video",
                    "referenceImageUrls": ["https://example.com/ref1.jpg"],
                    "referenceVoiceIds": ["ara", "eve", "leo", "rex"]
                }),
            );
            let (result, _) =
                do_start_answered(V15, &options, Canned::json(&create_video_response())).await;
            assert!(
                matches!(result, Err(AiMuxError::InvalidArgument(_))),
                "{result:?}"
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should reject empty-string preset reference voice ids
        #[tokio::test]
        async fn should_reject_empty_string_preset_reference_voice_ids() {
            let options = with_xai(
                default_options(),
                json!({
                    "mode": "reference-to-video",
                    "referenceImageUrls": ["https://example.com/ref1.jpg"],
                    "referenceVoiceIds": [""]
                }),
            );
            let (result, _) =
                do_start_answered(V15, &options, Canned::json(&create_video_response())).await;
            assert!(
                matches!(result, Err(AiMuxError::InvalidArgument(_))),
                "{result:?}"
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStart › should never forward referenceVoiceIds as a raw body key
        #[tokio::test]
        async fn should_never_forward_reference_voice_ids_as_a_raw_body_key() {
            let options = with_xai(
                default_options(),
                json!({
                    "mode": "reference-to-video",
                    "referenceImageUrls": ["https://example.com/ref1.jpg"],
                    "referenceVoiceIds": ["eve"]
                }),
            );
            let (_, request) = start(V15, &options).await;
            assert!(request.json_body().get("referenceVoiceIds").is_none());
        }
    }

    mod do_status {
        use super::*;

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStatus › should encode the request ID as a single URL path segment
        #[tokio::test]
        async fn should_encode_the_request_id_as_a_single_url_path_segment() {
            // `encodeURIComponent('abc/../../internal')`.
            let (_, requests) =
                do_status_answered("abc/../../internal", Canned::json(&done_status_response()))
                    .await;
            assert_eq!(
                requests[0].url,
                format!("{BASE_URL}/videos/abc%2F..%2F..%2Finternal")
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStatus › should preserve the $requestId request ID as a URL path segment
        #[tokio::test]
        async fn should_preserve_the_request_id_as_a_url_path_segment() {
            // The dot segments are sent encoded twice, so URL parsing keeps them.
            for (request_id, encoded_request_id) in [(".", "%252E"), ("..", "%252E%252E")] {
                let (_, requests) =
                    do_status_answered(request_id, Canned::json(&done_status_response())).await;
                assert_eq!(
                    requests[0].url,
                    format!("{BASE_URL}/videos/{encoded_request_id}"),
                    "requestId {request_id:?}"
                );
            }
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStatus › should return completed with video data when done
        #[tokio::test]
        async fn should_return_completed_with_video_data_when_done() {
            let (result, _) =
                do_status_answered("req-123", Canned::json(&done_status_response())).await;
            let done = completed(result);
            assert_eq!(done.videos.len(), 1);
            assert_strict_eq(
                &serde_json::to_value(&done.videos[0]).expect("video serializes"),
                &json!({ "type": "url", "url": VIDEO_URL, "mediaType": "video/mp4" }),
            );
            assert_strict_eq(
                &metadata_json(&done),
                &json!({
                    "xai": {
                        "requestId": "req-123",
                        "videoUrl": VIDEO_URL,
                        "duration": 5,
                        "progress": 100
                    }
                }),
            );
        }

        /// partial: `doStatus › should return error status on expired`. The
        /// upstream `result.response` check (`timestamp`, `modelId`,
        /// `headers`) is not expressible: an error status is `Err(AiMuxError)`
        /// here, which carries no `VideoResponse` (sub-feature in
        /// `layout.toml`).
        #[tokio::test]
        async fn should_return_error_status_on_expired() {
            let body = json!({ "status": "expired", "model": V1 });
            let (result, _) = do_status_answered("req-123", Canned::json(&body)).await;
            assert_eq!(error_message(result), "Video generation request expired.");
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStatus › should return error status on failed
        #[tokio::test]
        async fn should_return_error_status_on_failed() {
            let body = json!({
                "status": "failed",
                "model": V1,
                "progress": 0,
                "error": { "message": "Content policy violation" }
            });
            let (result, _) = do_status_answered("req-123", Canned::json(&body)).await;
            assert_eq!(
                error_message(result),
                "Video generation failed: Content policy violation"
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStatus › should report an error status when video URL missing on done
        #[tokio::test]
        async fn should_report_an_error_status_when_video_url_missing_on_done() {
            let body = json!({ "status": "done", "video": null, "model": V1 });
            let (result, _) = do_status_answered("req-123", Canned::json(&body)).await;
            assert_eq!(
                error_message(result),
                "Video generation completed but no video URL was returned."
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStatus › should report an error status when respect_moderation is false
        #[tokio::test]
        async fn should_report_an_error_status_when_respect_moderation_is_false() {
            let body = json!({
                "status": "done",
                "video": { "url": "", "respect_moderation": false },
                "model": V1
            });
            let (result, _) = do_status_answered("req-123", Canned::json(&body)).await;
            assert_eq!(
                error_message(result),
                "Video generation was blocked due to a content policy violation."
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStatus › should include response metadata
        #[tokio::test]
        async fn should_include_response_metadata() {
            // Upstream injects `currentDate` and expects `timestamp: testDate`.
            // The clock is read inside the model here, so the timestamp is
            // checked to lie between two reads.
            let before = Timestamp::now();
            let (result, _) =
                do_status_answered("req-123", Canned::json(&done_status_response())).await;
            let after = Timestamp::now();
            let done = completed(result);
            let timestamp = done.response.timestamp.expect("timestamp is set");
            assert!(before <= timestamp && timestamp <= after);
            assert_eq!(done.response.model_id.as_deref(), Some(V1));
            assert!(done.response.headers.is_some());
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStatus › should include costInUsdTicks when returned in usage
        #[tokio::test]
        async fn should_include_cost_in_usd_ticks_when_returned_in_usage() {
            let mut body = done_status_response();
            body["usage"] = json!({ "cost_in_usd_ticks": 4_000_000_000_u64 });
            let (result, _) = do_status_answered("req-123", Canned::json(&body)).await;
            let done = completed(result);
            assert_strict_eq(
                &metadata_json(&done),
                &json!({
                    "xai": {
                        "requestId": "req-123",
                        "videoUrl": VIDEO_URL,
                        "duration": 5,
                        "progress": 100,
                        "costInUsdTicks": 4_000_000_000_u64
                    }
                }),
            );
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStatus › should omit duration from metadata when absent in response
        #[tokio::test]
        async fn should_omit_duration_from_metadata_when_absent_in_response() {
            let body = json!({
                "status": "done",
                "video": { "url": VIDEO_URL, "respect_moderation": true },
                "model": V1
            });
            let (result, _) = do_status_answered("req-123", Canned::json(&body)).await;
            let done = completed(result);
            assert!(metadata_json(&done)["xai"].get("duration").is_none());
        }

        /// upstream: @ai-sdk/xai src/xai-video-model.test.ts › XaiVideoModel › doStatus › should omit progress from metadata when absent in response
        #[tokio::test]
        async fn should_omit_progress_from_metadata_when_absent_in_response() {
            let body = json!({
                "status": "done",
                "video": { "url": VIDEO_URL, "duration": 5, "respect_moderation": true },
                "model": V1
            });
            let (result, _) = do_status_answered("req-123", Canned::json(&body)).await;
            let done = completed(result);
            assert!(metadata_json(&done)["xai"].get("progress").is_none());
        }
    }
}
