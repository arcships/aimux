//! Stability image model tests.
//!
//! Uses wiremock to verify request construction (URL, auth header, multipart
//! body), binary image extraction, model-to-endpoint mapping, and 401 error
//! mapping. No real credentials or network access are required.

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
use aimux_providers::{StabilityConfig, StabilityProvider};

const PROMPT: &str = "A cute baby sea otter";
const API_KEY: &str = "test-key";

fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

/// Mount a mock that returns a binary PNG image for the ultra endpoint.
async fn mock_ultra_image(server: &MockServer, body: Vec<u8>) {
    Mock::given(method("POST"))
        .and(path("/v2beta/stable-image/generate/ultra"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(body),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn should_extract_generated_image() {
    let server = MockServer::start().await;
    mock_ultra_image(&server, b"fake-stability-image".to_vec()).await;

    let config = StabilityConfig::new(API_KEY).with_base_url(server.uri());
    let model = StabilityProvider::new(config).image("stable-image-ultra");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();

    match result.images {
        ImageOutputs::Binary(imgs) => {
            assert_eq!(imgs.len(), 1);
            assert_eq!(imgs[0], b"fake-stability-image");
        }
        _ => panic!("expected Binary image outputs"),
    }
}

#[tokio::test]
async fn should_use_correct_url_for_ultra() {
    let server = MockServer::start().await;
    mock_ultra_image(&server, b"image".to_vec()).await;

    let config = StabilityConfig::new(API_KEY).with_base_url(server.uri());
    let model = StabilityProvider::new(config).image("stable-image-ultra");
    model.do_generate(&options(PROMPT)).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].url.path(), "/v2beta/stable-image/generate/ultra");
}

#[tokio::test]
async fn should_use_correct_url_for_core() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2beta/stable-image/generate/core"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(b"image".to_vec()),
        )
        .mount(&server)
        .await;

    let config = StabilityConfig::new(API_KEY).with_base_url(server.uri());
    let model = StabilityProvider::new(config).image("stable-image-core");
    model.do_generate(&options(PROMPT)).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].url.path(), "/v2beta/stable-image/generate/core");
}

#[tokio::test]
async fn should_use_correct_url_for_sd3() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2beta/stable-image/generate/sd3"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(b"image".to_vec()),
        )
        .mount(&server)
        .await;

    let config = StabilityConfig::new(API_KEY).with_base_url(server.uri());
    let model = StabilityProvider::new(config).image("sd3");
    model.do_generate(&options(PROMPT)).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].url.path(), "/v2beta/stable-image/generate/sd3");
}

#[tokio::test]
async fn should_pass_auth_header() {
    let server = MockServer::start().await;
    mock_ultra_image(&server, b"image".to_vec()).await;

    let config = StabilityConfig::new(API_KEY).with_base_url(server.uri());
    let model = StabilityProvider::new(config).image("stable-image-ultra");
    model.do_generate(&options(PROMPT)).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        reqs[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer test-key"
    );
}

#[tokio::test]
async fn should_pass_prompt_and_output_format_in_multipart_body() {
    let server = MockServer::start().await;
    mock_ultra_image(&server, b"image".to_vec()).await;

    let config = StabilityConfig::new(API_KEY).with_base_url(server.uri());
    let model = StabilityProvider::new(config).image("stable-image-ultra");
    model.do_generate(&options(PROMPT)).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body = String::from_utf8_lossy(&reqs[0].body);
    assert!(
        body.contains("name=\"prompt\""),
        "body should include prompt field"
    );
    assert!(body.contains(PROMPT), "body should include the prompt text");
    assert!(
        body.contains("name=\"output_format\""),
        "body should include output_format field"
    );
    assert!(body.contains("png"), "output_format should default to png");
}

#[tokio::test]
async fn should_pass_seed_and_aspect_ratio() {
    let server = MockServer::start().await;
    mock_ultra_image(&server, b"image".to_vec()).await;

    let config = StabilityConfig::new(API_KEY).with_base_url(server.uri());
    let model = StabilityProvider::new(config).image("stable-image-ultra");
    let mut opts = options(PROMPT);
    opts.seed = Some(42);
    opts.aspect_ratio = Some(aimux_core::shared::AspectRatio::new(16, 9));
    model.do_generate(&opts).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body = String::from_utf8_lossy(&reqs[0].body);
    assert!(
        body.contains("name=\"seed\""),
        "body should include seed field"
    );
    assert!(body.contains("42"), "body should include the seed value");
    assert!(
        body.contains("name=\"aspect_ratio\""),
        "body should include aspect_ratio field"
    );
    assert!(
        body.contains("16:9"),
        "body should include the aspect ratio"
    );
}

#[tokio::test]
async fn should_map_401_to_auth_error() {
    let server = MockServer::start().await;
    let error_body = r#"{"id":"abc123","name":"unauthorized","errors":["invalid api key"]}"#;
    Mock::given(method("POST"))
        .and(path("/v2beta/stable-image/generate/ultra"))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("content-type", "application/json")
                .set_body_bytes(error_body.as_bytes().to_vec()),
        )
        .mount(&server)
        .await;

    let config = StabilityConfig::new(API_KEY).with_base_url(server.uri());
    let model = StabilityProvider::new(config).image("stable-image-ultra");
    let result = model.do_generate(&options(PROMPT)).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        AiMuxError::Auth(msg) => {
            assert_eq!(msg, "unauthorized");
        }
        e => panic!("expected Auth error, got: {e:?}"),
    }
}

#[tokio::test]
async fn should_decode_base64_json_response() {
    let server = MockServer::start().await;
    let png_bytes = b"fake-png-bytes".to_vec();
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_bytes);
    let json_body = format!(r#"{{"image":"{b64}","finish_reason":"SUCCESS","seed":7}}"#);
    Mock::given(method("POST"))
        .and(path("/v2beta/stable-image/generate/ultra"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_bytes(json_body.into_bytes()),
        )
        .mount(&server)
        .await;

    let config = StabilityConfig::new(API_KEY).with_base_url(server.uri());
    // Override Accept to request the JSON/base64 response shape.
    let mut headers = std::collections::HashMap::new();
    headers.insert("Accept".to_string(), "application/json".to_string());
    let config = config.with_headers(headers);
    let model = StabilityProvider::new(config).image("stable-image-ultra");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();

    match result.images {
        ImageOutputs::Binary(imgs) => {
            assert_eq!(imgs.len(), 1);
            assert_eq!(imgs[0], png_bytes);
        }
        _ => panic!("expected Binary image outputs"),
    }
}
