//! Recraft image provider tests.
//!
//! Each test spins up a `wiremock` mock server, configures a JSON response,
//! creates a `RecraftImageModel` pointing at the mock, calls `do_generate`,
//! and asserts on the request body / headers / result.
//!
//! Tests do not hit the public network and do not read real credentials.

use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::Provider;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
use aimux_core::shared::Size;
use aimux_providers::{RecraftConfig, RecraftProvider};

const PROMPT: &str = "A cute baby sea otter";
const API_KEY: &str = "test-recraft-key";
const MODEL: &str = "recraftv3";

/// Standard generation fixture: one base64 image.
fn b64_response_body() -> Value {
    json!({
        "data": [
            { "b64_json": "iVBORw0KGgoAAAANSUhEUgAAEgAAAA==fake-recraft-image" }
        ]
    })
}

/// Mount a mock JSON response at `/images/generations`.
async fn mock_generations_response(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

fn make_model(server: &MockServer) -> impl ImageModel {
    let config = RecraftConfig::new(API_KEY).with_base_url(server.uri());
    RecraftProvider::new(config).image(MODEL)
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn should_extract_generated_image() {
    let server = MockServer::start().await;
    mock_generations_response(&server, b64_response_body()).await;

    let model = make_model(&server);
    let result = model.do_generate(&options(PROMPT)).await.unwrap();

    match result.images {
        ImageOutputs::Base64(imgs) => {
            assert_eq!(imgs.len(), 1);
            assert_eq!(
                imgs[0],
                "iVBORw0KGgoAAAANSUhEUgAAEgAAAA==fake-recraft-image"
            );
        }
        _ => panic!("expected Base64 outputs"),
    }
    assert_eq!(result.response.model_id.as_deref(), Some(MODEL));
    assert!(result.response.timestamp.is_some());
}

#[tokio::test]
async fn should_post_to_images_generations_endpoint() {
    let server = MockServer::start().await;
    mock_generations_response(&server, b64_response_body()).await;

    let model = make_model(&server);
    model.do_generate(&options(PROMPT)).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].url.path(), "/images/generations");
}

#[tokio::test]
async fn should_send_bearer_auth_header() {
    let server = MockServer::start().await;
    mock_generations_response(&server, b64_response_body()).await;

    let model = make_model(&server);
    model.do_generate(&options(PROMPT)).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let auth = requests[0]
        .headers
        .get("authorization")
        .expect("authorization header present")
        .to_str()
        .unwrap();
    assert_eq!(auth, format!("Bearer {API_KEY}"));
}

#[tokio::test]
async fn should_pass_model_prompt_n_and_default_response_format() {
    let server = MockServer::start().await;
    mock_generations_response(&server, b64_response_body()).await;

    let model = make_model(&server);
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));
    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], MODEL);
    assert_eq!(body["prompt"], PROMPT);
    assert_eq!(body["n"], 1);
    // size is WxH format
    assert_eq!(body["size"], "1024x1024");
    // defaults to b64_json
    assert_eq!(body["response_format"], "b64_json");
}

#[tokio::test]
async fn should_forward_recraft_extension_fields() {
    let server = MockServer::start().await;
    mock_generations_response(&server, b64_response_body()).await;

    let model = make_model(&server);
    let mut opts = options(PROMPT);
    opts.provider_options.insert(
        "recraft".to_string(),
        json!({
            "style": "digital_illustration",
            "styleId": "50c0b14e-3e4f-4a18-9d8e-2b1f0a1c2d3e",
            "negativePrompt": "blurry, low quality",
            "randomSeed": 12345,
            "responseFormat": "b64_json"
        }),
    );
    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["style"], "digital_illustration");
    assert_eq!(body["style_id"], "50c0b14e-3e4f-4a18-9d8e-2b1f0a1c2d3e");
    assert_eq!(body["negative_prompt"], "blurry, low quality");
    assert_eq!(body["random_seed"], 12345);
    assert_eq!(body["response_format"], "b64_json");
}

#[tokio::test]
async fn should_map_options_seed_to_random_seed() {
    let server = MockServer::start().await;
    mock_generations_response(&server, b64_response_body()).await;

    let model = make_model(&server);
    let mut opts = options(PROMPT);
    opts.seed = Some(42);
    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["random_seed"], 42);
}

#[tokio::test]
async fn should_warn_on_unsupported_aspect_ratio() {
    let server = MockServer::start().await;
    mock_generations_response(&server, b64_response_body()).await;

    let model = make_model(&server);
    let mut opts = options(PROMPT);
    opts.aspect_ratio = Some("1:1".parse().unwrap());
    let result = model.do_generate(&opts).await.unwrap();

    assert!(
        result
            .warnings
            .iter()
            .any(|w| matches!(w, aimux_core::shared::Warning::Unsupported { feature, .. } if feature == "aspectRatio")),
        "expected an aspectRatio unsupported warning, got {:?}",
        result.warnings
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Error mapping
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn status_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "message": "Invalid API key", "type": "invalid_request_error" }
        })))
        .mount(&server)
        .await;

    let model = make_model(&server);
    let result = model.do_generate(&options(PROMPT)).await;
    assert!(
        matches!(result, Err(AiMuxError::Auth(ref m)) if m == "Invalid API key"),
        "expected Auth error, got {result:?}"
    );
}

#[tokio::test]
async fn status_500_maps_to_provider_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": { "message": "internal error", "type": "server_error" }
        })))
        .mount(&server)
        .await;

    let model = make_model(&server);
    let result = model.do_generate(&options(PROMPT)).await;
    assert!(
        matches!(result, Err(AiMuxError::Provider(_))),
        "expected Provider error, got {result:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Provider trait
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn provider_name_is_recraft() {
    let server = MockServer::start().await;
    let config = RecraftConfig::new(API_KEY).with_base_url(server.uri());
    let provider = RecraftProvider::new(config);
    assert_eq!(provider.name(), "recraft");
}

#[test]
fn language_model_returns_unsupported_error() {
    let config = RecraftConfig::new(API_KEY);
    let provider = RecraftProvider::new(config);
    let result = provider.language_model("recraftv3");
    assert!(
        matches!(result, Err(AiMuxError::Unsupported(_))),
        "expected Unsupported error"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Auth header matcher (verifies the header is sent on the wire)
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn should_match_request_with_bearer_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .and(header(
            "authorization",
            format!("Bearer {API_KEY}").as_str(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(b64_response_body()))
        .mount(&server)
        .await;

    let model = make_model(&server);
    // Will panic if the request did not carry the expected Authorization header.
    model.do_generate(&options(PROMPT)).await.unwrap();
}
