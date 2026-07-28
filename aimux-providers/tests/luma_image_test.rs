//! Rust translation of the Luma image model tests.
//! Source: `reference/ai/packages/luma/src/luma-image-model.test.ts`

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::image_model::{ImageCallOptions, ImageFile, ImageModel, ImageOutputs};
use aimux_providers::{LumaConfig, LumaProvider};

const PROMPT: &str = "A beautiful sunset";

async fn mock_luma(server: &MockServer) {
    let img_url = format!("{}/image.png", server.uri());
    Mock::given(method("POST"))
        .and(path("/dream-machine/v1/generations/image"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "id": "gen-123", "state": "queued" })),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/dream-machine/v1/generations/gen-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({ "id": "gen-123", "state": "completed", "assets": { "image": img_url } }),
        ))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/image.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fake-luma-image"))
        .mount(server)
        .await;
}

fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

#[tokio::test]
async fn should_extract_generated_image() {
    let server = MockServer::start().await;
    mock_luma(&server).await;
    let config = LumaConfig::new("test-key").with_base_url(server.uri());
    let model = LumaProvider::new(config).image("ray2");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    match result.images {
        ImageOutputs::Binary(imgs) => {
            assert_eq!(imgs.len(), 1);
            assert_eq!(imgs[0], b"fake-luma-image");
        }
        _ => panic!("expected Binary"),
    }
}

#[tokio::test]
async fn should_pass_prompt_and_model() {
    let server = MockServer::start().await;
    mock_luma(&server).await;
    let config = LumaConfig::new("test-key").with_base_url(server.uri());
    let model = LumaProvider::new(config).image("ray2");
    model.do_generate(&options(PROMPT)).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["prompt"], PROMPT);
    assert_eq!(body["model"], "ray2");
}

#[tokio::test]
async fn should_pass_aspect_ratio() {
    let server = MockServer::start().await;
    mock_luma(&server).await;
    let config = LumaConfig::new("test-key").with_base_url(server.uri());
    let model = LumaProvider::new(config).image("ray2");
    let mut opts = options(PROMPT);
    opts.aspect_ratio = Some(aimux_core::shared::AspectRatio::new(16, 9));
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["aspect_ratio"], "16:9");
}

#[tokio::test]
async fn should_warn_for_seed() {
    let server = MockServer::start().await;
    mock_luma(&server).await;
    let config = LumaConfig::new("test-key").with_base_url(server.uri());
    let model = LumaProvider::new(config).image("ray2");
    let mut opts = options(PROMPT);
    opts.seed = Some(42);
    let result = model.do_generate(&opts).await.unwrap();
    assert!(result.warnings.iter().any(|w| matches!(w, aimux_core::types::Warning::Unsupported { feature, .. } if feature == "seed")));
}

#[tokio::test]
async fn should_warn_for_size() {
    let server = MockServer::start().await;
    mock_luma(&server).await;
    let config = LumaConfig::new("test-key").with_base_url(server.uri());
    let model = LumaProvider::new(config).image("ray2");
    let mut opts = options(PROMPT);
    opts.size = Some(aimux_core::shared::Size::new(1024, 1024));
    let result = model.do_generate(&opts).await.unwrap();
    assert!(result.warnings.iter().any(|w| matches!(w, aimux_core::types::Warning::Unsupported { feature, .. } if feature == "size")));
}

#[tokio::test]
async fn should_pass_auth_headers() {
    let server = MockServer::start().await;
    mock_luma(&server).await;
    let config = LumaConfig::new("test-key").with_base_url(server.uri());
    let model = LumaProvider::new(config).image("ray2");
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
async fn should_support_image_editing_with_urls() {
    let server = MockServer::start().await;
    mock_luma(&server).await;
    let config = LumaConfig::new("test-key").with_base_url(server.uri());
    let model = LumaProvider::new(config).image("ray2");
    let mut opts = options("Edit this");
    opts.n = 1;
    opts.files = Some(vec![ImageFile::Url {
        url: "https://example.com/source.png".into(),
    }]);
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert!(body.get("image").is_some());
    let images = body["image"].as_array().unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0]["url"], "https://example.com/source.png");
}

#[tokio::test]
async fn should_throw_for_mask() {
    let server = MockServer::start().await;
    let config = LumaConfig::new("test-key").with_base_url(server.uri());
    let model = LumaProvider::new(config).image("ray2");
    let mut opts = options("Edit");
    opts.n = 1;
    opts.mask = Some(ImageFile::Url {
        url: "https://example.com/mask.png".into(),
    });
    assert!(model.do_generate(&opts).await.is_err());
}
