//! Compatible gateway video protocol and credential-boundary regressions (#530).
use std::collections::HashMap;

use aimux_core::provider::Provider;
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoFileData, VideoFrameImage, VideoFrameType,
    VideoModel, VideoOperationStatus, VideoPollOptions, generate_video,
};
use aimux_providers::openai_compatible::{
    OpenAICompatibleProvider, OpenAICompatibleProviderSettings, create_openai_compatible,
};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn provider(server: &MockServer) -> OpenAICompatibleProvider {
    create_openai_compatible(OpenAICompatibleProviderSettings {
        name: "gateway".into(),
        base_url: format!("{}/v1", server.uri()),
        api_key: Some("fixture-key".into()),
        ..Default::default()
    })
    .unwrap()
}

/// The same gateway behind an `api-version` query parameter, as Azure-style
/// relays and some compatible gateways authenticate.
fn provider_with_api_version(server: &MockServer) -> OpenAICompatibleProvider {
    create_openai_compatible(OpenAICompatibleProviderSettings {
        name: "gateway".into(),
        base_url: format!("{}/v1", server.uri()),
        api_key: Some("fixture-key".into()),
        query_params: Some(HashMap::from([(
            "api-version".to_string(),
            "2024-01".to_string(),
        )])),
        ..Default::default()
    })
    .unwrap()
}
fn image(bytes: &[u8]) -> VideoFile {
    VideoFile::File {
        media_type: "image/png".into(),
        data: VideoFileData::Binary(bytes.to_vec()),
    }
}
async fn status(server: &MockServer, body: Value) {
    Mock::given(method("GET"))
        .and(path("/v1/videos/task"))
        .and(header("authorization", "Bearer fixture-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn multipart_maps_standard_fields_and_repeated_reference_files() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/videos"))
        .and(header("authorization", "Bearer fixture-key"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id":"task","status":"queued"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let p = provider(&server);
    let model = p.video_model("minimax-h3").unwrap().unwrap();
    assert_eq!(model.provider(), "gateway.video");
    let mut options = VideoCallOptions::new("a moving cat");
    options.duration = Some(4);
    options.resolution = Some("1344x768".parse().unwrap());
    // Deliberately reversed input: the wire is first, then last.
    options.frame_images = Some(vec![
        VideoFrameImage {
            frame_type: VideoFrameType::LastFrame,
            image: image(b"last"),
        },
        VideoFrameImage {
            frame_type: VideoFrameType::FirstFrame,
            image: image(b"first"),
        },
    ]);
    options.provider_options.insert(
        "gateway".into(),
        json!({
            "extra_params": {"task":"fl2va","duration":4,"frame_indices":[0,-1]},
            "width": 1344, "flow_shift": 2.5, "custom": "literal"
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    assert_eq!(
        model.do_start(&options).await.unwrap().operation,
        json!({"id":"task"})
    );
    let requests = server.received_requests().await.unwrap();
    let body = String::from_utf8_lossy(&requests[0].body);
    assert_eq!(body.matches("name=\"input_references\"").count(), 2);
    assert!(!body.contains("name=\"input_reference\""));
    assert!(body.find("first").unwrap() < body.find("last").unwrap());
    for (field, value) in [
        ("model", "minimax-h3"),
        ("prompt", "a moving cat"),
        ("seconds", "4"),
        ("size", "1344x768"),
        ("width", "1344"),
        ("flow_shift", "2.5"),
        ("custom", "literal"),
    ] {
        assert!(
            body.contains(&format!("name=\"{field}\"\r\n\r\n{value}\r\n")),
            "{body}"
        );
    }
    assert!(body.contains("\"frame_indices\":[0,-1]"));
}

#[tokio::test]
async fn single_reference_uses_the_singular_field() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/videos"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id":"task","status":"queued"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let mut options = VideoCallOptions::new("a moving cat");
    options.image = Some(image(b"first"));
    provider(&server)
        .video("m")
        .do_start(&options)
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let body = String::from_utf8_lossy(&requests[0].body);
    // vLLM-Omni reads one file from `input_reference`; a repeated plural part
    // would leave it unset.
    assert_eq!(body.matches("name=\"input_reference\"").count(), 1);
    assert!(!body.contains("name=\"input_references\""));
}

#[tokio::test]
async fn core_does_not_resubmit_failed_creation() {
    for response in [
        ResponseTemplate::new(503)
            .set_body_json(json!({"error":{"message":"accepted then failed"}})),
        ResponseTemplate::new(200).set_body_json(json!({"status":"queued"})),
        ResponseTemplate::new(200)
            .set_body_json(json!({"id":"task","status":"failed","error":"no capacity"})),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        let error = generate_video(
            &provider(&server).video("test"),
            VideoCallOptions::new("cat"),
        )
        .await
        .unwrap_err();
        assert!(!error.is_retryable());
    }
}

#[tokio::test]
async fn pending_status_aliases_and_terminal_errors() {
    for state in [
        "queued",
        "pending",
        "processing",
        "in_progress",
        "running",
        // A gateway alias the model does not know keeps polling instead of
        // failing the task; the poll timeout bounds it.
        "gateway_alias",
    ] {
        let server = MockServer::start().await;
        status(&server, json!({"id":"task","status":state})).await;
        assert!(matches!(
            provider(&server)
                .video("m")
                .do_status(&json!({"id":"task"}), &VideoCallOptions::new("x"))
                .await
                .unwrap(),
            VideoOperationStatus::Pending
        ));
    }
    for state in ["failed", "cancelled", "canceled", "expired", "error"] {
        let server = MockServer::start().await;
        status(
            &server,
            json!({"id":"task","status":state,"error":{"message":"provider detail", "code":"quota"}}),
        )
        .await;
        let error = provider(&server)
            .video("m")
            .do_status(&json!({"id":"task"}), &VideoCallOptions::new("x"))
            .await
            .unwrap_err();
        assert!(!error.is_retryable());
        assert!(error.to_string().contains("provider detail"));
        assert!(error.to_string().contains("quota"));
    }
}

#[tokio::test]
async fn foreign_absolute_url_is_returned_without_downloading() {
    let server = MockServer::start().await;
    let asset = MockServer::start().await;
    let url = format!("{}/public.mp4", asset.uri());
    status(&server, json!({"id":"task","status":"completed","url":url})).await;
    let VideoOperationStatus::Completed(result) = provider(&server)
        .video("m")
        .do_status(&json!({"id":"task"}), &VideoCallOptions::new("x"))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(matches!(&result.videos[0], VideoData::Url { url: actual, .. } if actual == &url));
    assert!(asset.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn same_origin_absolute_url_is_downloaded_with_credentials() {
    let server = MockServer::start().await;
    let url = format!("{}/files/movie.mp4", server.uri());
    status(&server, json!({"id":"task","status":"completed","url":url})).await;
    Mock::given(method("GET"))
        .and(path("/files/movie.mp4"))
        .and(query_param("api-version", "2024-01"))
        .and(header("authorization", "Bearer fixture-key"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"video".to_vec()))
        .expect(1)
        .mount(&server)
        .await;
    let VideoOperationStatus::Completed(result) = provider_with_api_version(&server)
        .video("m")
        .do_status(&json!({"id":"task"}), &VideoCallOptions::new("x"))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(matches!(&result.videos[0], VideoData::Binary { data, .. } if data == b"video"));
}

#[tokio::test]
async fn relative_asset_keeps_the_gateway_query_parameters() {
    let server = MockServer::start().await;
    status(
        &server,
        json!({
            "id":"task",
            "status":"succeeded",
            "url":"assets/movie.mp4?Expires=1&Signature=abc"
        }),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/v1/assets/movie.mp4"))
        // The gateway's configured parameter is added, and the signature the
        // response's own URL carried stays.
        .and(query_param("api-version", "2024-01"))
        .and(query_param("Expires", "1"))
        .and(query_param("Signature", "abc"))
        .and(header("authorization", "Bearer fixture-key"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"video".to_vec()))
        .expect(1)
        .mount(&server)
        .await;
    let VideoOperationStatus::Completed(result) = provider_with_api_version(&server)
        .video("m")
        .do_status(&json!({"id":"task"}), &VideoCallOptions::new("x"))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(matches!(&result.videos[0], VideoData::Binary { data, .. } if data == b"video"));
}

#[tokio::test]
async fn relative_and_default_content_are_downloaded_with_credentials() {
    for (url, expected_path) in [
        (None, "/v1/videos/task/content"),
        (Some("assets/movie.mp4"), "/v1/assets/movie.mp4"),
        (Some("/files/movie.mp4"), "/files/movie.mp4"),
    ] {
        let server = MockServer::start().await;
        let mut body = json!({"id":"task","status":"succeeded"});
        if let Some(url) = url {
            body["url"] = json!(url);
        }
        status(&server, body).await;
        Mock::given(method("GET"))
            .and(path(expected_path))
            .and(header("authorization", "Bearer fixture-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(b"video".to_vec())
                    .insert_header("content-type", "video/webm"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let VideoOperationStatus::Completed(result) = provider(&server)
            .video("m")
            .do_status(&json!({"id":"task"}), &VideoCallOptions::new("x"))
            .await
            .unwrap()
        else {
            panic!()
        };
        assert!(
            matches!(&result.videos[0], VideoData::Binary { data, media_type } if data == b"video" && media_type == "video/webm")
        );
    }
}

#[tokio::test]
async fn invalid_asset_urls_are_not_credentialed_requests() {
    for url in [
        "file:///etc/passwd",
        "data:video/mp4;base64,eA==",
        "ftp://example.com/movie",
        "//example.com/movie",
        "\\\\example.com/movie",
        "http://user:secret@example.com/movie",
        "",
        " http://example.com/movie",
    ] {
        let server = MockServer::start().await;
        status(&server, json!({"id":"task","status":"completed","url":url})).await;
        let error = provider(&server)
            .video("m")
            .do_status(&json!({"id":"task"}), &VideoCallOptions::new("x"))
            .await
            .unwrap_err();
        assert!(!error.is_retryable(), "{url}");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn credentialed_redirect_cannot_reach_a_private_foreign_origin() {
    let server = MockServer::start().await;
    let foreign = MockServer::start().await;
    status(&server, json!({"id":"task","status":"completed"})).await;
    Mock::given(path("/v1/videos/task/content"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/stolen", foreign.uri())),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        provider(&server)
            .video("m")
            .do_status(&json!({"id":"task"}), &VideoCallOptions::new("x"))
            .await
            .is_err()
    );
    assert!(foreign.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn query_failures_remain_retryable_without_resubmitting_creation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/videos"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"task"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/v1/videos/task"))
        .respond_with(
            ResponseTemplate::new(503).set_body_json(json!({"error":{"message":"temporary"}})),
        )
        .expect(2)
        .mount(&server)
        .await;
    let mut options = VideoCallOptions::new("cat");
    options.max_retries = Some(1);
    options.poll = Some(VideoPollOptions {
        interval_ms: Some(1),
        timeout_ms: Some(5000),
    });
    assert!(
        generate_video(&provider(&server).video("m"), options)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn invalid_reference_options_fail_before_submission() {
    let server = MockServer::start().await;
    let mut options = VideoCallOptions::new("cat");
    options.image = Some(image(b"first"));
    options.frame_images = Some(vec![VideoFrameImage {
        frame_type: VideoFrameType::FirstFrame,
        image: image(b"duplicate"),
    }]);
    assert!(
        provider(&server)
            .video("m")
            .do_start(&options)
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn mismatched_task_id_is_rejected() {
    let server = MockServer::start().await;
    status(&server, json!({"id":"other","status":"queued"})).await;
    assert!(
        provider(&server)
            .video("m")
            .do_status(&json!({"id":"task"}), &VideoCallOptions::new("x"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn transport_timeout_is_not_replayed() {
    use aimux_provider_utils::{Fetch, FetchError, FetchRequest, FetchResponse};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct TimeoutFetch(AtomicUsize);
    #[async_trait::async_trait]
    impl Fetch for TimeoutFetch {
        async fn fetch(&self, _request: FetchRequest) -> Result<FetchResponse, FetchError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(FetchError::Timeout)
        }
    }
    let fetch = Arc::new(TimeoutFetch(AtomicUsize::new(0)));
    let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
        name: "gateway".into(),
        base_url: "https://gateway.example/v1".into(),
        fetch: Some(fetch.clone()),
        ..Default::default()
    })
    .unwrap();
    let error = generate_video(&provider.video("m"), VideoCallOptions::new("x"))
        .await
        .unwrap_err();
    assert!(!error.is_retryable());
    assert_eq!(fetch.0.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn task_identifiers_cannot_escape_the_task_path() {
    use aimux_providers::test_support::mock_fetch::{Canned, MockFetch};
    for (id, encoded) in [
        ("../other?x=1", "%2E%2E%2Fother%3Fx%3D1"),
        ("..", "%252E%252E"),
    ] {
        let fetch = MockFetch::new(vec![Canned::json(&json!({"id":id,"status":"queued"}))]);
        let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
            name: "gateway".into(),
            base_url: "https://gateway.example/v1".into(),
            fetch: Some(fetch.transport()),
            ..Default::default()
        })
        .unwrap();
        provider
            .video("m")
            .do_status(&json!({"id":id}), &VideoCallOptions::new("x"))
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].url,
            format!("https://gateway.example/v1/videos/{encoded}")
        );
    }
}

#[tokio::test]
async fn same_origin_content_redirect_keeps_credentials() {
    let server = MockServer::start().await;
    status(&server, json!({"id":"task","status":"completed"})).await;
    Mock::given(path("/v1/videos/task/content"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/asset"))
        .mount(&server)
        .await;
    Mock::given(path("/asset"))
        .and(header("authorization", "Bearer fixture-key"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"video".to_vec()))
        .expect(1)
        .mount(&server)
        .await;
    let result = provider(&server)
        .video("m")
        .do_status(&json!({"id":"task"}), &VideoCallOptions::new("x"))
        .await
        .unwrap();
    assert!(matches!(result, VideoOperationStatus::Completed(_)));
}
