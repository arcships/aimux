//! Rust translation of the OpenAI image model tests.
//!
//! Source: `reference/ai/packages/openai/src/image/openai-image-model.test.ts`
//!
//! Each test spins up a `wiremock` mock server, configures a JSON response,
//! creates an `OpenAIImageModel` pointing at the mock, calls `do_generate`,
//! and asserts on the request body / headers / result.
//!
//! The TS tests inject a custom `currentDate` via `_internal` for timestamp
//! assertions. The Rust model always uses `Utc::now()`; timestamp tests
//! verify that a timestamp is present and that `model_id` matches, rather
//! than asserting an exact timestamp value.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs,
};
use aimux_core::shared::{AspectRatio, Size};
use aimux_providers::{OpenAIConfig, OpenAIProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

const PROMPT: &str = "A cute baby sea otter";

/// The standard generation fixture response (2 images, first with revised_prompt).
fn image_response_body() -> Value {
    json!({
        "created": 1770935200,
        "data": [
            {
                "b64_json": "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAA3CGNhQlgAADcIanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj",
                "revised_prompt": "A small and adorable baby sea otter. This little creature is covered in a thick and fluffy brown fur, its tiny paws are slightly visible. The otter has bright, curious eyes and it's floating on its back on a calm sea, surrounded by floating seaweed."
            },
            {
                "b64_json": "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAEp2GNhQlgAASnYanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj"
            }
        ]
    })
}

/// The standard edit fixture response (1 image).
fn image_edit_response_body() -> Value {
    json!({
        "created": 1770935251,
        "background": "opaque",
        "data": [
            {
                "b64_json": "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAFEE2NhQlgAAUQTanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj"
            }
        ],
        "output_format": "png",
        "quality": "high",
        "size": "1024x1024"
    })
}

/// Mount a mock JSON response at `/images/generations`.
async fn mock_generations_response(server: &MockServer, body: Value) {
    mock_generations_response_with_headers(server, body, &[]).await;
}

/// Mount a mock JSON response at `/images/generations` with extra headers.
async fn mock_generations_response_with_headers(
    server: &MockServer,
    body: Value,
    headers: &[(&str, &str)],
) {
    let mut template = ResponseTemplate::new(200).set_body_json(body);
    for (k, v) in headers {
        template = template.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .respond_with(template)
        .mount(server)
        .await;
}

/// Mount a mock JSON response at `/images/edits`.
async fn mock_edits_response(server: &MockServer, body: Value) {
    mock_edits_response_with_headers(server, body, &[]).await;
}

/// Mount a mock JSON response at `/images/edits` with extra headers.
async fn mock_edits_response_with_headers(
    server: &MockServer,
    body: Value,
    headers: &[(&str, &str)],
) {
    let mut template = ResponseTemplate::new(200).set_body_json(body);
    for (k, v) in headers {
        template = template.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/images/edits"))
        .respond_with(template)
        .mount(server)
        .await;
}

/// Build options with a prompt and n=1.
fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

/// Build a binary image file.
fn binary_file(media_type: &str, bytes: &[u8]) -> ImageFile {
    ImageFile::File {
        media_type: media_type.to_string(),
        data: ImageFileData::Binary(bytes.to_vec()),
    }
}

/// Build a base64 image file.
fn base64_file(media_type: &str, b64: &str) -> ImageFile {
    ImageFile::File {
        media_type: media_type.to_string(),
        data: ImageFileData::Base64(b64.to_string()),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — generation
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should pass the model and the settings"
#[tokio::test]
async fn should_pass_the_model_and_the_settings() {
    let server = MockServer::start().await;
    mock_generations_response(&server, image_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("dall-e-3");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));
    opts.provider_options
        .insert("openai".to_string(), json!({ "style": "vivid" }));

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "dall-e-3");
    assert_eq!(body["prompt"], PROMPT);
    assert_eq!(body["n"], 1);
    assert_eq!(body["size"], "1024x1024");
    assert_eq!(body["style"], "vivid");
    assert_eq!(body["response_format"], "b64_json");
}

/// TS: "should map provider options to snake_case for /images/generations"
#[tokio::test]
async fn should_map_provider_options_to_snake_case_for_generations() {
    let server = MockServer::start().await;
    mock_generations_response(&server, image_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));
    opts.provider_options.insert(
        "openai".to_string(),
        json!({
            "quality": "high",
            "background": "transparent",
            "moderation": "low",
            "outputFormat": "webp",
            "outputCompression": 80,
            "user": "user-123"
        }),
    );

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "gpt-image-1");
    assert_eq!(body["prompt"], PROMPT);
    assert_eq!(body["n"], 1);
    assert_eq!(body["size"], "1024x1024");
    assert_eq!(body["quality"], "high");
    assert_eq!(body["background"], "transparent");
    assert_eq!(body["moderation"], "low");
    assert_eq!(body["output_format"], "webp");
    assert_eq!(body["output_compression"], 80);
    assert_eq!(body["user"], "user-123");
    // gpt-image-1 should NOT have response_format
    assert!(body.get("response_format").is_none());
}

/// TS: "should pass headers"
#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_generations_response(&server, image_response_body()).await;

    let mut provider_headers = HashMap::new();
    provider_headers.insert(
        "Custom-Provider-Header".to_string(),
        "provider-header-value".to_string(),
    );

    let config = OpenAIConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_org_id("test-organization")
        .with_project("test-project")
        .with_headers(provider_headers);
    let provider = OpenAIProvider::new(config);
    let model = provider.image("dall-e-3");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));
    opts.provider_options
        .insert("openai".to_string(), json!({ "style": "vivid" }));
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
        h.get("authorization").unwrap().to_str().unwrap(),
        "Bearer test-api-key"
    );
    assert_eq!(
        h.get("openai-organization").unwrap().to_str().unwrap(),
        "test-organization"
    );
    assert_eq!(
        h.get("openai-project").unwrap().to_str().unwrap(),
        "test-project"
    );
    assert_eq!(
        h.get("custom-provider-header").unwrap().to_str().unwrap(),
        "provider-header-value"
    );
    assert_eq!(
        h.get("custom-request-header").unwrap().to_str().unwrap(),
        "request-header-value"
    );
}

/// TS: "should extract the generated images"
#[tokio::test]
async fn should_extract_the_generated_images() {
    let server = MockServer::start().await;
    mock_generations_response(&server, image_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("dall-e-3");

    let result = model.do_generate(&options(PROMPT)).await.unwrap();

    match &result.images {
        ImageOutputs::Base64(imgs) => {
            assert_eq!(imgs.len(), 2);
            assert_eq!(
                imgs[0],
                "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAA3CGNhQlgAADcIanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj"
            );
            assert_eq!(
                imgs[1],
                "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAEp2GNhQlgAASnYanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj"
            );
        }
        _ => panic!("expected Base64 images"),
    }
}

/// TS: "should return warnings for unsupported settings"
#[tokio::test]
async fn should_return_warnings_for_unsupported_settings() {
    let server = MockServer::start().await;
    mock_generations_response(&server, image_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("dall-e-3");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));
    opts.aspect_ratio = Some(AspectRatio::new(1, 1));
    opts.seed = Some(123);

    let result = model.do_generate(&opts).await.unwrap();

    assert_eq!(result.warnings.len(), 2);
    // aspectRatio warning
    match &result.warnings[0] {
        aimux_core::types::Warning::Unsupported { feature, details } => {
            assert_eq!(feature, "aspectRatio");
            assert_eq!(
                details.as_deref(),
                Some("This model does not support aspect ratio. Use `size` instead.")
            );
        }
        _ => panic!("expected Unsupported warning for aspectRatio"),
    }
    // seed warning
    match &result.warnings[1] {
        aimux_core::types::Warning::Unsupported { feature, .. } => {
            assert_eq!(feature, "seed");
        }
        _ => panic!("expected Unsupported warning for seed"),
    }
}

/// TS: "should respect maxImagesPerCall setting"
#[tokio::test]
async fn should_respect_max_images_per_call_setting() {
    let config = OpenAIConfig::new("test-api-key");
    let provider = OpenAIProvider::new(config);

    let dall_e_2 = provider.image("dall-e-2");
    assert_eq!(dall_e_2.max_images_per_call(), Some(10));

    let future_gpt = provider.image("gpt-image-99");
    assert_eq!(future_gpt.max_images_per_call(), Some(10));

    let unknown = provider.image("unknown-model");
    assert_eq!(unknown.max_images_per_call(), Some(1));
}

/// TS: "should include response data with timestamp, modelId and headers"
#[tokio::test]
async fn should_include_response_data_with_timestamp_model_id_and_headers() {
    let server = MockServer::start().await;
    mock_generations_response_with_headers(
        &server,
        image_response_body(),
        &[
            ("x-request-id", "test-request-id"),
            ("x-ratelimit-remaining", "123"),
        ],
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("dall-e-3");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));

    let result = model.do_generate(&opts).await.unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id.as_deref(), Some("dall-e-3"));
    let headers = result.response.headers.as_ref().unwrap();
    assert_eq!(
        headers.get("x-request-id").map(|s| s.as_str()),
        Some("test-request-id")
    );
    assert_eq!(
        headers.get("x-ratelimit-remaining").map(|s| s.as_str()),
        Some("123")
    );
}

/// TS: "should use real date when no custom date provider is specified"
#[tokio::test]
async fn should_use_real_date_when_no_custom_date_provider_is_specified() {
    let server = MockServer::start().await;
    mock_generations_response(&server, image_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("dall-e-3");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));

    let before = chrono::Utc::now();
    let result = model.do_generate(&opts).await.unwrap();
    let after = chrono::Utc::now();

    let ts = result.response.timestamp.as_ref().unwrap();
    let parsed = chrono::DateTime::parse_from_rfc3339(ts).unwrap();
    let parsed_utc = parsed.with_timezone(&chrono::Utc);
    assert!(parsed_utc >= before);
    assert!(parsed_utc <= after);
    assert_eq!(result.response.model_id.as_deref(), Some("dall-e-3"));
}

/// TS: "should not include response_format for gpt-image-1"
#[tokio::test]
async fn should_not_include_response_format_for_gpt_image_1() {
    let server = MockServer::start().await;
    mock_generations_response(&server, image_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "gpt-image-1");
    assert_eq!(body["prompt"], PROMPT);
    assert_eq!(body["n"], 1);
    assert_eq!(body["size"], "1024x1024");
    assert!(body.get("response_format").is_none());
}

/// TS: "should not include response_format for gpt-image-2"
#[tokio::test]
async fn should_not_include_response_format_for_gpt_image_2() {
    let server = MockServer::start().await;
    mock_generations_response(&server, image_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-2");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "gpt-image-2");
    assert!(body.get("response_format").is_none());
}

/// TS: "should not include response_format for future gpt-image models"
#[tokio::test]
async fn should_not_include_response_format_for_future_gpt_image_models() {
    let server = MockServer::start().await;
    mock_generations_response(&server, image_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-99");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "gpt-image-99");
    assert!(body.get("response_format").is_none());
}

/// TS: "should not include response_format for chatgpt-image-latest"
#[tokio::test]
async fn should_not_include_response_format_for_chatgpt_image_latest() {
    let server = MockServer::start().await;
    mock_generations_response(&server, image_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("chatgpt-image-latest");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "chatgpt-image-latest");
    assert!(body.get("response_format").is_none());
}

/// TS: "should not include response_format for date-suffixed gpt-image model IDs (Azure deployment names)"
#[tokio::test]
async fn should_not_include_response_format_for_date_suffixed_gpt_image() {
    let server = MockServer::start().await;
    mock_generations_response(&server, image_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1.5-2025-12-16");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "gpt-image-1.5-2025-12-16");
    assert!(body.get("response_format").is_none());
}

/// TS: "should handle null revised_prompt responses"
#[tokio::test]
async fn should_handle_null_revised_prompt_responses() {
    let server = MockServer::start().await;
    mock_generations_response(
        &server,
        json!({
            "created": 1733837122,
            "data": [
                {
                    "revised_prompt": null,
                    "b64_json": "base64-image-1"
                }
            ]
        }),
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));

    let result = model.do_generate(&opts).await.unwrap();

    match &result.images {
        ImageOutputs::Base64(imgs) => {
            assert_eq!(imgs, &["base64-image-1".to_string()]);
        }
        _ => panic!("expected Base64 images"),
    }
    assert!(result.warnings.is_empty());

    let meta = result.provider_metadata.unwrap();
    let openai = meta.get("openai").unwrap();
    let images = openai.get("images").unwrap().as_array().unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].get("created"), Some(&json!(1733837122)));
    // revisedPrompt should be absent (revised_prompt was null)
    assert!(images[0].get("revisedPrompt").is_none());
    // size, quality, background, outputFormat should be absent (not in response)
    assert!(images[0].get("size").is_none());
    assert!(images[0].get("quality").is_none());
    assert!(images[0].get("background").is_none());
    assert!(images[0].get("outputFormat").is_none());
}

/// TS: "should include response_format for dall-e-3"
#[tokio::test]
async fn should_include_response_format_for_dall_e_3() {
    let server = MockServer::start().await;
    mock_generations_response(&server, image_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("dall-e-3");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["response_format"], "b64_json");
}

/// TS: "should return image meta data"
#[tokio::test]
async fn should_return_image_meta_data() {
    let server = MockServer::start().await;
    mock_generations_response(&server, image_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("dall-e-3");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));
    opts.provider_options
        .insert("openai".to_string(), json!({ "style": "vivid" }));

    let result = model.do_generate(&opts).await.unwrap();

    let meta = result.provider_metadata.unwrap();
    let openai = meta.get("openai").unwrap();
    let images = openai.get("images").unwrap().as_array().unwrap();
    assert_eq!(images.len(), 2);
    // First image has revisedPrompt
    assert_eq!(
        images[0].get("revisedPrompt"),
        Some(&json!(
            "A small and adorable baby sea otter. This little creature is covered in a thick and fluffy brown fur, its tiny paws are slightly visible. The otter has bright, curious eyes and it's floating on its back on a calm sea, surrounded by floating seaweed."
        ))
    );
    assert_eq!(images[0].get("created"), Some(&json!(1770935200)));
    // Second image has no revisedPrompt
    assert!(images[1].get("revisedPrompt").is_none());
    assert_eq!(images[1].get("created"), Some(&json!(1770935200)));
}

/// TS: "should map OpenAI usage to usage"
#[tokio::test]
async fn should_map_openai_usage_to_usage() {
    let server = MockServer::start().await;
    mock_generations_response(
        &server,
        json!({
            "created": 1733837122,
            "data": [
                { "b64_json": "base64-image-1" }
            ],
            "usage": {
                "input_tokens": 12,
                "output_tokens": 0,
                "total_tokens": 12,
                "input_tokens_details": {
                    "image_tokens": 7,
                    "text_tokens": 5
                }
            }
        }),
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));

    let result = model.do_generate(&opts).await.unwrap();

    let usage = result.usage.unwrap();
    assert_eq!(usage.input_tokens, Some(12));
    assert_eq!(usage.output_tokens, Some(0));
    assert_eq!(usage.total_tokens, Some(12));

    let meta = result.provider_metadata.unwrap();
    let openai = meta.get("openai").unwrap();
    let images = openai.get("images").unwrap().as_array().unwrap();
    assert_eq!(images[0].get("imageTokens"), Some(&json!(7)));
    assert_eq!(images[0].get("textTokens"), Some(&json!(5)));
}

/// TS: "should distribute input token details evenly across images"
#[tokio::test]
async fn should_distribute_input_token_details_evenly_across_images() {
    let server = MockServer::start().await;
    mock_generations_response(
        &server,
        json!({
            "created": 1733837122,
            "data": [
                { "b64_json": "base64-image-1" },
                { "b64_json": "base64-image-2" },
                { "b64_json": "base64-image-3" }
            ],
            "usage": {
                "input_tokens": 30,
                "output_tokens": 900,
                "total_tokens": 930,
                "input_tokens_details": {
                    "image_tokens": 194,
                    "text_tokens": 28
                }
            }
        }),
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 3;
    opts.size = Some(Size::new(1024, 1024));

    let result = model.do_generate(&opts).await.unwrap();

    let meta = result.provider_metadata.unwrap();
    let openai = meta.get("openai").unwrap();
    let images = openai.get("images").unwrap().as_array().unwrap();
    assert_eq!(images.len(), 3);
    assert_eq!(images[0].get("imageTokens"), Some(&json!(64)));
    assert_eq!(images[0].get("textTokens"), Some(&json!(9)));
    assert_eq!(images[1].get("imageTokens"), Some(&json!(64)));
    assert_eq!(images[1].get("textTokens"), Some(&json!(9)));
    assert_eq!(images[2].get("imageTokens"), Some(&json!(66)));
    assert_eq!(images[2].get("textTokens"), Some(&json!(10)));
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — image editing
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should call /images/edits endpoint when files are provided"
#[tokio::test]
async fn should_call_edits_endpoint_when_files_are_provided() {
    let server = MockServer::start().await;
    mock_edits_response(&server, image_edit_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));
    opts.files = Some(vec![binary_file("image/png", &[137, 80, 78, 71])]);

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/images/edits");
}

/// TS: "should send image as form data with Uint8Array input"
#[tokio::test]
async fn should_send_image_as_form_data_with_uint8array_input() {
    let server = MockServer::start().await;
    mock_edits_response(&server, image_edit_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));
    opts.files = Some(vec![binary_file("image/png", &[137, 80, 78, 71])]);

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body_str = String::from_utf8_lossy(&requests[0].body);
    assert!(
        body_str.contains("name=\"model\""),
        "should contain model field: {body_str}"
    );
    assert!(
        body_str.contains("gpt-image-1"),
        "should contain model value: {body_str}"
    );
    assert!(
        body_str.contains("name=\"prompt\""),
        "should contain prompt field: {body_str}"
    );
    assert!(
        body_str.contains(PROMPT),
        "should contain prompt value: {body_str}"
    );
    assert!(
        body_str.contains("name=\"n\""),
        "should contain n field: {body_str}"
    );
    assert!(
        body_str.contains("name=\"size\""),
        "should contain size field: {body_str}"
    );
    assert!(
        body_str.contains("1024x1024"),
        "should contain size value: {body_str}"
    );
    assert!(
        body_str.contains("name=\"image\""),
        "should contain image field: {body_str}"
    );
}

/// TS: "should send image as form data with base64 string input"
#[tokio::test]
async fn should_send_image_as_form_data_with_base64_string_input() {
    let server = MockServer::start().await;
    mock_edits_response(&server, image_edit_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.files = Some(vec![base64_file("image/png", "iVBORw0KGgo=")]);

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body_str = String::from_utf8_lossy(&requests[0].body);
    assert!(
        body_str.contains("name=\"model\""),
        "should contain model: {body_str}"
    );
    assert!(
        body_str.contains("gpt-image-1"),
        "should contain model value: {body_str}"
    );
    assert!(
        body_str.contains("name=\"image\""),
        "should contain image field: {body_str}"
    );
}

/// TS: "should send multiple images as form data array"
#[tokio::test]
async fn should_send_multiple_images_as_form_data_array() {
    let server = MockServer::start().await;
    mock_edits_response(&server, image_edit_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.files = Some(vec![
        binary_file("image/png", &[137, 80, 78, 71]),
        binary_file("image/jpeg", &[255, 216, 255, 224]),
    ]);

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body_str = String::from_utf8_lossy(&requests[0].body);
    assert!(
        body_str.contains("name=\"image[]\""),
        "should contain image[] field for multiple images: {body_str}"
    );
}

/// TS: "should pass provider options in form data"
#[tokio::test]
async fn should_pass_provider_options_in_form_data() {
    let server = MockServer::start().await;
    mock_edits_response(&server, image_edit_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));
    opts.files = Some(vec![binary_file("image/png", &[137, 80, 78, 71])]);
    opts.provider_options.insert(
        "openai".to_string(),
        json!({
            "quality": "high",
            "background": "transparent"
        }),
    );

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body_str = String::from_utf8_lossy(&requests[0].body);
    assert!(
        body_str.contains("name=\"quality\""),
        "should contain quality field: {body_str}"
    );
    assert!(
        body_str.contains("high"),
        "should contain quality value: {body_str}"
    );
    assert!(
        body_str.contains("name=\"background\""),
        "should contain background field: {body_str}"
    );
    assert!(
        body_str.contains("transparent"),
        "should contain background value: {body_str}"
    );
}

/// TS: "should map provider options to snake_case for /images/edits"
#[tokio::test]
async fn should_map_provider_options_to_snake_case_for_edits() {
    let server = MockServer::start().await;
    mock_edits_response(&server, image_edit_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));
    opts.files = Some(vec![binary_file("image/png", &[137, 80, 78, 71])]);
    opts.provider_options.insert(
        "openai".to_string(),
        json!({
            "inputFidelity": "high",
            "outputFormat": "webp",
            "outputCompression": 80,
            "user": "user-123"
        }),
    );

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body_str = String::from_utf8_lossy(&requests[0].body);
    assert!(
        body_str.contains("name=\"input_fidelity\""),
        "should contain input_fidelity: {body_str}"
    );
    assert!(
        body_str.contains("name=\"output_format\""),
        "should contain output_format: {body_str}"
    );
    assert!(
        body_str.contains("name=\"output_compression\""),
        "should contain output_compression: {body_str}"
    );
    assert!(
        body_str.contains("name=\"user\""),
        "should contain user: {body_str}"
    );
    assert!(
        body_str.contains("user-123"),
        "should contain user value: {body_str}"
    );
}

/// TS: "should extract the edited images from response"
#[tokio::test]
async fn should_extract_the_edited_images_from_response() {
    let server = MockServer::start().await;
    mock_edits_response(&server, image_edit_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.files = Some(vec![binary_file("image/png", &[137, 80, 78, 71])]);

    let result = model.do_generate(&opts).await.unwrap();

    match &result.images {
        ImageOutputs::Base64(imgs) => {
            assert_eq!(imgs.len(), 1);
            assert_eq!(
                imgs[0],
                "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAFEE2NhQlgAAUQTanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj"
            );
        }
        _ => panic!("expected Base64 images"),
    }
}

/// TS: "should include response metadata for edited images"
#[tokio::test]
async fn should_include_response_metadata_for_edited_images() {
    let server = MockServer::start().await;
    mock_edits_response_with_headers(
        &server,
        image_edit_response_body(),
        &[("x-request-id", "edit-request-id")],
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.files = Some(vec![binary_file("image/png", &[137, 80, 78, 71])]);

    let result = model.do_generate(&opts).await.unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id.as_deref(), Some("gpt-image-1"));
    let headers = result.response.headers.as_ref().unwrap();
    assert_eq!(
        headers.get("x-request-id").map(|s| s.as_str()),
        Some("edit-request-id")
    );
}

/// TS: "should return warnings for unsupported settings in edit mode"
#[tokio::test]
async fn should_return_warnings_for_unsupported_settings_in_edit_mode() {
    let server = MockServer::start().await;
    mock_edits_response(&server, image_edit_response_body()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));
    opts.aspect_ratio = Some(AspectRatio::new(16, 9));
    opts.seed = Some(42);
    opts.files = Some(vec![binary_file("image/png", &[137, 80, 78, 71])]);

    let result = model.do_generate(&opts).await.unwrap();

    assert_eq!(result.warnings.len(), 2);
    match &result.warnings[0] {
        aimux_core::types::Warning::Unsupported { feature, .. } => {
            assert_eq!(feature, "aspectRatio");
        }
        _ => panic!("expected Unsupported warning for aspectRatio"),
    }
    match &result.warnings[1] {
        aimux_core::types::Warning::Unsupported { feature, .. } => {
            assert_eq!(feature, "seed");
        }
        _ => panic!("expected Unsupported warning for seed"),
    }
}

/// TS: "should return usage information for edited images"
#[tokio::test]
async fn should_return_usage_information_for_edited_images() {
    let server = MockServer::start().await;
    mock_edits_response(
        &server,
        json!({
            "created": 1733837122,
            "data": [
                { "b64_json": "edited-base64-image-1" }
            ],
            "usage": {
                "input_tokens": 25,
                "output_tokens": 0,
                "total_tokens": 25
            }
        }),
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.image("gpt-image-1");

    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.files = Some(vec![binary_file("image/png", &[137, 80, 78, 71])]);

    let result = model.do_generate(&opts).await.unwrap();

    let usage = result.usage.unwrap();
    assert_eq!(usage.input_tokens, Some(25));
    assert_eq!(usage.output_tokens, Some(0));
    assert_eq!(usage.total_tokens, Some(25));
}
