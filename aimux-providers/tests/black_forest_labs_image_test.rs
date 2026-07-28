//! Rust translation of the Black Forest Labs image model tests.
//! Source: `reference/ai/packages/black-forest-labs/src/black-forest-labs-image-model.test.ts`

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
use aimux_providers::{BlackForestLabsConfig, BlackForestLabsProvider};

const PROMPT: &str = "A cute baby sea otter";

async fn mock_bfl(server: &MockServer) {
    let server_uri = server.uri();
    let submit = json!({ "id": "test-id", "polling_url": format!("{}/v1/get_result", server_uri), "cost": 0.03, "input_mp": 1.0, "output_mp": 1.0 });
    let poll = json!({ "status": "Ready", "result": { "sample": format!("{}/image.png", server_uri), "seed": 42, "start_time": 100, "end_time": 200, "duration": 100 } });
    Mock::given(method("POST"))
        .and(path("/flux-pro-1.1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(submit))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/get_result"))
        .respond_with(ResponseTemplate::new(200).set_body_json(poll))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/image.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fake-bfl-image"))
        .mount(server)
        .await;
}

fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

#[tokio::test]
async fn should_extract_generated_image() {
    let server = MockServer::start().await;
    mock_bfl(&server).await;
    let config = BlackForestLabsConfig::new("test-key").with_base_url(server.uri());
    let model = BlackForestLabsProvider::new(config).image("flux-pro-1.1");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    match result.images {
        ImageOutputs::Binary(imgs) => {
            assert_eq!(imgs.len(), 1);
            assert_eq!(imgs[0], b"fake-bfl-image");
        }
        _ => panic!("expected Binary"),
    }
}

#[tokio::test]
async fn should_pass_prompt() {
    let server = MockServer::start().await;
    mock_bfl(&server).await;
    let config = BlackForestLabsConfig::new("test-key").with_base_url(server.uri());
    let model = BlackForestLabsProvider::new(config).image("flux-pro-1.1");
    model.do_generate(&options(PROMPT)).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["prompt"], PROMPT);
}

#[tokio::test]
async fn should_pass_aspect_ratio() {
    let server = MockServer::start().await;
    mock_bfl(&server).await;
    let config = BlackForestLabsConfig::new("test-key").with_base_url(server.uri());
    let model = BlackForestLabsProvider::new(config).image("flux-pro-1.1");
    let mut opts = options(PROMPT);
    opts.aspect_ratio = Some(aimux_core::shared::AspectRatio::new(16, 9));
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["aspect_ratio"], "16:9");
}

#[tokio::test]
async fn should_pass_seed() {
    let server = MockServer::start().await;
    mock_bfl(&server).await;
    let config = BlackForestLabsConfig::new("test-key").with_base_url(server.uri());
    let model = BlackForestLabsProvider::new(config).image("flux-pro-1.1");
    let mut opts = options(PROMPT);
    opts.seed = Some(42);
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["seed"], 42);
}

#[tokio::test]
async fn should_pass_auth_headers() {
    let server = MockServer::start().await;
    mock_bfl(&server).await;
    let config = BlackForestLabsConfig::new("test-key").with_base_url(server.uri());
    let model = BlackForestLabsProvider::new(config).image("flux-pro-1.1");
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
async fn should_return_provider_metadata() {
    let server = MockServer::start().await;
    mock_bfl(&server).await;
    let config = BlackForestLabsConfig::new("test-key").with_base_url(server.uri());
    let model = BlackForestLabsProvider::new(config).image("flux-pro-1.1");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    let meta = result.provider_metadata.unwrap();
    let bfl = meta.get("blackForestLabs").unwrap();
    let images = bfl.get("images").unwrap().as_array().unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].get("seed"), Some(&json!(42)));
    assert_eq!(images[0].get("cost"), Some(&json!(0.03)));
}
