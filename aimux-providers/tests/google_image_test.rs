//! Rust translation of the Google image model tests.
//!
//! Source: `reference/ai/packages/google/src/google-image-model.test.ts`
//!
//! Tests both Imagen (non-gemini) and Gemini image model paths.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs,
};
use aimux_core::shared::{AspectRatio, Size};
use aimux_core::types::Warning;
use aimux_providers::{GoogleConfig, GoogleImageSettings, GoogleProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

const PROMPT: &str = "A cute baby sea otter";

fn imagen_response_body() -> Value {
    json!({
        "predictions": [
            { "bytesBase64Encoded": "base64-image-1" },
            { "bytesBase64Encoded": "base64-image-2" },
        ]
    })
}

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

async fn mock_imagen_response(server: &MockServer, body: Value) {
    mock_imagen_response_with_headers(server, body, &[]).await;
}

async fn mock_imagen_response_with_headers(
    server: &MockServer,
    body: Value,
    headers: &[(&str, &str)],
) {
    let mut template = ResponseTemplate::new(200).set_body_json(body);
    for (k, v) in headers {
        template = template.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/models/imagen-3.0-generate-002:predict"))
        .respond_with(template)
        .mount(server)
        .await;
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
// Imagen tests
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should pass headers"
#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_imagen_response(&server, imagen_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("imagen-3.0-generate-002");

    let mut opts = options(PROMPT);
    opts.n = 2;
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
    let config = GoogleConfig::new("test-api-key");
    let provider = GoogleProvider::new(config);
    let model = provider.image_with_settings(
        "imagen-3.0-generate-002",
        GoogleImageSettings {
            max_images_per_call: Some(2),
        },
    );
    assert_eq!(model.max_images_per_call(), Some(2));
}

/// TS: "should use default maxImagesPerCall when not specified"
#[tokio::test]
async fn should_use_default_max_images_per_call_when_not_specified() {
    let config = GoogleConfig::new("test-api-key");
    let provider = GoogleProvider::new(config);
    let model = provider.image("imagen-3.0-generate-002");
    assert_eq!(model.max_images_per_call(), Some(4));
}

/// TS: "should extract the generated images"
#[tokio::test]
async fn should_extract_the_generated_images() {
    let server = MockServer::start().await;
    mock_imagen_response(&server, imagen_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("imagen-3.0-generate-002");

    let mut opts = options(PROMPT);
    opts.n = 2;

    let result = model.do_generate(&opts).await.unwrap();

    match &result.images {
        ImageOutputs::Base64(imgs) => {
            assert_eq!(
                imgs,
                &["base64-image-1".to_string(), "base64-image-2".to_string()]
            );
        }
        _ => panic!("expected Base64 images"),
    }
}

/// TS: "sends aspect ratio in the request"
#[tokio::test]
async fn sends_aspect_ratio_in_the_request() {
    let server = MockServer::start().await;
    mock_imagen_response(&server, imagen_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("imagen-3.0-generate-002");

    let mut opts = options("test prompt");
    opts.n = 1;
    opts.aspect_ratio = Some(AspectRatio::new(16, 9));

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["instances"][0]["prompt"], "test prompt");
    assert_eq!(body["parameters"]["sampleCount"], 1);
    assert_eq!(body["parameters"]["aspectRatio"], "16:9");
}

/// TS: "should combine aspectRatio and provider options"
#[tokio::test]
async fn should_combine_aspect_ratio_and_provider_options() {
    let server = MockServer::start().await;
    mock_imagen_response(&server, imagen_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("imagen-3.0-generate-002");

    let mut opts = options("test prompt");
    opts.n = 1;
    opts.aspect_ratio = Some(AspectRatio::new(1, 1));
    opts.provider_options.insert(
        "google".to_string(),
        json!({ "personGeneration": "dont_allow" }),
    );

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["parameters"]["aspectRatio"], "1:1");
    assert_eq!(body["parameters"]["personGeneration"], "dont_allow");
    assert_eq!(body["parameters"]["sampleCount"], 1);
}

/// TS: "should return warnings for unsupported settings"
#[tokio::test]
async fn should_return_warnings_for_unsupported_settings() {
    let server = MockServer::start().await;
    mock_imagen_response(&server, imagen_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("imagen-3.0-generate-002");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));
    opts.aspect_ratio = Some(AspectRatio::new(1, 1));
    opts.seed = Some(123);

    let result = model.do_generate(&opts).await.unwrap();

    assert_eq!(result.warnings.len(), 2);
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
    match &result.warnings[1] {
        Warning::Unsupported { feature, details } => {
            assert_eq!(feature, "seed");
            assert_eq!(
                details.as_deref(),
                Some("This model does not support the `seed` option through this provider.")
            );
        }
        _ => panic!("expected Unsupported warning for seed"),
    }
}

/// TS: "should include response data with timestamp, modelId and headers"
#[tokio::test]
async fn should_include_response_data_with_timestamp_model_id_and_headers() {
    let server = MockServer::start().await;
    mock_imagen_response_with_headers(
        &server,
        imagen_response_body(),
        &[
            ("request-id", "test-request-id"),
            ("x-goog-quota-remaining", "123"),
        ],
    )
    .await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("imagen-3.0-generate-002");

    let mut opts = options(PROMPT);
    opts.n = 1;

    let result = model.do_generate(&opts).await.unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(
        result.response.model_id.as_deref(),
        Some("imagen-3.0-generate-002")
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

/// TS: "should use real date when no custom date provider is specified"
#[tokio::test]
async fn should_use_real_date_when_no_custom_date_provider_is_specified() {
    let server = MockServer::start().await;
    mock_imagen_response(&server, imagen_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("imagen-3.0-generate-002");

    let mut opts = options(PROMPT);
    opts.n = 2;

    let before = chrono::Utc::now();
    let result = model.do_generate(&opts).await.unwrap();
    let after = chrono::Utc::now();

    let ts = result.response.timestamp.as_ref().unwrap();
    let parsed = chrono::DateTime::parse_from_rfc3339(ts).unwrap();
    let parsed_utc = parsed.with_timezone(&chrono::Utc);
    assert!(parsed_utc >= before);
    assert!(parsed_utc <= after);
    assert_eq!(
        result.response.model_id.as_deref(),
        Some("imagen-3.0-generate-002")
    );
}

/// TS: "should only pass valid provider options"
#[tokio::test]
async fn should_only_pass_valid_provider_options() {
    let server = MockServer::start().await;
    mock_imagen_response(&server, imagen_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("imagen-3.0-generate-002");

    let mut opts = options(PROMPT);
    opts.n = 2;
    opts.aspect_ratio = Some(AspectRatio::new(16, 9));
    opts.provider_options.insert(
        "google".to_string(),
        json!({
            "addWatermark": false,
            "personGeneration": "allow_all",
            "foo": "bar",
            "negativePrompt": "negative prompt"
        }),
    );

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    // Only personGeneration should be passed (other options are not in the schema)
    assert_eq!(body["parameters"]["personGeneration"], "allow_all");
    assert_eq!(body["parameters"]["aspectRatio"], "16:9");
    assert_eq!(body["parameters"]["sampleCount"], 2);
    // Invalid options should not be present
    assert!(body["parameters"].get("addWatermark").is_none());
    assert!(body["parameters"].get("foo").is_none());
    assert!(body["parameters"].get("negativePrompt").is_none());
}

/// TS: "should emit an unsupported warning and not leak into parameters" (googleSearch on Imagen)
#[tokio::test]
async fn should_emit_warning_for_google_search_on_imagen() {
    let server = MockServer::start().await;
    mock_imagen_response(&server, imagen_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("imagen-3.0-generate-002");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.provider_options
        .insert("google".to_string(), json!({ "googleSearch": {} }));

    let result = model.do_generate(&opts).await.unwrap();

    assert!(result.warnings.iter().any(|w| {
        matches!(w, Warning::Unsupported { feature, details } if feature == "googleSearch"
            && details.as_deref() == Some("Google Search grounding is only supported on Gemini image models."))
    }));

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(body["parameters"].get("googleSearch").is_none());
}

/// TS: "should throw error when files are provided"
#[tokio::test]
async fn should_throw_error_when_files_are_provided() {
    let server = MockServer::start().await;
    mock_imagen_response(&server, imagen_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("imagen-3.0-generate-002");

    let mut opts = options("Edit this image");
    opts.n = 1;
    opts.files = Some(vec![base64_file("image/png", "base64-source-image")]);

    let result = model.do_generate(&opts).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("does not support image editing with Imagen models"));
}

/// TS: "should throw error when mask is provided"
#[tokio::test]
async fn should_throw_error_when_mask_is_provided() {
    let server = MockServer::start().await;
    mock_imagen_response(&server, imagen_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("imagen-3.0-generate-002");

    let mut opts = options("Edit this image");
    opts.n = 1;
    opts.mask = Some(base64_file("image/png", "base64-mask-image"));

    let result = model.do_generate(&opts).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("does not support image editing with masks"));
}

// ════════════════════════════════════════════════════════════════════════════
// Gemini tests
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should return 10 for Gemini image models by default"
#[tokio::test]
async fn gemini_should_return_10_for_max_images_per_call() {
    let config = GoogleConfig::new("test-api-key");
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");
    assert_eq!(model.max_images_per_call(), Some(10));
}

/// TS: "should respect custom maxImagesPerCall setting"
#[tokio::test]
async fn gemini_should_respect_custom_max_images_per_call() {
    let config = GoogleConfig::new("test-api-key");
    let provider = GoogleProvider::new(config);
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

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");

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

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");

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

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");

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

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");

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

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");

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

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");

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

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");

    let mut opts = options("A beautiful sunset");
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));

    let result = model.do_generate(&opts).await.unwrap();

    assert!(result.warnings.iter().any(|w| {
        matches!(w, Warning::Unsupported { feature, details } if feature == "size"
            && details.as_deref() == Some("This model does not support the `size` option. Use `aspectRatio` instead."))
    }));
}

/// TS: "should not send a tools field when googleSearch is not set"
#[tokio::test]
async fn gemini_should_not_send_tools_when_no_google_search() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");

    let mut opts = options("A beautiful sunset");
    opts.n = 1;

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(body.get("tools").is_none());
}

/// TS: "should forward providerOptions.google.googleSearch as the google_search tool"
#[tokio::test]
async fn gemini_should_forward_google_search_as_tool() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");

    let mut opts = options("A beautiful sunset");
    opts.n = 1;
    opts.provider_options.insert(
        "google".to_string(),
        json!({ "googleSearch": { "searchTypes": { "imageSearch": {} } } }),
    );

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["tools"],
        json!([{ "googleSearch": { "searchTypes": { "imageSearch": {} } } }])
    );
}

/// TS: "should not leak googleSearch into providerOptions passthrough"
#[tokio::test]
async fn gemini_should_not_leak_google_search_into_passthrough() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");

    let mut opts = options("A beautiful sunset");
    opts.n = 1;
    opts.provider_options
        .insert("google".to_string(), json!({ "googleSearch": {} }));

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(body["generationConfig"].get("googleSearch").is_none());
}

/// TS: "should include input images in request for editing"
#[tokio::test]
async fn gemini_should_include_input_images_for_editing() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");

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

/// TS: "should throw error when n > 1"
#[tokio::test]
async fn gemini_should_throw_error_when_n_gt_1() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");

    let mut opts = options("A beautiful sunset");
    opts.n = 2;

    let result = model.do_generate(&opts).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("do not support generating a set number of images per call"));
}

/// TS: "should throw error when mask is provided"
#[tokio::test]
async fn gemini_should_throw_error_when_mask_is_provided() {
    let server = MockServer::start().await;
    mock_gemini_response(&server, gemini_response_body()).await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");

    let mut opts = options("Edit this image");
    opts.n = 1;
    opts.mask = Some(base64_file("image/png", "base64-mask-image"));

    let result = model.do_generate(&opts).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("do not support mask-based image editing"));
}

/// TS: "should forward groundingMetadata from the language-model response into providerMetadata.google"
#[tokio::test]
async fn gemini_should_forward_grounding_metadata() {
    let server = MockServer::start().await;
    let grounding_metadata = json!({
        "webSearchQueries": ["who performs at the 2026 super bowl halftime show"],
        "groundingChunks": [
            { "web": { "uri": "https://example.com/source", "title": "Example" } }
        ]
    });
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
                "finishReason": "STOP",
                "groundingMetadata": grounding_metadata
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 100,
                "totalTokenCount": 110
            }
        }),
    )
    .await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.image("gemini-2.5-flash-image");

    let mut opts = options("A beautiful sunset");
    opts.n = 1;
    opts.provider_options
        .insert("google".to_string(), json!({ "googleSearch": {} }));

    let result = model.do_generate(&opts).await.unwrap();

    let meta = result.provider_metadata.unwrap();
    let google = meta.get("google").unwrap();
    assert_eq!(google.get("groundingMetadata"), Some(&grounding_metadata));
    // images should still be present
    let images = google.get("images").unwrap().as_array().unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0], json!({}));
}
