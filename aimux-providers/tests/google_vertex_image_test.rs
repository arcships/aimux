//! Rust translation of the Google Vertex AI image model tests.
//!
//! Source: `reference/ai/packages/google-vertex/src/google-vertex-image-model.test.ts`

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs,
};
use aimux_core::shared::{AspectRatio, Size};
use aimux_providers::{VertexProvider, VertexProviderConfig};

const PROMPT: &str = "A cute baby sea otter";

fn imagen_response() -> Value {
    json!({ "predictions": [{ "bytesBase64Encoded": "base64-image-1", "mimeType": "image/png" }] })
}

fn gemini_response() -> Value {
    json!({
        "candidates": [{
            "content": { "parts": [{ "inlineData": { "mimeType": "image/png", "data": "base64-generated-image" } }], "role": "model" },
            "finishReason": "STOP"
        }],
        "usageMetadata": { "promptTokenCount": 10, "candidatesTokenCount": 100, "totalTokenCount": 110 }
    })
}

fn config(server: &MockServer) -> VertexProviderConfig {
    VertexProviderConfig::with_api_key("test-api-key").with_base_url(server.uri())
}

fn config_bearer(server: &MockServer) -> VertexProviderConfig {
    VertexProviderConfig::new("test-token", "test-project", "us-central1")
        .with_base_url(server.uri())
}

fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

#[tokio::test]
async fn imagen_should_extract_generated_images() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/imagen-4.0-generate-001:predict"))
        .respond_with(ResponseTemplate::new(200).set_body_json(imagen_response()))
        .mount(&server)
        .await;
    let provider = VertexProvider::new(config(&server));
    let model = provider.image("imagen-4.0-generate-001");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    match result.images {
        ImageOutputs::Base64(i) => assert_eq!(i, ["base64-image-1"]),
        _ => panic!("expected Base64"),
    }
}

#[tokio::test]
async fn imagen_should_send_aspect_ratio() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/imagen-4.0-generate-001:predict"))
        .respond_with(ResponseTemplate::new(200).set_body_json(imagen_response()))
        .mount(&server)
        .await;
    let provider = VertexProvider::new(config(&server));
    let model = provider.image("imagen-4.0-generate-001");
    let mut opts = options("test prompt");
    opts.n = 1;
    opts.aspect_ratio = Some(AspectRatio::new(16, 9));
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["parameters"]["aspectRatio"], "16:9");
    assert_eq!(body["parameters"]["sampleCount"], 1);
}

#[tokio::test]
async fn imagen_should_warn_for_size() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/imagen-4.0-generate-001:predict"))
        .respond_with(ResponseTemplate::new(200).set_body_json(imagen_response()))
        .mount(&server)
        .await;
    let provider = VertexProvider::new(config(&server));
    let model = provider.image("imagen-4.0-generate-001");
    let mut opts = options(PROMPT);
    opts.n = 1;
    opts.size = Some(Size::new(1024, 1024));
    let result = model.do_generate(&opts).await.unwrap();
    assert!(result.warnings.iter().any(|w| w.feature() == "size"));
}

#[tokio::test]
async fn imagen_should_support_editing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/imagen-4.0-generate-001:predict"))
        .respond_with(ResponseTemplate::new(200).set_body_json(imagen_response()))
        .mount(&server)
        .await;
    let provider = VertexProvider::new(config(&server));
    let model = provider.image("imagen-4.0-generate-001");
    let mut opts = options("Edit this");
    opts.n = 1;
    opts.files = Some(vec![ImageFile::File {
        media_type: "image/png".into(),
        data: ImageFileData::Base64("base64-src".into()),
    }]);
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert!(body["instances"][0]["referenceImages"].is_array());
    assert_eq!(
        body["parameters"]["editMode"],
        "EDIT_MODE_INPAINT_INSERTION"
    );
}

#[tokio::test]
async fn gemini_should_extract_image() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.5-flash-image:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gemini_response()))
        .mount(&server)
        .await;
    let provider = VertexProvider::new(config(&server));
    let model = provider.image("gemini-2.5-flash-image");
    let mut opts = options("A sunset");
    opts.n = 1;
    let result = model.do_generate(&opts).await.unwrap();
    match result.images {
        ImageOutputs::Base64(i) => assert_eq!(i, ["base64-generated-image"]),
        _ => panic!("expected Base64"),
    }
}

#[tokio::test]
async fn gemini_should_send_response_modalities() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.5-flash-image:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gemini_response()))
        .mount(&server)
        .await;
    let provider = VertexProvider::new(config(&server));
    let model = provider.image("gemini-2.5-flash-image");
    let mut opts = options("A sunset");
    opts.n = 1;
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(
        body["generationConfig"]["responseModalities"],
        json!(["IMAGE"])
    );
}

#[tokio::test]
async fn gemini_should_pass_aspect_ratio() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.5-flash-image:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gemini_response()))
        .mount(&server)
        .await;
    let provider = VertexProvider::new(config(&server));
    let model = provider.image("gemini-2.5-flash-image");
    let mut opts = options("A sunset");
    opts.n = 1;
    opts.aspect_ratio = Some(AspectRatio::new(16, 9));
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(
        body["generationConfig"]["imageConfig"],
        json!({ "aspectRatio": "16:9" })
    );
}

#[tokio::test]
async fn gemini_should_throw_for_n_gt_1() {
    let server = MockServer::start().await;
    let provider = VertexProvider::new(config(&server));
    let model = provider.image("gemini-2.5-flash-image");
    let mut opts = options("A sunset");
    opts.n = 2;
    assert!(model.do_generate(&opts).await.is_err());
}

#[tokio::test]
async fn gemini_should_throw_for_mask() {
    let server = MockServer::start().await;
    let provider = VertexProvider::new(config(&server));
    let model = provider.image("gemini-2.5-flash-image");
    let mut opts = options("Edit");
    opts.n = 1;
    opts.mask = Some(ImageFile::File {
        media_type: "image/png".into(),
        data: ImageFileData::Base64("base64-mask".into()),
    });
    assert!(model.do_generate(&opts).await.is_err());
}

#[tokio::test]
async fn should_use_bearer_token_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/imagen-4.0-generate-001:predict"))
        .respond_with(ResponseTemplate::new(200).set_body_json(imagen_response()))
        .mount(&server)
        .await;
    let provider = VertexProvider::new(config_bearer(&server));
    let model = provider.image("imagen-4.0-generate-001");
    model.do_generate(&options(PROMPT)).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    assert!(
        reqs[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("Bearer ")
    );
}

// Helper trait for warning feature access
trait WarningFeature {
    fn feature(&self) -> &str;
}
impl WarningFeature for aimux_core::types::Warning {
    fn feature(&self) -> &str {
        match self {
            aimux_core::types::Warning::Unsupported { feature, .. } => feature,
            _ => "",
        }
    }
}
