use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{AspectRatio, Size, Warning};
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoFileData, VideoModel, VideoOperationStatus,
};

use crate::google::{GoogleProviderSettings, GoogleVideoModel, create_google};
use crate::test_support::mock_fetch::{Canned, RouteFetch, Seen};

const PROMPT: &str = "A futuristic city with flying cars";

/// `defaultOptions`: the prompt, `n: 1`, everything else unset.
fn default_options() -> VideoCallOptions {
    VideoCallOptions::new(PROMPT)
}

/// The options of `createMockModel`.
struct MockModel {
    model_id: &'static str,
    operation_name: &'static str,
    operation_done: bool,
    operation_error: Option<Value>,
    videos: Value,
    api_key: &'static str,
}

impl Default for MockModel {
    fn default() -> Self {
        Self {
            model_id: "veo-3.1-generate-preview",
            operation_name: "operations/test-operation-id",
            operation_done: true,
            operation_error: None,
            videos: json!([{
                "video": {"uri": "https://generativelanguage.googleapis.com/files/video-123.mp4"},
            }]),
            api_key: "test-api-key",
        }
    }
}

/// `createMockModel`: `:predictLongRunning` answers the operation name, a
/// URL containing the operation name answers its status, anything else 404.
fn create_mock_model(options: MockModel) -> (GoogleVideoModel, Arc<RouteFetch>) {
    let MockModel {
        model_id,
        operation_name,
        operation_done,
        operation_error,
        videos,
        api_key,
    } = options;
    let fetch = RouteFetch::new(move |seen: &Seen| {
        if seen.url.contains(":predictLongRunning") {
            return Canned::json(&json!({"name": operation_name, "done": false}));
        }
        if seen.url.contains(operation_name) {
            let mut body = json!({"name": operation_name, "done": operation_done});
            match &operation_error {
                Some(error) => body["error"] = error.clone(),
                None => {
                    body["response"] =
                        json!({"generateVideoResponse": {"generatedSamples": videos}});
                }
            }
            return Canned::json(&body);
        }
        Canned {
            status: 404,
            headers: Vec::new(),
            body: b"Not found".to_vec(),
        }
    });
    (model_with_fetch(model_id, api_key, &fetch), fetch)
}

/// A model whose every request gets `body`.
fn model_answering(api_key: &'static str, body: Value) -> (GoogleVideoModel, Arc<RouteFetch>) {
    let fetch = RouteFetch::new(move |_: &Seen| Canned::json(&body));
    (
        model_with_fetch("veo-3.1-generate-preview", api_key, &fetch),
        fetch,
    )
}

fn model_with_fetch(model_id: &str, api_key: &str, fetch: &Arc<RouteFetch>) -> GoogleVideoModel {
    create_google(GoogleProviderSettings {
        api_key: Some(api_key.into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
    .video(model_id)
}

/// The body of the `:predictLongRunning` request (`capturedBody`).
fn start_body(fetch: &RouteFetch) -> Value {
    fetch
        .seen()
        .iter()
        .find(|seen| seen.url.contains(":predictLongRunning"))
        .expect("a start request")
        .json_body()
}

fn google_options(value: Value) -> aimux_core::shared::SharedProviderOptions {
    let Value::Object(map) = value else {
        panic!("options must be an object")
    };
    [("google".to_string(), map)].into_iter().collect()
}

fn wire_message(error: &AiMuxError) -> String {
    serde_json::to_value(error).unwrap()["message"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

mod google_video_model {
    use super::*;

    mod constructor {
        use super::*;

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › constructor › should expose correct provider and model information
        #[test]
        fn should_expose_correct_provider_and_model_information() {
            let (model, _) = create_mock_model(MockModel::default());

            assert_eq!(model.provider(), "google.generative-ai");
            assert_eq!(model.model_id(), "veo-3.1-generate-preview");
            assert_eq!(model.specification_version(), "v4");
            assert_eq!(model.max_videos_per_call(), Some(4));
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › constructor › should support different model IDs
        #[test]
        fn should_support_different_model_ids() {
            let (model, _) = create_mock_model(MockModel {
                model_id: "veo-3.1-generate",
                ..Default::default()
            });

            assert_eq!(model.model_id(), "veo-3.1-generate");
        }
    }

    mod do_start {
        use super::*;

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should return operation with operationName
        #[tokio::test]
        async fn should_return_operation_with_operation_name() {
            let (model, _) = create_mock_model(MockModel {
                operation_name: "operations/start-test-op",
                ..Default::default()
            });

            let result = model.do_start(&default_options()).await.unwrap();

            assert_eq!(
                result.operation,
                json!({"operationName": "operations/start-test-op"})
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should pass correct request body
        #[tokio::test]
        async fn should_pass_correct_request_body() {
            let (model, fetch) = create_mock_model(MockModel::default());

            model.do_start(&default_options()).await.unwrap();

            assert_eq!(
                start_body(&fetch),
                json!({"instances": [{"prompt": PROMPT}], "parameters": {"sampleCount": 1}})
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should pass seed when provided
        #[tokio::test]
        async fn should_pass_seed_when_provided() {
            let (model, fetch) = create_mock_model(MockModel::default());

            let mut options = default_options();
            options.seed = Some(42);
            model.do_start(&options).await.unwrap();

            assert_eq!(
                start_body(&fetch),
                json!({
                    "instances": [{"prompt": PROMPT}],
                    "parameters": {"sampleCount": 1, "seed": 42},
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should pass aspect ratio when provided
        #[tokio::test]
        async fn should_pass_aspect_ratio_when_provided() {
            let (model, fetch) = create_mock_model(MockModel::default());

            let mut options = default_options();
            options.aspect_ratio = Some(AspectRatio::new(16, 9));
            model.do_start(&options).await.unwrap();

            assert_eq!(
                start_body(&fetch),
                json!({
                    "instances": [{"prompt": PROMPT}],
                    "parameters": {"sampleCount": 1, "aspectRatio": "16:9"},
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should convert resolution to Google format
        #[tokio::test]
        async fn should_convert_resolution_to_google_format() {
            let (model, fetch) = create_mock_model(MockModel::default());

            let mut options = default_options();
            options.resolution = Some(Size::new(1920, 1080));
            model.do_start(&options).await.unwrap();

            assert_eq!(
                start_body(&fetch),
                json!({
                    "instances": [{"prompt": PROMPT}],
                    "parameters": {"sampleCount": 1, "resolution": "1080p"},
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should pass duration as durationSeconds
        #[tokio::test]
        async fn should_pass_duration_as_duration_seconds() {
            let (model, fetch) = create_mock_model(MockModel::default());

            let mut options = default_options();
            options.duration = Some(5);
            model.do_start(&options).await.unwrap();

            assert_eq!(
                start_body(&fetch),
                json!({
                    "instances": [{"prompt": PROMPT}],
                    "parameters": {"sampleCount": 1, "durationSeconds": 5},
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should pass n as sampleCount
        #[tokio::test]
        async fn should_pass_n_as_sample_count() {
            let (model, fetch) = create_mock_model(MockModel {
                videos: json!([
                    {"video": {"uri": "https://example.com/video1.mp4"}},
                    {"video": {"uri": "https://example.com/video2.mp4"}},
                ]),
                ..Default::default()
            });

            let mut options = default_options();
            options.n = 2;
            model.do_start(&options).await.unwrap();

            assert_eq!(
                start_body(&fetch),
                json!({"instances": [{"prompt": PROMPT}], "parameters": {"sampleCount": 2}})
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should return warnings array
        #[tokio::test]
        async fn should_return_warnings_array() {
            let (model, _) = create_mock_model(MockModel::default());

            let result = model.do_start(&default_options()).await.unwrap();

            assert!(result.warnings.is_empty());
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should include timestamp and modelId in response
        #[tokio::test]
        async fn should_include_timestamp_and_model_id_in_response() {
            // No `currentDate` hook in Rust: the timestamp is the real time.
            let (model, _) = create_mock_model(MockModel::default());

            let before = aimux_core::Timestamp::now();
            let result = model.do_start(&default_options()).await.unwrap();
            let after = aimux_core::Timestamp::now();

            let timestamp = result.response.timestamp.unwrap();
            assert!(before <= timestamp && timestamp <= after);
            assert_eq!(
                result.response.model_id.as_deref(),
                Some("veo-3.1-generate-preview")
            );
            assert!(result.response.headers.is_some());
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let (model, fetch) = create_mock_model(MockModel::default());

            let mut options = default_options();
            options.headers = Some(
                [("x-custom-header".to_string(), "custom-value".to_string())]
                    .into_iter()
                    .collect(),
            );
            model.do_start(&options).await.unwrap();

            let seen = fetch.seen();
            let start = seen
                .iter()
                .find(|seen| seen.url.contains(":predictLongRunning"))
                .unwrap();
            assert_eq!(
                start.headers.get("x-custom-header").map(String::as_str),
                Some("custom-value")
            );
            assert_eq!(
                start.headers.get("x-goog-api-key").map(String::as_str),
                Some("test-api-key")
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should throw when no operation name returned
        #[tokio::test]
        async fn should_throw_when_no_operation_name_returned() {
            let (model, _) = model_answering("test-key", json!({"done": false}));

            let error = model.do_start(&default_options()).await.unwrap_err();

            assert_eq!(wire_message(&error), "No operation name returned from API");
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should send image as inlineData
        #[tokio::test]
        async fn should_send_image_as_inline_data() {
            let (model, fetch) = create_mock_model(MockModel::default());

            let mut options = default_options();
            options.image = Some(VideoFile::File {
                media_type: "image/png".into(),
                data: VideoFileData::Base64("base64-image-data".into()),
            });
            model.do_start(&options).await.unwrap();

            assert_eq!(
                start_body(&fetch)["instances"][0]["image"],
                json!({"bytesBase64Encoded": "base64-image-data", "mimeType": "image/png"})
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should warn when URL-based image is provided
        #[tokio::test]
        async fn should_warn_when_url_based_image_is_provided() {
            let (model, _) = create_mock_model(MockModel::default());

            let mut options = default_options();
            options.image = Some(VideoFile::Url {
                url: "https://example.com/image.png".into(),
                media_type: None,
            });
            let result = model.do_start(&options).await.unwrap();

            assert_eq!(result.warnings.len(), 1);
            assert!(
                matches!(
                    &result.warnings[0],
                    Warning::Unsupported { feature, .. } if feature == "URL-based image input"
                ),
                "{:?}",
                result.warnings
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should pass personGeneration option
        #[tokio::test]
        async fn should_pass_person_generation_option() {
            let (model, fetch) = create_mock_model(MockModel::default());

            let mut options = default_options();
            options.provider_options = google_options(json!({"personGeneration": "allow_adult"}));
            model.do_start(&options).await.unwrap();

            assert_eq!(
                start_body(&fetch),
                json!({
                    "instances": [{"prompt": PROMPT}],
                    "parameters": {"sampleCount": 1, "personGeneration": "allow_adult"},
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should pass negativePrompt option
        #[tokio::test]
        async fn should_pass_negative_prompt_option() {
            let (model, fetch) = create_mock_model(MockModel::default());

            let mut options = default_options();
            options.provider_options =
                google_options(json!({"negativePrompt": "blurry, low quality"}));
            model.do_start(&options).await.unwrap();

            assert_eq!(
                start_body(&fetch),
                json!({
                    "instances": [{"prompt": PROMPT}],
                    "parameters": {"sampleCount": 1, "negativePrompt": "blurry, low quality"},
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStart › should pass referenceImages option
        #[tokio::test]
        async fn should_pass_reference_images_option() {
            let (model, fetch) = create_mock_model(MockModel::default());

            let mut options = default_options();
            options.provider_options = google_options(json!({
                "referenceImages": [
                    {"bytesBase64Encoded": "reference-image-data"},
                    {"gcsUri": "gs://bucket/reference.png"},
                ],
            }));
            model.do_start(&options).await.unwrap();

            assert_eq!(
                start_body(&fetch)["instances"][0]["referenceImages"],
                json!([
                    {
                        "image": {"bytesBase64Encoded": "reference-image-data", "mimeType": "image/png"},
                        "referenceType": "asset",
                    },
                    {
                        "image": {"gcsUri": "gs://bucket/reference.png", "mimeType": "image/png"},
                        "referenceType": "asset",
                    },
                ])
            );
        }
    }

    mod do_status {
        use super::*;

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStatus › should return completed with video data when done
        #[tokio::test]
        async fn should_return_completed_with_video_data_when_done() {
            let (model, _) = create_mock_model(MockModel {
                operation_name: "operations/status-test-op",
                videos: json!([{
                    "video": {"uri": "https://generativelanguage.googleapis.com/files/video-456.mp4"},
                }]),
                ..Default::default()
            });

            let result = model
                .do_status(
                    &json!({"operationName": "operations/status-test-op"}),
                    &default_options(),
                )
                .await
                .unwrap();

            let VideoOperationStatus::Completed(result) = result else {
                panic!("expected completed, got {result:?}")
            };
            assert_eq!(result.videos.len(), 1);
            let VideoData::Url { url, media_type } = &result.videos[0] else {
                panic!("expected a URL video, got {:?}", result.videos[0])
            };
            assert_eq!(
                url,
                "https://generativelanguage.googleapis.com/files/video-456.mp4?key=test-api-key"
            );
            assert_eq!(media_type, "video/mp4");
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStatus › should return pending when not done
        #[tokio::test]
        async fn should_return_pending_when_not_done() {
            let (model, _) = model_answering(
                "test-key",
                json!({"name": "operations/pending-op", "done": false}),
            );

            let result = model
                .do_status(
                    &json!({"operationName": "operations/pending-op"}),
                    &default_options(),
                )
                .await
                .unwrap();

            // aimux's `Pending` carries no response: upstream's
            // `response.timestamp` / `response.modelId` have no Rust field.
            assert!(
                matches!(result, VideoOperationStatus::Pending),
                "{result:?}"
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStatus › should return error status on operation error
        #[tokio::test]
        async fn should_return_error_status_on_operation_error() {
            let (model, _) = model_answering(
                "test-key",
                json!({
                    "name": "operations/error-op",
                    "done": true,
                    "error": {
                        "code": 400,
                        "message": "Content policy violation",
                        "status": "FAILED_PRECONDITION",
                    },
                }),
            );

            // aimux maps the `error` status to `Err` (see `VideoOperationStatus`).
            let error = model
                .do_status(
                    &json!({"operationName": "operations/error-op"}),
                    &default_options(),
                )
                .await
                .unwrap_err();

            assert!(
                error.to_string().contains("Content policy violation"),
                "{error}"
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStatus › should throw when no videos in response
        #[tokio::test]
        async fn should_throw_when_no_videos_in_response() {
            let (model, _) = model_answering(
                "test-key",
                json!({
                    "name": "operations/empty-op",
                    "done": true,
                    "response": {"generateVideoResponse": {"generatedSamples": []}},
                }),
            );

            let error = model
                .do_status(
                    &json!({"operationName": "operations/empty-op"}),
                    &default_options(),
                )
                .await
                .unwrap_err();

            assert!(
                wire_message(&error).contains("No videos in response"),
                "{error}"
            );
        }

        /// upstream: @ai-sdk/google src/google-video-model.test.ts › GoogleVideoModel › doStatus › should pass headers to status request
        #[tokio::test]
        async fn should_pass_headers_to_status_request() {
            let (model, fetch) = model_answering(
                "test-key",
                json!({"name": "operations/headers-test", "done": false}),
            );

            let mut options = default_options();
            options.headers = Some(
                [("x-custom".to_string(), "value".to_string())]
                    .into_iter()
                    .collect(),
            );
            model
                .do_status(
                    &json!({"operationName": "operations/headers-test"}),
                    &options,
                )
                .await
                .unwrap();

            let seen = fetch.seen();
            let captured = &seen.last().unwrap().headers;
            assert_eq!(captured.get("x-custom").map(String::as_str), Some("value"));
            assert_eq!(
                captured.get("x-goog-api-key").map(String::as_str),
                Some("test-key")
            );
        }
    }
}

// ── Older ports (previously `tests/google_video_test.rs`) ──
//
// Rust translation of the Google video model tests.
// Source: `reference/ai/packages/google/src/google-video-model.test.ts`

mod older_video {
    use std::sync::{Arc, Mutex};

    use crate::{GoogleProviderSettings, create_google};
    use aimux_core::download::{Download, DownloadRequest, DownloadedFile};
    use aimux_core::provider::Provider;
    use aimux_core::video_model::{VideoCallOptions, generate_video};
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

    /// partial: `should return completed with video data when done`; full port in src/google/tests/video_model.rs.
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
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut opts = options("A cat");
        opts.download = Some(recording_download(requests.clone()));
        let r = generate_video(model.as_ref(), opts).await.unwrap();
        // `generate_video` downloads the returned URL, as the AI SDK does.
        assert_eq!(
            *requests.lock().unwrap(),
            [format!(
                "{}/files/video-123.mp4?key=test-api-key",
                server.uri()
            )]
        );
        assert_eq!(r.videos.len(), 1);
        match &r.videos[0] {
            aimux_core::video_model::VideoData::Binary { data, media_type } => {
                assert_eq!(data, VIDEO_BYTES);
                assert_eq!(media_type, "video/mp4");
            }
            _ => panic!("expected the downloaded video"),
        }
    }

    /// partial: `should pass correct request body`; full port in src/google/tests/video_model.rs.
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

    /// partial: `should pass aspect ratio when provided`; full port in src/google/tests/video_model.rs.
    /// partial: `should pass duration as durationSeconds`; full port in src/google/tests/video_model.rs.
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

    /// partial: `should include timestamp and modelId in response`; full port in src/google/tests/video_model.rs.
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
}
