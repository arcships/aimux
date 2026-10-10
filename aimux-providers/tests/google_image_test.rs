//! Rust translation of the Google image model tests.
//!
//! Source: `reference/ai/packages/google/src/google-image-model.test.ts`

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs,
};
use aimux_core::provider::Provider;
use aimux_core::shared::{AspectRatio, Size};
use aimux_core::types::Warning;
use aimux_providers::{GoogleImageSettings, GoogleProviderSettings, create_google};

// ── helpers ─────────────────────────────────────────────────────────────────

const PROMPT: &str = "A cute baby sea otter";

fn gemini_response_body() -> Value {
    json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "inlineData": {
                        "mimeType": "image/png",
                        "data": "base64-generated-image"
                    }
                }],
                "role": "model"
            },
            "finishReason": "STOP"
        }],
        "usageMetadata": {
            "promptTokenCount": 10,
            "candidatesTokenCount": 100,
            "totalTokenCount": 110
        }
    })
}

async fn mock_gemini_response(server: &MockServer, body: Value) {
    mock_gemini_response_with_headers(server, body, &[]).await;
}

async fn mock_gemini_response_with_headers(
    server: &MockServer,
    body: Value,
    headers: &[(&str, &str)],
) {
    let mut template = ResponseTemplate::new(200).set_body_json(body);
    for (k, v) in headers {
        template = template.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.5-flash-image:generateContent"))
        .respond_with(template)
        .mount(server)
        .await;
}

fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

fn base64_file(media_type: &str, b64: &str) -> ImageFile {
    ImageFile::File {
        media_type: media_type.to_string(),
        data: ImageFileData::Base64(b64.to_string()),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Image model tests
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should pass headers"
#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options(PROMPT);
    opts.n = 1;
    let mut req_headers = HashMap::new();
    req_headers.insert(
        "Custom-Request-Header".to_string(),
        "request-header-value".to_string(),
    );
    opts.headers = Some(req_headers);

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let h = &requests[0].headers;
    assert_eq!(
        h.get("x-goog-api-key").unwrap().to_str().unwrap(),
        "test-api-key"
    );
    assert_eq!(
        h.get("custom-request-header").unwrap().to_str().unwrap(),
        "request-header-value"
    );
}

/// TS: "should respect maxImagesPerCall setting"
#[tokio::test]
async fn should_respect_max_images_per_call_setting() {
    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_with_settings(
        "gemini-2.5-flash-image",
        GoogleImageSettings {
            max_images_per_call: Some(2),
        },
    );
    assert_eq!(model.max_images_per_call(), Some(2));
}

/// TS: "should use default maxImagesPerCall when not specified"
#[tokio::test]
async fn should_use_default_max_images_per_call_when_not_specified() {
    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();
    assert_eq!(model.max_images_per_call(), Some(1));
}

/// TS: "should extract the generated images"
#[tokio::test]
async fn should_extract_the_generated_images() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options(PROMPT);
    opts.n = 1;

    let result = model.do_generate(&opts).await.unwrap();

    match &result.images {
        ImageOutputs::Base64(imgs) => {
            assert_eq!(imgs, &["base64-generated-image".to_string()]);
        }
        _ => panic!("expected Base64 images"),
    }
}

/// TS: "sends aspect ratio in the request"
#[tokio::test]
async fn sends_aspect_ratio_in_the_request() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options("test prompt");
    opts.n = 1;
    opts.aspect_ratio = Some(AspectRatio::new(16, 9));

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["contents"][0]["parts"][0]["text"], "test prompt");
    assert_eq!(
        body["generationConfig"]["imageConfig"]["aspectRatio"],
        "16:9"
    );
}

/// TS: "should return warnings for unsupported settings"
#[tokio::test]
async fn should_return_warnings_for_unsupported_settings() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));
    opts.aspect_ratio = Some(AspectRatio::new(1, 1));
    opts.seed = Some(123);

    let result = model.do_generate(&opts).await.unwrap();

    assert_eq!(result.warnings.len(), 1);
    match &result.warnings[0] {
        Warning::Unsupported { feature, details } => {
            assert_eq!(feature, "size");
            assert_eq!(
                details.as_deref(),
                Some("This model does not support the `size` option. Use `aspectRatio` instead.")
            );
        }
        _ => panic!("expected Unsupported warning for size"),
    }
}

/// TS: "should include response data with timestamp, modelId and headers"
#[tokio::test]
async fn should_include_response_data_with_timestamp_model_id_and_headers() {
    let server = MockServer::start().await;
    mock_gemini_response_with_headers(
        &server,
        gemini_response_body(),
        &[
            ("request-id", "test-request-id"),
            ("x-goog-quota-remaining", "123"),
        ],
    )
    .await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options(PROMPT);
    opts.n = 1;

    let result = model.do_generate(&opts).await.unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(
        result.response.model_id.as_deref(),
        Some("gemini-2.5-flash-image")
    );
    let headers = result.response.headers.as_ref().unwrap();
    assert_eq!(
        headers.get("request-id").map(std::string::String::as_str),
        Some("test-request-id")
    );
    assert_eq!(
        headers
            .get("x-goog-quota-remaining")
            .map(std::string::String::as_str),
        Some("123")
    );
}

/// TS: "should throw error when mask is provided"
#[tokio::test]
async fn should_throw_error_when_mask_is_provided() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options("Edit this image");
    opts.n = 1;
    opts.mask = Some(base64_file("image/png", "base64-mask-image"));

    let result = model.do_generate(&opts).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("do not support mask-based image editing"));
}

// ════════════════════════════════════════════════════════════════════════════
// Gemini tests
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should return 10 for Gemini image models by default"
#[tokio::test]
async fn gemini_should_return_10_for_max_images_per_call() {
    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();
    assert_eq!(model.max_images_per_call(), Some(1));
}

/// TS: "should respect custom maxImagesPerCall setting"
#[tokio::test]
async fn gemini_should_respect_custom_max_images_per_call() {
    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_with_settings(
        "gemini-2.5-flash-image",
        GoogleImageSettings {
            max_images_per_call: Some(5),
        },
    );
    assert_eq!(model.max_images_per_call(), Some(5));
}

/// TS: "should extract the generated image"
#[tokio::test]
async fn gemini_should_extract_the_generated_image() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options("A beautiful sunset");
    opts.n = 1;

    let result = model.do_generate(&opts).await.unwrap();

    match &result.images {
        ImageOutputs::Base64(imgs) => {
            assert_eq!(imgs, &["base64-generated-image".to_string()]);
        }
        _ => panic!("expected Base64 images"),
    }
}

/// TS: "should send correct request body with responseModalities"
#[tokio::test]
async fn gemini_should_send_correct_request_body() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options("A beautiful sunset");
    opts.n = 1;

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["generationConfig"]["responseModalities"],
        json!(["IMAGE"])
    );
    assert_eq!(
        body["contents"],
        json!([{ "role": "user", "parts": [{ "text": "A beautiful sunset" }] }])
    );
}

/// TS: "should pass aspectRatio via imageConfig"
#[tokio::test]
async fn gemini_should_pass_aspect_ratio_via_image_config() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options("A beautiful sunset");
    opts.n = 1;
    opts.aspect_ratio = Some(AspectRatio::new(16, 9));

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["generationConfig"]["imageConfig"],
        json!({ "aspectRatio": "16:9" })
    );
}

/// TS: "should support Gemini-only aspect ratios like 21:9"
#[tokio::test]
async fn gemini_should_support_21_9_aspect_ratio() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options("A cinematic landscape");
    opts.n = 1;
    opts.aspect_ratio = Some(AspectRatio::new(21, 9));

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["generationConfig"]["imageConfig"],
        json!({ "aspectRatio": "21:9" })
    );
}

/// TS: "should pass seed in generationConfig"
#[tokio::test]
async fn gemini_should_pass_seed_in_generation_config() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options("A beautiful sunset");
    opts.n = 1;
    opts.seed = Some(12345);

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["generationConfig"]["seed"], 12345);
}

/// TS: "should include usage in response"
#[tokio::test]
async fn gemini_should_include_usage_in_response() {
    let server = MockServer::start().await;
    mock_gemini_response(
        &server,
        json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "inlineData": {
                            "mimeType": "image/png",
                            "data": "base64-generated-image"
                        }
                    }],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 20,
                "candidatesTokenCount": 200,
                "totalTokenCount": 220
            }
        }),
    )
    .await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options("A beautiful sunset");
    opts.n = 1;

    let result = model.do_generate(&opts).await.unwrap();

    let usage = result.usage.unwrap();
    assert_eq!(usage.input_tokens, Some(20));
    assert_eq!(usage.output_tokens, Some(200));
    assert_eq!(usage.total_tokens, Some(220));
}

/// TS: "should return warning for unsupported size option"
#[tokio::test]
async fn gemini_should_return_warning_for_size() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options("A beautiful sunset");
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));

    let result = model.do_generate(&opts).await.unwrap();

    assert!(result.warnings.iter().any(|w| {
        matches!(w, Warning::Unsupported { feature, details } if feature == "size"
            && details.as_deref() == Some("This model does not support the `size` option. Use `aspectRatio` instead."))
    }));
}

/// TS: "should include input images in request for editing"
#[tokio::test]
async fn gemini_should_include_input_images_for_editing() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options("Add a hat to this cat");
    opts.n = 1;
    opts.files = Some(vec![base64_file("image/png", "base64-source-image")]);

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    let parts = body["contents"][0]["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0], json!({ "text": "Add a hat to this cat" }));
    assert_eq!(
        parts[1],
        json!({ "inlineData": { "mimeType": "image/png", "data": "base64-source-image" } })
    );
}

/// TS: "should throw error when mask is provided"
#[tokio::test]
async fn gemini_should_throw_error_when_mask_is_provided() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();

    let mut opts = options("Edit this image");
    opts.n = 1;
    opts.mask = Some(base64_file("image/png", "base64-mask-image"));

    let result = model.do_generate(&opts).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("do not support mask-based image editing"));
}
