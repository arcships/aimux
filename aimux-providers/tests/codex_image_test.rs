//! Codex image models on the ChatGPT subscription channel (#526).
//!
//! `gpt-image-2`: JSON `/images/generations` and JSON `/images/edits`.
//! `gpt-image-2.5-*`: the streamed Responses `image_generation` tool.
//! Option validation, size rules, no replay of failures, 401 → TokenExpired.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageOutputs, generate_image,
};
use aimux_core::provider::Provider;
use aimux_core::shared::Size;
use aimux_provider_utils::{Fetch, FetchError, FetchRequest, FetchResponse, Resolvable};
use aimux_providers::{CodexMode, CodexProvider, CodexProviderSettings, create_codex};

const PNG_B64: &str = "iVBORw0KGgo=";

fn subscription(base_url: impl Into<String>) -> CodexProvider {
    create_codex(CodexProviderSettings {
        mode: CodexMode::ChatGptAccount {
            token: Resolvable::Value("account-token".to_string()),
            account_id: Some("acct_123".to_string()),
        },
        base_url: Some(base_url.into()),
        originator: Some("aimux-test".to_string()),
        ..Default::default()
    })
    .expect("valid settings")
}

fn png_file() -> ImageFile {
    ImageFile::File {
        media_type: "image/png".to_string(),
        data: ImageFileData::Binary(vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    }
}

fn image_api_response() -> Value {
    json!({
        "background": "opaque",
        "created": 1_763_600_000,
        "data": [{ "b64_json": PNG_B64 }],
        "output_format": "png",
        "quality": "high",
        "size": "1024x1024",
        "usage": {
            "input_tokens": 12,
            "output_tokens": 4160,
            "total_tokens": 4172,
            "input_tokens_details": { "image_tokens": 0, "text_tokens": 12 }
        }
    })
}

async fn requests(server: &MockServer) -> Vec<wiremock::Request> {
    server.received_requests().await.expect("requests recorded")
}

fn body_of(request: &wiremock::Request) -> Value {
    serde_json::from_slice(&request.body).expect("JSON body")
}

fn header<'a>(request: &'a wiremock::Request, name: &str) -> Option<&'a str> {
    request.headers.get(name).map(|v| v.to_str().unwrap())
}

fn sse(events: &[Value]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(&format!("data: {event}\n\n"));
    }
    body
}

async fn mount_sse(server: &MockServer, events: &[Value]) {
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse(events)),
        )
        .mount(server)
        .await;
}

fn image_call_done(result: &str) -> Value {
    json!({
        "type": "response.output_item.done",
        "output_index": 0,
        "item": {
            "id": "ig_1",
            "type": "image_generation_call",
            "status": "completed",
            "output_format": "png",
            "size": "1024x1536",
            "revised_prompt": "a red fox",
            "result": result
        }
    })
}

fn completed(output: Value) -> Value {
    json!({
        "type": "response.completed",
        "response": {
            "id": "resp_img_1",
            "created_at": 1_763_600_001,
            "status": "completed",
            "model": "gpt-5.6-luna",
            "output": output
        }
    })
}

// ── gpt-image-2: direct endpoints ───────────────────────────────────────────

#[tokio::test]
async fn gpt_image_2_generate_posts_the_minimal_json_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(image_api_response()))
        .mount(&server)
        .await;

    let model = subscription(server.uri())
        .image_model("gpt-image-2")
        .expect("image model");
    assert_eq!(model.provider(), "codex.image");
    assert_eq!(model.max_images_per_call(), Some(1));

    let result = model
        .do_generate(&ImageCallOptions::new("a red fox"))
        .await
        .expect("generate");

    let ImageOutputs::Base64(images) = &result.images else {
        panic!("expected base64 images");
    };
    assert_eq!(images, &[PNG_B64.to_string()]);
    assert_eq!(
        result.usage.as_ref().and_then(|u| u.total_tokens),
        Some(4172)
    );
    let meta = &result.provider_metadata.as_ref().unwrap()["openai"]["images"][0];
    assert_eq!(meta["size"], "1024x1024");
    assert_eq!(meta["outputFormat"], "png");

    let seen = requests(&server).await;
    assert_eq!(seen.len(), 1);
    assert_eq!(
        body_of(&seen[0]),
        json!({ "model": "gpt-image-2", "prompt": "a red fox" })
    );
    assert_eq!(
        header(&seen[0], "authorization"),
        Some("Bearer account-token")
    );
    assert_eq!(header(&seen[0], "originator"), Some("aimux-test"));
    assert_eq!(header(&seen[0], "chatgpt-account-id"), Some("acct_123"));
    assert!(header(&seen[0], "content-type").is_some_and(|v| v.starts_with("application/json")));
}

#[tokio::test]
async fn gpt_image_2_edit_posts_json_images_not_multipart() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/images/edits"))
        .respond_with(ResponseTemplate::new(200).set_body_json(image_api_response()))
        .mount(&server)
        .await;

    let model = subscription(server.uri())
        .image_model("gpt-image-2")
        .unwrap();
    let mut options = ImageCallOptions::new("make it blue");
    options.files = Some(vec![
        png_file(),
        // A partial media type is sniffed from the bytes.
        ImageFile::File {
            media_type: "image".to_string(),
            data: ImageFileData::Base64("/9j/4AAQSkZJRg==".to_string()),
        },
    ]);
    model.do_generate(&options).await.expect("edit");

    let seen = requests(&server).await;
    assert_eq!(seen.len(), 1);
    assert!(header(&seen[0], "content-type").is_some_and(|v| v.starts_with("application/json")));
    assert_eq!(
        body_of(&seen[0]),
        json!({
            "model": "gpt-image-2",
            "prompt": "make it blue",
            "images": [
                { "image_url": "data:image/png;base64,iVBORw0KGgo=" },
                { "image_url": "data:image/jpeg;base64,/9j/4AAQSkZJRg==" }
            ]
        })
    );
}

#[tokio::test]
async fn gpt_image_2_rejects_unprobed_options_without_sending() {
    let server = MockServer::start().await;
    let model = subscription(server.uri())
        .image_model("gpt-image-2")
        .unwrap();

    let mut sized = ImageCallOptions::new("x");
    sized.size = Some(Size::new(1024, 1024));
    let mut quality = ImageCallOptions::new("x");
    quality.provider_options.insert(
        "openai".into(),
        json!({ "quality": "high" }).as_object().unwrap().clone(),
    );
    let mut format = ImageCallOptions::new("x");
    format.provider_options.insert(
        "codex".into(),
        json!({ "outputFormat": "webp" })
            .as_object()
            .unwrap()
            .clone(),
    );
    let mut masked = ImageCallOptions::new("x");
    masked.files = Some(vec![png_file()]);
    masked.mask = Some(png_file());
    let mut no_files = ImageCallOptions::new("x");
    no_files.files = Some(Vec::new());
    let mut two = ImageCallOptions::new("x");
    two.n = 2;

    for (name, options) in [
        ("size", sized),
        ("quality", quality),
        ("outputFormat", format),
        ("mask", masked),
        ("empty files", no_files),
        ("n = 2", two),
    ] {
        let error = model.do_generate(&options).await.expect_err(name);
        assert!(
            matches!(error, AiMuxError::InvalidArgument { .. }),
            "{name}: {error:?}"
        );
    }
    assert!(requests(&server).await.is_empty());
}

#[tokio::test]
async fn gpt_image_2_server_error_is_not_replayed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_json(json!({ "error": { "message": "upstream failure" } })),
        )
        .mount(&server)
        .await;

    let model = subscription(server.uri())
        .image_model("gpt-image-2")
        .unwrap();
    // `generate_image` retries retryable errors twice by default.
    let error = generate_image(model.as_ref(), ImageCallOptions::new("x"))
        .await
        .expect_err("500");
    let AiMuxError::ApiCall(detail) = &error else {
        panic!("expected ApiCall, got {error:?}");
    };
    assert_eq!(detail.status_code, Some(500));
    assert!(!detail.is_retryable);
    assert_eq!(
        requests(&server).await.len(),
        1,
        "a 500 must not be replayed"
    );
}

/// A transport whose every request fails with a reset connection.
struct ResetFetch(AtomicUsize);

#[async_trait]
impl Fetch for ResetFetch {
    async fn fetch(&self, _request: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let reset = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset");
        Err(FetchError::Io(aimux_core::SourceError::new(&reset)))
    }
}

#[tokio::test]
async fn gpt_image_2_connection_reset_is_not_replayed() {
    let fetch = Arc::new(ResetFetch(AtomicUsize::new(0)));
    let provider = create_codex(CodexProviderSettings {
        mode: CodexMode::ChatGptAccount {
            token: Resolvable::Value("t".to_string()),
            account_id: None,
        },
        fetch: Some(fetch.clone()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gpt-image-2").unwrap();
    let error = generate_image(model.as_ref(), ImageCallOptions::new("x"))
        .await
        .expect_err("reset");
    assert!(!error.is_retryable(), "{error:?}");
    assert_eq!(fetch.0.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn gpt_image_2_401_is_token_expired() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({ "error": { "message": "invalid token" } })),
        )
        .mount(&server)
        .await;

    let model = subscription(server.uri())
        .image_model("gpt-image-2")
        .unwrap();
    let error = model
        .do_generate(&ImageCallOptions::new("x"))
        .await
        .expect_err("401");
    assert!(matches!(error, AiMuxError::TokenExpired(_)), "{error:?}");
}

#[tokio::test]
async fn gpt_image_2_403_is_a_plain_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(json!({ "error": { "message": "no image access" } })),
        )
        .mount(&server)
        .await;

    let model = subscription(server.uri())
        .image_model("gpt-image-2")
        .unwrap();
    let error = model
        .do_generate(&ImageCallOptions::new("x"))
        .await
        .expect_err("403");
    let AiMuxError::ApiCall(detail) = &error else {
        panic!("expected ApiCall, got {error:?}");
    };
    assert_eq!(detail.status_code, Some(403));
    assert_eq!(detail.message, "no image access");
}

// ── gpt-image-2.5: Responses image_generation tool ─────────────────────────

#[tokio::test]
async fn gpt_image_2_5_generate_sends_the_tool_request_and_reads_the_item() {
    let server = MockServer::start().await;
    // The final event's `output` is empty: the image is only in the item.
    mount_sse(
        &server,
        &[
            json!({ "type": "response.created", "response": { "id": "resp_img_1" } }),
            image_call_done(PNG_B64),
            completed(json!([])),
        ],
    )
    .await;

    let model = subscription(server.uri())
        .image_model("gpt-image-2.5-flare")
        .unwrap();
    let mut options = ImageCallOptions::new("a red fox");
    options.size = Some(Size::new(1024, 1536));
    let result = model.do_generate(&options).await.expect("generate");

    let ImageOutputs::Base64(images) = &result.images else {
        panic!("expected base64 images");
    };
    assert_eq!(images, &[PNG_B64.to_string()]);
    let meta = &result.provider_metadata.as_ref().unwrap()["openai"];
    assert_eq!(meta["images"][0]["outputFormat"], "png");
    assert_eq!(meta["images"][0]["revisedPrompt"], "a red fox");
    assert_eq!(meta["responseId"], "resp_img_1");

    let seen = requests(&server).await;
    assert_eq!(seen.len(), 1);
    assert_eq!(header(&seen[0], "accept"), Some("text/event-stream"));
    assert_eq!(
        header(&seen[0], "authorization"),
        Some("Bearer account-token")
    );
    assert_eq!(
        body_of(&seen[0]),
        json!({
            "model": "gpt-5.6-luna",
            "store": false,
            "stream": true,
            "instructions": "Call image_generation with the user's prompt exactly as written, preserving all wording, language, punctuation, and visual constraints.",
            "tool_choice": { "type": "image_generation" },
            "tools": [{
                "type": "image_generation",
                "model": "gpt-image-2.5-flare",
                "action": "generate",
                "size": "1024x1536",
                "output_format": "png"
            }],
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "a red fox" }]
            }]
        })
    );
}

#[tokio::test]
async fn gpt_image_2_5_edit_sends_input_images_and_falls_back_to_the_output() {
    let server = MockServer::start().await;
    // No `output_item.done` with a result: the completed output is used.
    mount_sse(
        &server,
        &[completed(json!([{
            "id": "ig_1",
            "type": "image_generation_call",
            "status": "completed",
            "output_format": "png",
            "result": PNG_B64
        }]))],
    )
    .await;

    let model = subscription(server.uri())
        .image_model("gpt-image-2.5-sunburst-2026-09-08")
        .unwrap();
    let mut options = ImageCallOptions::new("make it blue");
    options.files = Some(vec![png_file()]);
    let result = model.do_generate(&options).await.expect("edit");
    assert!(matches!(&result.images, ImageOutputs::Base64(i) if i == &[PNG_B64.to_string()]));

    let body = body_of(&requests(&server).await[0]);
    assert_eq!(body["tools"][0]["action"], "edit");
    assert_eq!(
        body["tools"][0]["model"],
        "gpt-image-2.5-sunburst-2026-09-08"
    );
    assert!(body["tools"][0].get("size").is_none());
    assert_eq!(
        body["input"][0]["content"],
        json!([
            { "type": "input_text", "text": "make it blue" },
            { "type": "input_image", "image_url": "data:image/png;base64,iVBORw0KGgo=" }
        ])
    );
}

#[tokio::test]
async fn gpt_image_2_5_every_family_id_is_served() {
    let provider = subscription("https://chatgpt.com/backend-api/codex");
    for id in [
        "gpt-image-2",
        "gpt-image-2.5-flare",
        "gpt-image-2.5-flare-2026-09-08",
        "gpt-image-2.5-sunburst",
        "gpt-image-2.5-sunburst-2026-09-08",
    ] {
        let model = provider.image_model(id).expect(id);
        assert_eq!(model.model_id(), id);
        assert_eq!(model.supports_file_inputs(), Some(true));
        assert_eq!(model.supports_mask_inputs(), Some(false));
    }
    for id in ["gpt-image-1", "dall-e-3", "gpt-5.2-codex"] {
        assert!(matches!(
            provider.image_model(id),
            Err(AiMuxError::NoSuchModel { .. })
        ));
    }
}

#[tokio::test]
async fn api_key_mode_serves_no_image_models() {
    let provider = create_codex(CodexProviderSettings {
        mode: CodexMode::ApiKey(Some(Resolvable::Value("sk".to_string()))),
        ..Default::default()
    })
    .unwrap();
    assert!(matches!(
        provider.image_model("gpt-image-2"),
        Err(AiMuxError::NoSuchModel { .. })
    ));
}

#[tokio::test]
async fn gpt_image_2_5_validates_size_before_sending() {
    let server = MockServer::start().await;
    let model = subscription(server.uri())
        .image_model("gpt-image-2.5-flare")
        .unwrap();
    for (w, h) in [
        (1000, 1000),
        (4096, 2048),
        (3840, 1264),
        (800, 800),
        (3840, 2176),
    ] {
        let mut options = ImageCallOptions::new("x");
        options.size = Some(Size::new(w, h));
        let error = model.do_generate(&options).await.expect_err("bad size");
        assert!(
            matches!(error, AiMuxError::InvalidArgument { .. }),
            "{w}x{h}: {error:?}"
        );
    }
    let mut quality = ImageCallOptions::new("x");
    quality.provider_options.insert(
        "openai".into(),
        json!({ "quality": "high" }).as_object().unwrap().clone(),
    );
    assert!(model.do_generate(&quality).await.is_err());
    assert!(requests(&server).await.is_empty());
}

#[tokio::test]
async fn gpt_image_2_5_failure_events_are_errors_and_not_replayed() {
    for (name, events) in [
        (
            "error",
            vec![
                json!({ "type": "error", "error": { "code": "server_error", "message": "boom" } }),
            ],
        ),
        (
            "response.failed",
            vec![json!({
                "type": "response.failed",
                "response": { "status": "failed", "error": { "code": "server_error", "message": "boom" } }
            })],
        ),
        (
            "response.incomplete",
            vec![json!({
                "type": "response.incomplete",
                "response": { "status": "incomplete", "incomplete_details": { "reason": "max_output_tokens" } }
            })],
        ),
        (
            "completed without completed status",
            vec![json!({
                "type": "response.completed",
                "response": { "status": "in_progress", "output": [] }
            })],
        ),
    ] {
        let server = MockServer::start().await;
        mount_sse(&server, &events).await;
        let model = subscription(server.uri())
            .image_model("gpt-image-2.5-flare")
            .unwrap();
        let error = generate_image(model.as_ref(), ImageCallOptions::new("x"))
            .await
            .expect_err(name);
        assert!(matches!(error, AiMuxError::ApiCall(_)), "{name}: {error:?}");
        assert!(!error.is_retryable(), "{name}");
        assert_eq!(requests(&server).await.len(), 1, "{name}: not replayed");
    }
}

#[tokio::test]
async fn malformed_image_payloads_are_rejected_without_replay() {
    for model_id in ["gpt-image-2", "gpt-image-2.5-flare"] {
        for payload in ["", " \n", "not!base64"] {
            let server = MockServer::start().await;
            if model_id == "gpt-image-2" {
                Mock::given(method("POST"))
                    .and(path("/images/generations"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                        "data": [{ "b64_json": payload }]
                    })))
                    .mount(&server)
                    .await;
            } else {
                mount_sse(
                    &server,
                    &[
                        image_call_done(payload),
                        completed(json!([{
                            "type": "image_generation_call",
                            "result": payload
                        }])),
                    ],
                )
                .await;
            }
            let model = subscription(server.uri()).image_model(model_id).unwrap();
            let error = generate_image(model.as_ref(), ImageCallOptions::new("x"))
                .await
                .expect_err("invalid image must not be returned as success");
            assert!(matches!(error, AiMuxError::InvalidResponseData { .. }));
            assert!(!error.is_retryable());
            assert_eq!(requests(&server).await.len(), 1, "{model_id}: not replayed");
        }
    }
}

#[tokio::test]
async fn malformed_sse_frame_is_not_hidden_by_completed_or_replayed() {
    let server = MockServer::start().await;
    let body = format!(
        "data: {{invalid json\n\n{}",
        sse(&[image_call_done(PNG_B64), completed(json!([]))])
    );
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;
    let model = subscription(server.uri())
        .image_model("gpt-image-2.5-flare")
        .unwrap();
    let error = generate_image(model.as_ref(), ImageCallOptions::new("x"))
        .await
        .expect_err("malformed SSE frame");
    assert!(!error.is_retryable());
    assert_eq!(requests(&server).await.len(), 1);
}

#[tokio::test]
async fn gpt_image_2_5_completed_without_an_image_is_invalid_response() {
    let server = MockServer::start().await;
    mount_sse(&server, &[completed(json!([]))]).await;
    let model = subscription(server.uri())
        .image_model("gpt-image-2.5-flare")
        .unwrap();
    let error = model
        .do_generate(&ImageCallOptions::new("x"))
        .await
        .expect_err("no image");
    assert!(
        matches!(error, AiMuxError::InvalidResponseData { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn gpt_image_2_5_stream_without_completed_is_an_error() {
    let server = MockServer::start().await;
    mount_sse(&server, &[image_call_done(PNG_B64)]).await;
    let model = subscription(server.uri())
        .image_model("gpt-image-2.5-flare")
        .unwrap();
    let error = generate_image(model.as_ref(), ImageCallOptions::new("x"))
        .await
        .expect_err("truncated");
    assert!(!error.is_retryable());
    assert_eq!(requests(&server).await.len(), 1);
}

#[tokio::test]
async fn gpt_image_2_5_401_is_token_expired() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({ "error": { "message": "invalid token" } })),
        )
        .mount(&server)
        .await;
    let model = subscription(server.uri())
        .image_model("gpt-image-2.5-flare")
        .unwrap();
    let error = model
        .do_generate(&ImageCallOptions::new("x"))
        .await
        .expect_err("401");
    assert!(matches!(error, AiMuxError::TokenExpired(_)), "{error:?}");
}
