//! Offline Ark wire-contract tests based on the official API schemas.
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageOutputs, generate_image,
};
use aimux_core::video_model::{
    VideoCallOptions, VideoFile, VideoFileData, VideoFrameImage, VideoFrameType,
    VideoOperationStatus,
};
use aimux_core::{AiMuxError, ImageModel, VideoModel};
use aimux_provider_utils::{Fetch, FetchError, FetchRequest, FetchResponse, Resolvable};
use aimux_providers::{
    ByteDanceProvider, ByteDanceProviderSettings, create_bytedance, create_provider,
    preset::PresetSettings,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

struct CountingFetch(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl Fetch for CountingFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        aimux_provider_utils::default_fetch().fetch(request).await
    }
}

fn provider(server: &MockServer) -> ByteDanceProvider {
    create_bytedance(ByteDanceProviderSettings {
        base_url: Some(format!("{}/relay/api/v3/", server.uri())),
        api_key: Some(Resolvable::from("mock-key".to_owned())),
        ..Default::default()
    })
    .unwrap()
}
fn video_file() -> VideoFile {
    VideoFile::File {
        media_type: "image/png".into(),
        data: VideoFileData::Binary(b"image".to_vec()),
    }
}
fn opts(value: Value) -> aimux_core::shared::SharedProviderOptions {
    serde_json::from_value(json!({"bytedance":value})).unwrap()
}
async fn image_mock(server: &MockServer, value: Value) {
    Mock::given(method("POST"))
        .and(path("/relay/api/v3/images/generations"))
        .and(header("authorization", "Bearer mock-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn seedance_create_preserves_options_references_and_relay_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/relay/api/v3/contents/generations/tasks"))
        .and(header("authorization", "Bearer mock-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"cgt-1"})))
        .expect(1)
        .mount(&server)
        .await;
    let mut call = VideoCallOptions::new("a cat");
    call.input_references = Some(vec![video_file()]);
    call.duration = Some(5);
    call.fps = Some(24);
    call.seed = Some(42);
    call.generate_audio = Some(true);
    call.provider_options = opts(
        json!({"resolution":"720p", "ratio":"16:9", "return_last_frame":true, "service_tier":"default", "execution_expires_after":3600, "callback_url":"https://example.com/callback", "imageUrls":["https://example.com/a.png"], "videoUrls":["https://example.com/a.mp4"], "audioUrls":["https://example.com/a.wav"]}),
    );
    let result = provider(&server)
        .video("relay-seedance")
        .do_start(&call)
        .await
        .unwrap();
    assert_eq!(result.operation, json!({"id":"cgt-1"}));
    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(body["model"], "relay-seedance");
    assert_eq!(body["duration"], 5);
    assert_eq!(body["framespersecond"], 24);
    assert_eq!(body["seed"], 42);
    assert_eq!(body["content"][0], json!({"type":"text", "text":"a cat"}));
    assert_eq!(
        body["content"][1],
        json!({"type":"image_url", "image_url":{"url":"data:image/png;base64,aW1hZ2U="},"role":"reference_image"})
    );
    assert_eq!(
        body["content"][3]["video_url"]["url"],
        "https://example.com/a.mp4"
    );
    assert_eq!(body["content"][4]["role"], "reference_audio");
    assert!(body.get("imageUrls").is_none());
}

#[tokio::test]
async fn seedance_frames_and_media_only_inputs() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"cgt-1"})))
        .mount(&server)
        .await;
    let mut call = VideoCallOptions::new("");
    call.frame_images = Some(vec![
        VideoFrameImage {
            image: video_file(),
            frame_type: VideoFrameType::FirstFrame,
        },
        VideoFrameImage {
            image: video_file(),
            frame_type: VideoFrameType::LastFrame,
        },
    ]);
    provider(&server)
        .video("seedance")
        .do_start(&call)
        .await
        .unwrap();
    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(body["content"][0]["role"], "first_frame");
    assert_eq!(body["content"][1]["role"], "last_frame");
}

#[tokio::test]
async fn seedance_statuses_and_last_frame() {
    for status in [
        "queued",
        "running",
        "succeeded",
        "failed",
        "cancelled",
        "expired",
        "unknown",
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/relay/api/v3/contents/generations/tasks/cgt-1")).and(header("authorization", "Bearer mock-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"cgt-1", "status":status,"content":{"video_url":"https://example.com/output.mp4","last_frame_url":"https://example.com/last.png"}, "error":{"message":"task stopped"}}))).expect(1).mount(&server).await;
        let result = provider(&server)
            .video("seedance")
            .do_status(&json!({"id":"cgt-1"}), &VideoCallOptions::new("cat"))
            .await;
        match status {
            "queued" | "running" => {
                assert!(matches!(result.unwrap(), VideoOperationStatus::Pending))
            }
            "succeeded" => {
                let VideoOperationStatus::Completed(result) = result.unwrap() else {
                    panic!()
                };
                assert_eq!(result.videos.len(), 1);
                assert_eq!(
                    result.provider_metadata.unwrap()["bytedance"]["lastFrameUrl"],
                    "https://example.com/last.png"
                );
            }
            "unknown" => assert!(matches!(
                result,
                Err(AiMuxError::InvalidResponseData { .. })
            )),
            _ => {
                let Err(AiMuxError::ApiCall(error)) = result else {
                    panic!()
                };
                assert_eq!(error.message, "task stopped");
                assert_eq!(error.is_retryable, status == "expired");
            }
        }
    }
}

#[tokio::test]
async fn seedance_rejects_invalid_operation_and_response() {
    let server = MockServer::start().await;
    let model = provider(&server).video("seedance");
    assert!(
        model
            .do_status(&json!({}), &VideoCallOptions::new("cat"))
            .await
            .is_err()
    );
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"succeeded"})))
        .mount(&server)
        .await;
    assert!(
        model
            .do_status(&json!({"id":"x"}), &VideoCallOptions::new("cat"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn seedance_invalid_inputs_fail_without_network() {
    let server = MockServer::start().await;
    let model = provider(&server).video("seedance");
    for value in [
        json!({"duration":"five"}),
        json!({"videoUrls":["data:video/mp4;base64,AA=="]}),
        json!({"unknown":true}),
        json!({"imageUrls":true}),
    ] {
        let mut call = VideoCallOptions::new("cat");
        call.provider_options = opts(value);
        assert!(matches!(
            model.do_start(&call).await,
            Err(AiMuxError::InvalidArgument { .. })
        ));
    }
    let mut call = VideoCallOptions::new("cat");
    call.image = Some(video_file());
    call.input_references = Some(vec![video_file()]);
    assert!(model.do_start(&call).await.is_err());
    call.input_references = None;
    call.image = Some(VideoFile::File {
        media_type: "video/mp4".into(),
        data: VideoFileData::Binary(vec![1]),
    });
    assert!(model.do_start(&call).await.is_err());
    call.image = Some(VideoFile::Url {
        url: "https://example.com/a.mp4".into(),
        media_type: Some("video/mp4".into()),
    });
    assert!(model.do_start(&call).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn seedream_json_editing_base64_and_metadata() {
    let server = MockServer::start().await;
    image_mock(&server, json!({"data":[{"b64_json":"aW1hZ2U=", "size":"2048x2048"}],"usage":{"output_tokens":5,"total_tokens":5}})).await;
    let mut call = ImageCallOptions::new("edit this");
    call.files = Some(vec![
        ImageFile::File {
            media_type: "image/png".into(),
            data: ImageFileData::Binary(b"image".to_vec()),
        },
        ImageFile::Url {
            url: "https://example.com/ref.png".into(),
        },
    ]);
    call.provider_options = opts(
        json!({"watermark":false,"background":"transparent", "sequential_image_generation":"auto", "sequential_image_generation_options":{"max_images":2},"optimize_prompt_options":{"mode":"standard"},"guidance_scale":5.0,"size":"2K"}),
    );
    let result = provider(&server)
        .image("seedream")
        .do_generate(&call)
        .await
        .unwrap();
    let ImageOutputs::Binary(images) = result.images else {
        panic!()
    };
    assert_eq!(images, vec![b"image".to_vec()]);
    assert_eq!(result.usage.unwrap().total_tokens, Some(5));
    assert_eq!(
        result.provider_metadata.unwrap()["bytedance"]["images"][0]["size"],
        "2048x2048"
    );
    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(
        body["image"],
        json!([
            "data:image/png;base64,aW1hZ2U=",
            "https://example.com/ref.png"
        ])
    );
    assert_eq!(body["size"], "2K");
    assert_eq!(body["watermark"], false);
    assert!(body.get("n").is_none());
}

#[tokio::test]
async fn seedream_single_reference_and_url_download_are_credential_free() {
    let server = MockServer::start().await;
    image_mock(
        &server,
        json!({"data":[{"url":format!("{}/asset.png",server.uri())}]}),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/asset.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"image"))
        .expect(1)
        .mount(&server)
        .await;
    let mut call = ImageCallOptions::new("cat");
    call.files = Some(vec![ImageFile::Url {
        url: "https://example.com/ref.png".into(),
    }]);
    call.provider_options = opts(json!({"response_format":"url"}));
    provider(&server)
        .image("seedream")
        .do_generate(&call)
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: Value = requests[0].body_json().unwrap();
    assert_eq!(body["image"], "https://example.com/ref.png");
    assert!(!requests[1].headers.contains_key("authorization"));
}

#[tokio::test]
async fn seedream_stream_collects_images_and_rejects_incomplete_or_error_events() {
    for (events, success) in [
        (
            "data: {\"type\":\"image_generation.partial_succeeded\",\"b64_json\":\"aW1hZ2U=\"}\n\ndata: {\"type\":\"image_generation.completed\",\"usage\":{\"total_tokens\":5}}\n\n",
            true,
        ),
        (
            "data: {\"type\":\"image_generation.partial_succeeded\",\"b64_json\":\"aW1hZ2U=\"}\n\n",
            false,
        ),
        (
            "data: {\"type\":\"image_generation.partial_failed\",\"error\":{\"message\":\"failed\"}}\n\ndata: {\"type\":\"image_generation.completed\"}\n\n",
            false,
        ),
        (
            "data: not-json\n\ndata: {\"type\":\"image_generation.completed\"}\n\n",
            false,
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(events),
            )
            .mount(&server)
            .await;
        let mut call = ImageCallOptions::new("cat");
        call.provider_options = opts(json!({"stream":true}));
        let result = provider(&server).image("seedream").do_generate(&call).await;
        assert_eq!(result.is_ok(), success);
        if let Ok(result) = result {
            assert_eq!(result.usage.unwrap().total_tokens, Some(5));
        }
    }
}

#[tokio::test]
async fn seedream_rejects_empty_or_invalid_success_payloads() {
    for value in [
        json!({"data":[]}),
        json!({"data":[{"b64_json":""}]}),
        json!({"data":[{"b64_json":"!invalid"}]}),
        json!({"data":[{}]}),
        json!({"data":[{"error":{"message":"blocked"}}]}),
    ] {
        let server = MockServer::start().await;
        image_mock(&server, value).await;
        assert!(
            provider(&server)
                .image("seedream")
                .do_generate(&ImageCallOptions::new("cat"))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn generation_and_download_failures_never_replay_billed_calls() {
    for status in [429, 500] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(json!({"error":{"message":"busy"}})),
            )
            .expect(2)
            .mount(&server)
            .await;
        let mut call = ImageCallOptions::new("cat");
        call.max_retries = Some(2);
        assert!(
            generate_image(&provider(&server).image("seedream"), call)
                .await
                .is_err()
        );
        let error = provider(&server)
            .video("seedance")
            .do_start(&VideoCallOptions::new("cat"))
            .await
            .unwrap_err();
        let AiMuxError::ApiCall(error) = error else {
            panic!()
        };
        assert!(!error.is_retryable);
    }
    let server = MockServer::start().await;
    image_mock(
        &server,
        json!({"data":[{"url":format!("{}/asset",server.uri())}]}),
    )
    .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let mut call = ImageCallOptions::new("cat");
    call.max_retries = Some(2);
    assert!(
        generate_image(&provider(&server).image("seedream"), call)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn unified_factory_keeps_chat_discovery_and_media_transport() {
    let server = MockServer::start().await;
    let count = Arc::new(AtomicUsize::new(0));
    let transport = Arc::new(CountingFetch(count.clone()));
    let provider = create_provider(
        "bytedance",
        PresetSettings {
            base_url: Some(format!("{}/relay/api/v3", server.uri())),
            api_key: Some(Resolvable::from("mock-key".to_owned())),
            fetch: Some(transport),
            transform_request_body: Some(Arc::new(|mut body| {
                body["relay_marker"] = json!(true);
                body
            })),
            headers: Some(std::collections::HashMap::from([(
                "x-relay".into(),
                Some("yes".into()),
            )])),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        provider.language_model("chat-model").unwrap().provider(),
        "bytedance.chat"
    );
    assert!(provider.embedding_model("embedding").is_ok());
    assert!(provider.discovery().is_some());
    Mock::given(method("POST"))
        .and(path("/relay/api/v3/images/generations"))
        .and(header("x-relay", "yes"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":[{"b64_json":"aW1hZ2U="}]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    provider
        .image_model("relay-seedream")
        .unwrap()
        .do_generate(&ImageCallOptions::new("cat"))
        .await
        .unwrap();
    assert_eq!(
        provider
            .video_model("relay-seedance")
            .unwrap()
            .unwrap()
            .provider(),
        "bytedance.video"
    );
    Mock::given(method("POST")).and(path("/relay/api/v3/chat/completions")).and(header("x-relay", "yes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"chat-1","model":"chat-model","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]}))).expect(1).mount(&server).await;
    let chat = provider.language_model("chat-model").unwrap();
    let response = chat
        .do_generate(&aimux_core::options::CallOptions::new(vec![
            aimux_core::language_model_message::LanguageModelMessage::user_text("hi"),
        ]))
        .await
        .unwrap();
    assert!(!response.content.is_empty());
    Mock::given(method("POST"))
        .and(path("/relay/api/v3/contents/generations/tasks"))
        .and(header("x-relay", "yes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"cgt-1"})))
        .expect(1)
        .mount(&server)
        .await;
    provider
        .video_model("relay-seedance")
        .unwrap()
        .unwrap()
        .do_start(&VideoCallOptions::new("cat"))
        .await
        .unwrap();
    for request in server.received_requests().await.unwrap() {
        let body: Value = request.body_json().unwrap();
        assert_eq!(body["relay_marker"], true);
    }
    assert_eq!(count.load(Ordering::Relaxed), 3);
}

#[test]
fn factory_validates_url_without_reading_credentials() {
    assert!(create_bytedance(ByteDanceProviderSettings::default()).is_ok());
    assert!(
        create_bytedance(ByteDanceProviderSettings {
            base_url: Some("file:///tmp/ark".into()),
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        !format!(
            "{:?}",
            ByteDanceProviderSettings {
                api_key: Some(Resolvable::from("never-print-this".to_owned())),
                ..Default::default()
            }
        )
        .contains("never-print-this")
    );
}
