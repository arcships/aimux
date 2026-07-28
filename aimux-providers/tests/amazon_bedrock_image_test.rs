//! Rust translation of the Amazon Bedrock image model tests.
//! Source: `reference/ai/packages/amazon-bedrock/src/amazon-bedrock-image-model.test.ts`

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs,
};
use aimux_providers::{BedrockProvider, BedrockProviderConfig};

const PROMPT: &str = "A cute baby sea otter";

fn bedrock_response() -> Value {
    json!({ "images": ["base64-image-1", "base64-image-2"] })
}

async fn mock_bedrock(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/model/amazon.titan-image-generator-v1/invoke"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn config(server: &MockServer) -> BedrockProviderConfig {
    BedrockProviderConfig::with_bearer_token("test-token", "us-east-1").with_base_url(server.uri())
}

fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

#[tokio::test]
async fn should_extract_generated_images() {
    let server = MockServer::start().await;
    mock_bedrock(&server, bedrock_response()).await;
    let provider = BedrockProvider::new(config(&server));
    let model = provider.image("amazon.titan-image-generator-v1");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    match result.images {
        ImageOutputs::Base64(imgs) => {
            assert_eq!(imgs.len(), 2);
            assert_eq!(imgs[0], "base64-image-1");
        }
        _ => panic!("expected Base64"),
    }
}

#[tokio::test]
async fn should_pass_prompt() {
    let server = MockServer::start().await;
    mock_bedrock(&server, bedrock_response()).await;
    let provider = BedrockProvider::new(config(&server));
    let model = provider.image("amazon.titan-image-generator-v1");
    model.do_generate(&options(PROMPT)).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["taskType"], "TEXT_IMAGE");
    assert_eq!(body["textToImageParams"]["text"], PROMPT);
}

#[tokio::test]
async fn should_pass_size_and_n() {
    let server = MockServer::start().await;
    mock_bedrock(&server, bedrock_response()).await;
    let provider = BedrockProvider::new(config(&server));
    let model = provider.image("amazon.titan-image-generator-v1");
    let mut opts = options(PROMPT);
    opts.n = 2;
    opts.size = Some(aimux_core::shared::Size::new(1024, 1024));
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["imageGenerationConfig"]["width"], 1024);
    assert_eq!(body["imageGenerationConfig"]["height"], 1024);
    assert_eq!(body["imageGenerationConfig"]["numberOfImages"], 2);
}

#[tokio::test]
async fn should_pass_seed() {
    let server = MockServer::start().await;
    mock_bedrock(&server, bedrock_response()).await;
    let provider = BedrockProvider::new(config(&server));
    let model = provider.image("amazon.titan-image-generator-v1");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.seed = Some(42);
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["imageGenerationConfig"]["seed"], 42);
}

#[tokio::test]
async fn should_warn_for_aspect_ratio() {
    let server = MockServer::start().await;
    mock_bedrock(&server, bedrock_response()).await;
    let provider = BedrockProvider::new(config(&server));
    let model = provider.image("amazon.titan-image-generator-v1");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.aspect_ratio = Some(aimux_core::shared::AspectRatio::new(16, 9));
    let result = model.do_generate(&opts).await.unwrap();
    assert!(result.warnings.iter().any(|w| matches!(w, aimux_core::types::Warning::Unsupported { feature, .. } if feature == "aspectRatio")));
}

#[tokio::test]
async fn should_pass_bearer_auth() {
    let server = MockServer::start().await;
    mock_bedrock(&server, bedrock_response()).await;
    let provider = BedrockProvider::new(config(&server));
    let model = provider.image("amazon.titan-image-generator-v1");
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
}

#[tokio::test]
async fn should_support_image_variation() {
    let server = MockServer::start().await;
    mock_bedrock(&server, bedrock_response()).await;
    let provider = BedrockProvider::new(config(&server));
    let model = provider.image("amazon.titan-image-generator-v1");
    let mut opts = options("Variation");
    opts.n = 1;
    opts.files = Some(vec![ImageFile::File {
        media_type: "image/png".into(),
        data: ImageFileData::Base64("base64-src".into()),
    }]);
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["taskType"], "IMAGE_VARIATION");
    assert!(body["imageVariationParams"]["images"].is_array());
}

#[tokio::test]
async fn should_support_inpainting() {
    let server = MockServer::start().await;
    mock_bedrock(&server, bedrock_response()).await;
    let provider = BedrockProvider::new(config(&server));
    let model = provider.image("amazon.titan-image-generator-v1");
    let mut opts = options("Inpaint");
    opts.n = 1;
    opts.files = Some(vec![ImageFile::File {
        media_type: "image/png".into(),
        data: ImageFileData::Base64("base64-src".into()),
    }]);
    opts.mask = Some(ImageFile::File {
        media_type: "image/png".into(),
        data: ImageFileData::Base64("base64-mask".into()),
    });
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["taskType"], "INPAINTING");
    assert!(body["inPaintingParams"]["maskImage"].is_string());
}

#[tokio::test]
async fn should_handle_moderated_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/model/amazon.titan-image-generator-v1/invoke"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "Request Moderated",
            "details": { "Moderation Reasons": ["violence", "adult"] }
        })))
        .mount(&server)
        .await;
    let provider = BedrockProvider::new(config(&server));
    let model = provider.image("amazon.titan-image-generator-v1");
    let result = model.do_generate(&options(PROMPT)).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("moderated"));
}

#[tokio::test]
async fn should_pass_negative_text() {
    let server = MockServer::start().await;
    mock_bedrock(&server, bedrock_response()).await;
    let provider = BedrockProvider::new(config(&server));
    let model = provider.image("amazon.titan-image-generator-v1");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.provider_options.insert(
        "amazonBedrock".into(),
        json!({ "negativeText": "ugly, blurry" }),
    );
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["textToImageParams"]["negativeText"], "ugly, blurry");
}
