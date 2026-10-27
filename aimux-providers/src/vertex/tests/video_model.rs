use crate::shared::{Endpoint, EndpointConfig, ResolveEndpoint};
use crate::test_support::mock_fetch::{Canned, RouteFetch, Seen};
use crate::vertex::{
    VertexProvider, VertexProviderSettings, VertexVideoModel, create_google_vertex,
};
use aimux_core::Provider;
use aimux_core::download::{Download, DownloadRequest, DownloadedFile};
use aimux_core::error::AiMuxError;
use aimux_core::shared::{AspectRatio, SharedProviderOptions, Size};
use aimux_core::video_model::{
    VideoCallOptions, VideoFile, VideoFileData, VideoModel, VideoOperationStatus, generate_video,
};
use aimux_provider_utils::HeaderMapOpt;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PROMPT: &str = "A futuristic city with flying cars";

/// A base URL and headers fixed for every request (the upstream model config).
struct StaticEndpoint {
    base_url: String,
    headers: HeaderMapOpt,
}

#[async_trait::async_trait]
impl ResolveEndpoint for StaticEndpoint {
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        Ok(Endpoint {
            base_url: self.base_url.clone(),
            headers: self.headers.clone(),
        })
    }
}

fn json_response(body: &Value) -> Canned {
    Canned::json(body)
}

fn not_found() -> Canned {
    Canned {
        status: 404,
        headers: Vec::new(),
        body: b"Not found".to_vec(),
    }
}

/// `new GoogleVertexVideoModel(modelId, { provider: 'google-vertex', baseURL: 'https://api.example.com', headers: () => ({ 'api-key': 'test-key' }), fetch })`.
fn model_with_fetch(model_id: &str, fetch: &Arc<RouteFetch>) -> VertexVideoModel {
    VertexVideoModel::from_config(
        model_id.to_string(),
        EndpointConfig::new(
            "google-vertex".to_string(),
            StaticEndpoint {
                base_url: "https://api.example.com".to_string(),
                headers: [("api-key".to_string(), Some("test-key".to_string()))].into(),
            },
            Some(fetch.transport()),
        ),
    )
}

/// The parameters of upstream's `createMockModel`.
struct MockModel {
    model_id: &'static str,
    operation_name: &'static str,
    operation_done: bool,
    operation_error: Option<Value>,
    videos: Vec<Value>,
    polls_until_done: usize,
}

impl Default for MockModel {
    fn default() -> Self {
        Self {
            model_id: "veo-2.0-generate-001",
            operation_name: "operations/test-operation-id",
            operation_done: true,
            operation_error: None,
            videos: vec![
                json!({"bytesBase64Encoded": "base64-video-data", "mimeType": "video/mp4"}),
            ],
            polls_until_done: 1,
        }
    }
}

/// `createMockModel(...)`: the model and its recording fetch.
fn create_mock_model(mock: MockModel) -> (VertexVideoModel, Arc<RouteFetch>) {
    let poll_count = AtomicUsize::new(0);
    let MockModel {
        model_id,
        operation_name,
        operation_done,
        operation_error,
        videos,
        polls_until_done,
    } = mock;
    let fetch = RouteFetch::new(move |seen: &Seen| {
        if seen.url.contains(":predictLongRunning") {
            return json_response(&json!({"name": operation_name, "done": false}));
        }
        if seen.url.contains(":fetchPredictOperation") {
            let count = poll_count.fetch_add(1, Ordering::SeqCst) + 1;
            if count < polls_until_done {
                return json_response(&json!({"name": operation_name, "done": false}));
            }
            let mut body = json!({"name": operation_name, "done": operation_done});
            match &operation_error {
                Some(error) => body["error"] = error.clone(),
                None => body["response"] = json!({"videos": videos}),
            }
            return json_response(&body);
        }
        not_found()
    });
    (model_with_fetch(model_id, &fetch), fetch)
}

/// `defaultOptions`.
fn default_options() -> VideoCallOptions {
    VideoCallOptions::new(PROMPT)
}

fn provider_options(value: Value) -> SharedProviderOptions {
    serde_json::from_value(value).unwrap()
}

/// The `:predictLongRunning` body the fetch saw.
fn captured_body(fetch: &RouteFetch) -> Value {
    fetch
        .seen()
        .iter()
        .find(|seen| seen.url.contains(":predictLongRunning"))
        .expect("a predictLongRunning request")
        .json_body()
}

async fn start_with(options: VideoCallOptions) -> Value {
    let (model, fetch) = create_mock_model(MockModel::default());
    model.do_start(&options).await.unwrap();
    captured_body(&fetch)
}

/// The message of an error, without the variant's display prefix.
fn message(error: &AiMuxError) -> String {
    match error {
        AiMuxError::InvalidArgument { message, .. }
        | AiMuxError::InvalidResponseData { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

mod google_vertex_video_model {
    use super::*;

    mod constructor {
        use super::*;

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › constructor › should expose correct provider and model information
        #[test]
        fn should_expose_correct_provider_and_model_information() {
            let (model, _) = create_mock_model(MockModel::default());
            assert_eq!(model.provider(), "google-vertex");
            assert_eq!(model.model_id(), "veo-2.0-generate-001");
            assert_eq!(model.specification_version(), "v4");
            assert_eq!(model.max_videos_per_call(), Some(4));
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › constructor › should support different model IDs
        #[test]
        fn should_support_different_model_ids() {
            let (model, _) = create_mock_model(MockModel {
                model_id: "veo-3.0-generate-001",
                ..Default::default()
            });
            assert_eq!(model.model_id(), "veo-3.0-generate-001");
        }
    }

    mod do_start {
        use super::*;

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › should return operation with operationName
        #[tokio::test]
        async fn should_return_operation_with_operation_name() {
            let (model, _) = create_mock_model(MockModel {
                operation_name: "operations/my-op-123",
                ..Default::default()
            });
            let result = model.do_start(&default_options()).await.unwrap();
            assert_eq!(
                result.operation,
                json!({"operationName": "operations/my-op-123"})
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › should pass correct request body
        #[tokio::test]
        async fn should_pass_correct_request_body() {
            assert_eq!(
                start_with(default_options()).await,
                json!({"instances": [{"prompt": PROMPT}], "parameters": {"sampleCount": 1}})
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › should pass seed when provided
        #[tokio::test]
        async fn should_pass_seed_when_provided() {
            let mut options = default_options();
            options.seed = Some(42);
            assert_eq!(
                start_with(options).await,
                json!({"instances": [{"prompt": PROMPT}], "parameters": {"sampleCount": 1, "seed": 42}})
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › should pass aspect ratio when provided
        #[tokio::test]
        async fn should_pass_aspect_ratio_when_provided() {
            let mut options = default_options();
            options.aspect_ratio = Some(AspectRatio::new(16, 9));
            assert_eq!(
                start_with(options).await,
                json!({"instances": [{"prompt": PROMPT}], "parameters": {"sampleCount": 1, "aspectRatio": "16:9"}})
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › should convert resolution to Vertex format
        #[tokio::test]
        async fn should_convert_resolution_to_vertex_format() {
            let mut options = default_options();
            options.resolution = Some(Size::new(1920, 1080));
            assert_eq!(
                start_with(options).await,
                json!({"instances": [{"prompt": PROMPT}], "parameters": {"sampleCount": 1, "resolution": "1080p"}})
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › should pass duration as durationSeconds
        #[tokio::test]
        async fn should_pass_duration_as_duration_seconds() {
            let mut options = default_options();
            options.duration = Some(5);
            assert_eq!(
                start_with(options).await,
                json!({"instances": [{"prompt": PROMPT}], "parameters": {"sampleCount": 1, "durationSeconds": 5}})
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › should pass n as sampleCount
        #[tokio::test]
        async fn should_pass_n_as_sample_count() {
            let mut options = default_options();
            options.n = 2;
            assert_eq!(
                start_with(options).await,
                json!({"instances": [{"prompt": PROMPT}], "parameters": {"sampleCount": 2}})
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › should return warnings array
        #[tokio::test]
        async fn should_return_warnings_array() {
            let (model, _) = create_mock_model(MockModel::default());
            let result = model.do_start(&default_options()).await.unwrap();
            assert!(result.warnings.is_empty());
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › should throw when no operation name returned
        #[tokio::test]
        async fn should_throw_when_no_operation_name_returned() {
            let fetch = RouteFetch::new(|_| json_response(&json!({"done": false})));
            let model = model_with_fetch("veo-2.0-generate-001", &fetch);
            let error = model.do_start(&default_options()).await.unwrap_err();
            assert_eq!(message(&error), "No operation name returned from API");
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › should include warnings and response metadata
        #[tokio::test]
        async fn should_include_warnings_and_response_metadata() {
            // aimux cannot inject `currentDate`: the timestamp is asserted to
            // be set, not to equal a fixed date.
            let before = aimux_core::Timestamp::now();
            let (model, _) = create_mock_model(MockModel::default());
            let result = model.do_start(&default_options()).await.unwrap();
            assert!(result.warnings.is_empty());
            let timestamp = result.response.timestamp.expect("timestamp");
            assert!(timestamp.unix_millis() >= before.unix_millis());
            assert_eq!(
                result.response.model_id.as_deref(),
                Some("veo-2.0-generate-001")
            );
        }

        mod image_to_video {
            use super::*;

            /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › Image-to-Video › should send image as bytesBase64Encoded
            #[tokio::test]
            async fn should_send_image_as_bytes_base64_encoded() {
                let mut options = default_options();
                options.image = Some(VideoFile::File {
                    media_type: "image/png".into(),
                    data: VideoFileData::Base64("base64-image-data".into()),
                });
                let body = start_with(options).await;
                // `toMatchObject`
                assert_eq!(
                    body["instances"],
                    json!([{
                        "prompt": PROMPT,
                        "image": {"bytesBase64Encoded": "base64-image-data", "mimeType": "image/png"},
                    }])
                );
                assert_eq!(body["parameters"]["sampleCount"], json!(1));
            }

            /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › Image-to-Video › should warn when URL-based image is provided
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
                let warning = serde_json::to_value(&result.warnings[0]).unwrap();
                assert_eq!(warning["type"], "unsupported");
                assert_eq!(warning["feature"], "URL-based image input");
            }
        }

        mod provider_options {
            use super::*;

            /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › Provider Options › should pass personGeneration option
            #[tokio::test]
            async fn should_pass_person_generation_option() {
                let mut options = default_options();
                options.provider_options =
                    provider_options(json!({"vertex": {"personGeneration": "allow_adult"}}));
                assert_eq!(
                    start_with(options).await,
                    json!({
                        "instances": [{"prompt": PROMPT}],
                        "parameters": {"sampleCount": 1, "personGeneration": "allow_adult"},
                    })
                );
            }

            /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › Provider Options › should pass negativePrompt option
            #[tokio::test]
            async fn should_pass_negative_prompt_option() {
                let mut options = default_options();
                options.provider_options =
                    provider_options(json!({"vertex": {"negativePrompt": "blurry, low quality"}}));
                assert_eq!(
                    start_with(options).await,
                    json!({
                        "instances": [{"prompt": PROMPT}],
                        "parameters": {"sampleCount": 1, "negativePrompt": "blurry, low quality"},
                    })
                );
            }

            /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › Provider Options › should pass generateAudio option
            #[tokio::test]
            async fn should_pass_generate_audio_option() {
                let mut options = default_options();
                options.provider_options =
                    provider_options(json!({"vertex": {"generateAudio": true}}));
                assert_eq!(
                    start_with(options).await,
                    json!({
                        "instances": [{"prompt": PROMPT}],
                        "parameters": {"sampleCount": 1, "generateAudio": true},
                    })
                );
            }

            /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › Provider Options › should pass gcsOutputDirectory option
            #[tokio::test]
            async fn should_pass_gcs_output_directory_option() {
                let mut options = default_options();
                options.provider_options = provider_options(
                    json!({"vertex": {"gcsOutputDirectory": "gs://bucket/output/"}}),
                );
                assert_eq!(
                    start_with(options).await,
                    json!({
                        "instances": [{"prompt": PROMPT}],
                        "parameters": {"sampleCount": 1, "gcsOutputDirectory": "gs://bucket/output/"},
                    })
                );
            }

            /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStart › Provider Options › should pass referenceImages option
            #[tokio::test]
            async fn should_pass_reference_images_option() {
                let mut options = default_options();
                options.provider_options = provider_options(json!({
                    "vertex": {
                        "referenceImages": [
                            {"bytesBase64Encoded": "reference-image-data"},
                            {"gcsUri": "gs://bucket/reference.png"},
                        ],
                    },
                }));
                let body = start_with(options).await;
                assert_eq!(
                    body["instances"][0]["referenceImages"],
                    json!([
                        {"bytesBase64Encoded": "reference-image-data"},
                        {"gcsUri": "gs://bucket/reference.png"},
                    ])
                );
            }
        }
    }

    mod do_status {
        use super::*;

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStatus › should return completed with video data when done
        #[tokio::test]
        async fn should_return_completed_with_video_data_when_done() {
            let (model, _) = create_mock_model(MockModel {
                operation_name: "operations/my-op-123",
                ..Default::default()
            });
            let result = model
                .do_status(
                    &json!({"operationName": "operations/my-op-123"}),
                    &default_options(),
                )
                .await
                .unwrap();
            let VideoOperationStatus::Completed(result) = result else {
                panic!("expected status completed, got pending");
            };
            assert_eq!(
                serde_json::to_value(result.videos).unwrap(),
                json!([{"type": "base64", "data": "base64-video-data", "mediaType": "video/mp4"}])
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStatus › should return pending when not done
        #[tokio::test]
        async fn should_return_pending_when_not_done() {
            let (model, _) = create_mock_model(MockModel {
                operation_name: "operations/my-op-123",
                polls_until_done: 3,
                ..Default::default()
            });
            let result = model
                .do_status(
                    &json!({"operationName": "operations/my-op-123"}),
                    &default_options(),
                )
                .await
                .unwrap();
            assert!(matches!(result, VideoOperationStatus::Pending));
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStatus › should use POST with operationName in body
        #[tokio::test]
        async fn should_use_post_with_operation_name_in_body() {
            let fetch = RouteFetch::new(|seen: &Seen| {
                if seen.url.contains(":predictLongRunning") {
                    return json_response(&json!({"name": "operations/check-post", "done": false}));
                }
                if seen.url.contains(":fetchPredictOperation") {
                    return json_response(&json!({
                        "name": "operations/check-post",
                        "done": true,
                        "response": {"videos": [{"bytesBase64Encoded": "video-data", "mimeType": "video/mp4"}]},
                    }));
                }
                not_found()
            });
            let model = model_with_fetch("veo-2.0-generate-001", &fetch);
            model
                .do_status(
                    &json!({"operationName": "operations/check-post"}),
                    &default_options(),
                )
                .await
                .unwrap();
            let seen = fetch.seen();
            let poll = seen
                .iter()
                .find(|seen| seen.url.contains(":fetchPredictOperation"))
                .expect("captured url contains :fetchPredictOperation");
            assert_eq!(poll.method, "POST");
            assert_eq!(
                poll.json_body(),
                json!({"operationName": "operations/check-post"})
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-video-model.test.ts › GoogleVertexVideoModel › doStatus › should return completed with GCS URI video
        #[tokio::test]
        async fn should_return_completed_with_gcs_uri_video() {
            let (model, _) = create_mock_model(MockModel {
                operation_name: "operations/gcs-op",
                videos: vec![json!({"gcsUri": "gs://bucket/video.mp4", "mimeType": "video/mp4"})],
                ..Default::default()
            });
            let result = model
                .do_status(
                    &json!({"operationName": "operations/gcs-op"}),
                    &default_options(),
                )
                .await
                .unwrap();
            let VideoOperationStatus::Completed(result) = result else {
                panic!("expected status completed, got pending");
            };
            assert_eq!(
                serde_json::to_value(result.videos).unwrap(),
                json!([{"type": "url", "url": "gs://bucket/video.mp4", "mediaType": "video/mp4"}])
            );
        }
    }
}

// ── Older ports (previously `tests/vertex_video_test.rs`) ──
//
// Rust translation of the Google Vertex video model tests.
// Source: `reference/ai/packages/google-vertex/src/google-vertex-video-model.test.ts`

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
