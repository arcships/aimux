//! Rust translation of the fal image model tests.
//! Source: `reference/ai/packages/fal/src/fal-image-model.test.ts`

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
use aimux_providers::{FalConfig, FalProvider};

const PROMPT: &str = "A cute baby sea otter";

async fn mock_fal(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/fal-ai/flux/schnell"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/image.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fake-image-bytes"))
        .mount(server)
        .await;
}

fn fal_response(server_uri: &str) -> Value {
    json!({ "images": [{ "url": format!("{}/image.png", server_uri), "width": 1024, "height": 1024 }] })
}

fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

#[tokio::test]
async fn should_extract_generated_images() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let config = FalConfig::new("test-key").with_base_url(server.uri());
    let model = FalProvider::new(config).image("fal-ai/flux/schnell");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    match result.images {
        ImageOutputs::Binary(imgs) => {
            assert_eq!(imgs.len(), 1);
            assert_eq!(imgs[0], b"fake-image-bytes");
        }
        _ => panic!("expected Binary"),
    }
}

#[tokio::test]
async fn should_pass_prompt_and_n() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let config = FalConfig::new("test-key").with_base_url(server.uri());
    let model = FalProvider::new(config).image("fal-ai/flux/schnell");
    let mut opts = options(PROMPT);
    opts.n = 1;
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["prompt"], PROMPT);
    assert_eq!(body["num_images"], 1);
}

#[tokio::test]
async fn should_pass_size() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let config = FalConfig::new("test-key").with_base_url(server.uri());
    let model = FalProvider::new(config).image("fal-ai/flux/schnell");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(aimux_core::shared::Size::new(1024, 1024));
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["image_size"]["width"], 1024);
    assert_eq!(body["image_size"]["height"], 1024);
}

#[tokio::test]
async fn should_pass_aspect_ratio() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let config = FalConfig::new("test-key").with_base_url(server.uri());
    let model = FalProvider::new(config).image("fal-ai/flux/schnell");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.aspect_ratio = Some(aimux_core::shared::AspectRatio::new(16, 9));
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["image_size"], "landscape_16_9");
}

#[tokio::test]
async fn should_pass_seed() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let config = FalConfig::new("test-key").with_base_url(server.uri());
    let model = FalProvider::new(config).image("fal-ai/flux/schnell");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.seed = Some(42);
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["seed"], 42);
}

#[tokio::test]
async fn should_pass_auth_headers() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let config = FalConfig::new("test-key").with_base_url(server.uri());
    let model = FalProvider::new(config).image("fal-ai/flux/schnell");
    model.do_generate(&options(PROMPT)).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        reqs[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        "Key test-key"
    );
}

#[tokio::test]
async fn should_forward_provider_options() {
    let server = MockServer::start().await;
    mock_fal(&server, fal_response(&server.uri())).await;
    let config = FalConfig::new("test-key").with_base_url(server.uri());
    let model = FalProvider::new(config).image("fal-ai/flux/schnell");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.provider_options.insert(
        "fal".into(),
        json!({ "numInferenceSteps": 30, "guidanceScale": 7.5, "outputFormat": "png" }),
    );
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["num_inference_steps"], 30);
    assert_eq!(body["guidance_scale"], 7.5);
    assert_eq!(body["output_format"], "png");
}

#[tokio::test]
async fn should_handle_single_image_response() {
    let server = MockServer::start().await;
    let body = json!({ "image": { "url": format!("{}/image.png", server.uri()) } });
    mock_fal(&server, body).await;
    let config = FalConfig::new("test-key").with_base_url(server.uri());
    let model = FalProvider::new(config).image("fal-ai/flux/schnell");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    match result.images {
        ImageOutputs::Binary(imgs) => assert_eq!(imgs.len(), 1),
        _ => panic!("expected Binary"),
    }
}
