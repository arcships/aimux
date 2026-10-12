//! Rust translation of the Google Vertex AI image model tests.
//!
//! Source: `reference/ai/packages/google-vertex/src/google-vertex-image-model.test.ts`

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::image_model::{ImageCallOptions, ImageFile, ImageFileData, ImageOutputs};
use aimux_core::provider::Provider;
use aimux_core::shared::{AspectRatio, Size};
use aimux_provider_utils::Resolvable;
use aimux_providers::{VertexProviderSettings, create_google_vertex};

const PROMPT: &str = "A cute baby sea otter";

fn gemini_response() -> Value {
    json!({
        "candidates": [{
            "content": { "parts": [{ "inlineData": { "mimeType": "image/png", "data": "base64-generated-image" } }], "role": "model" },
            "finishReason": "STOP"
        }],
        "usageMetadata": { "promptTokenCount": 10, "candidatesTokenCount": 100, "totalTokenCount": 110 }
    })
}

fn config(server: &MockServer) -> VertexProviderSettings {
    VertexProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri()),
        ..Default::default()
    }
}

fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

#[tokio::test]
async fn imagen_should_extract_generated_images() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.5-flash-image:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gemini_response()))
        .mount(&server)
        .await;
    let provider = create_google_vertex(config(&server)).unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    match result.images {
        ImageOutputs::Base64(i) => assert_eq!(i, ["base64-generated-image"]),
        _ => panic!("expected Base64"),
    }
}

#[tokio::test]
async fn imagen_should_send_aspect_ratio() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.5-flash-image:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gemini_response()))
        .mount(&server)
        .await;
    let provider = create_google_vertex(config(&server)).unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();
    let mut opts = options("test prompt");
    opts.n = 1;
    opts.aspect_ratio = Some(AspectRatio::new(16, 9));
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(
        body["generationConfig"]["imageConfig"]["aspectRatio"],
        "16:9"
    );
}

#[tokio::test]
async fn imagen_should_warn_for_size() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.5-flash-image:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gemini_response()))
        .mount(&server)
        .await;
    let provider = create_google_vertex(config(&server)).unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();
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
        .and(path("/models/gemini-2.5-flash-image:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gemini_response()))
        .mount(&server)
        .await;
    let provider = create_google_vertex(config(&server)).unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();
    let mut opts = options("Edit this");
    opts.n = 1;
    opts.files = Some(vec![ImageFile::File {
        media_type: "image/png".into(),
        data: ImageFileData::Base64("base64-src".into()),
    }]);
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(
        body["contents"][0]["parts"][1],
        json!({ "inlineData": { "mimeType": "image/png", "data": "base64-src" } })
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
    let provider = create_google_vertex(config(&server)).unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();
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
    let provider = create_google_vertex(config(&server)).unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();
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
    let provider = create_google_vertex(config(&server)).unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();
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
async fn gemini_should_throw_for_mask() {
    let server = MockServer::start().await;
    let provider = create_google_vertex(config(&server)).unwrap();
    let model = provider.image_model("gemini-2.5-flash-image").unwrap();
    let mut opts = options("Edit");
    opts.n = 1;
    opts.mask = Some(ImageFile::File {
        media_type: "image/png".into(),
        data: ImageFileData::Base64("base64-mask".into()),
    });
    assert!(model.do_generate(&opts).await.is_err());
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
