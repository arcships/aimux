//! Rust translation of the Replicate image model tests.
//! Source: `reference/ai/packages/replicate/src/replicate-image-model.test.ts`

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
use aimux_providers::{ReplicateConfig, ReplicateProvider};

const PROMPT: &str = "A cute baby sea otter";

async fn mock_replicate(server: &MockServer, output: Value) {
    let body = json!({ "output": output });
    Mock::given(method("POST"))
        .and(path("/models/black-forest-labs/flux-schnell/predictions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/image.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fake-image-data"))
        .mount(server)
        .await;
}

fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

#[tokio::test]
async fn should_extract_generated_images() {
    let server = MockServer::start().await;
    mock_replicate(&server, json!(format!("{}/image.png", server.uri()))).await;
    let config = ReplicateConfig::new("test-token").with_base_url(server.uri());
    let model = ReplicateProvider::new(config).image("black-forest-labs/flux-schnell");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    match result.images {
        ImageOutputs::Binary(imgs) => {
            assert_eq!(imgs.len(), 1);
            assert_eq!(imgs[0], b"fake-image-data");
        }
        _ => panic!("expected Binary"),
    }
}

#[tokio::test]
async fn should_pass_prompt_and_n() {
    let server = MockServer::start().await;
    mock_replicate(&server, json!(format!("{}/image.png", server.uri()))).await;
    let config = ReplicateConfig::new("test-token").with_base_url(server.uri());
    let model = ReplicateProvider::new(config).image("black-forest-labs/flux-schnell");
    let mut opts = options(PROMPT);
    opts.n = 1;
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["input"]["prompt"], PROMPT);
    assert_eq!(body["input"]["num_outputs"], 1);
}

#[tokio::test]
async fn should_pass_aspect_ratio() {
    let server = MockServer::start().await;
    mock_replicate(&server, json!(format!("{}/image.png", server.uri()))).await;
    let config = ReplicateConfig::new("test-token").with_base_url(server.uri());
    let model = ReplicateProvider::new(config).image("black-forest-labs/flux-schnell");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.aspect_ratio = Some(aimux_core::shared::AspectRatio::new(16, 9));
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["input"]["aspect_ratio"], "16:9");
}

#[tokio::test]
async fn should_pass_seed() {
    let server = MockServer::start().await;
    mock_replicate(&server, json!(format!("{}/image.png", server.uri()))).await;
    let config = ReplicateConfig::new("test-token").with_base_url(server.uri());
    let model = ReplicateProvider::new(config).image("black-forest-labs/flux-schnell");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.seed = Some(42);
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["input"]["seed"], 42);
}

#[tokio::test]
async fn should_pass_auth_headers() {
    let server = MockServer::start().await;
    mock_replicate(&server, json!(format!("{}/image.png", server.uri()))).await;
    let config = ReplicateConfig::new("test-token").with_base_url(server.uri());
    let model = ReplicateProvider::new(config).image("black-forest-labs/flux-schnell");
    model.do_generate(&options(PROMPT)).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        reqs[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer test-token"
    );
    assert_eq!(
        reqs[0].headers.get("prefer").unwrap().to_str().unwrap(),
        "wait"
    );
}

#[tokio::test]
async fn should_use_custom_wait_time() {
    let server = MockServer::start().await;
    mock_replicate(&server, json!(format!("{}/image.png", server.uri()))).await;
    let config = ReplicateConfig::new("test-token").with_base_url(server.uri());
    let model = ReplicateProvider::new(config).image("black-forest-labs/flux-schnell");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.provider_options
        .insert("replicate".into(), json!({ "maxWaitTimeInSeconds": 30 }));
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        reqs[0].headers.get("prefer").unwrap().to_str().unwrap(),
        "wait=30"
    );
}

#[tokio::test]
async fn should_handle_array_output() {
    let server = MockServer::start().await;
    let img_url = format!("{}/image.png", server.uri());
    mock_replicate(&server, json!([img_url, img_url])).await;
    let config = ReplicateConfig::new("test-token").with_base_url(server.uri());
    let model = ReplicateProvider::new(config).image("black-forest-labs/flux-schnell");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    match result.images {
        ImageOutputs::Binary(imgs) => assert_eq!(imgs.len(), 2),
        _ => panic!("expected Binary"),
    }
}

#[tokio::test]
async fn should_use_versioned_endpoint() {
    let server = MockServer::start().await;
    mock_replicate(&server, json!(format!("{}/image.png", server.uri()))).await;
    let config = ReplicateConfig::new("test-token").with_base_url(server.uri());
    let model = ReplicateProvider::new(config).image("black-forest-labs/flux-schnell:abc123");
    model.do_generate(&options(PROMPT)).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["version"], "abc123");
}
