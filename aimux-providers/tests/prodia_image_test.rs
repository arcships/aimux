//! Rust translation of the Prodia image model tests.
//! Source: `reference/ai/packages/prodia/src/prodia-image-model.test.ts`

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
use aimux_providers::{ProdiaConfig, ProdiaProvider};

const PROMPT: &str = "A cute baby sea otter";

fn multipart_response(boundary: &str, job_json: &str, image_bytes: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"job\"\r\n");
    body.extend_from_slice(b"Content-Type: application/json\r\n\r\n");
    body.extend_from_slice(job_json.as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"output\"\r\n");
    body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body.extend_from_slice(image_bytes);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

async fn mock_prodia(server: &MockServer, body: Vec<u8>) {
    Mock::given(method("POST"))
        .and(path("/job"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "multipart/form-data; boundary=testboundary")
                .set_body_bytes(body),
        )
        .mount(server)
        .await;
}

fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

#[tokio::test]
async fn should_extract_generated_image() {
    let server = MockServer::start().await;
    let job = json!({ "job": { "status": "succeeded" } }).to_string();
    let body = multipart_response("testboundary", &job, b"fake-prodia-image");
    mock_prodia(&server, body).await;
    let config = ProdiaConfig::new("test-key").with_base_url(server.uri());
    let model = ProdiaProvider::new(config).image("sdxl");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    match result.images {
        ImageOutputs::Binary(imgs) => {
            assert_eq!(imgs.len(), 1);
            assert_eq!(imgs[0], b"fake-prodia-image");
        }
        _ => panic!("expected Binary"),
    }
}

#[tokio::test]
async fn should_pass_prompt() {
    let server = MockServer::start().await;
    let job = json!({ "job": { "status": "succeeded" } }).to_string();
    let body = multipart_response("testboundary", &job, b"image");
    mock_prodia(&server, body).await;
    let config = ProdiaConfig::new("test-key").with_base_url(server.uri());
    let model = ProdiaProvider::new(config).image("sdxl");
    model.do_generate(&options(PROMPT)).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let req_body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(req_body["config"]["prompt"], PROMPT);
    assert_eq!(req_body["type"], "sdxl");
}

#[tokio::test]
async fn should_pass_size() {
    let server = MockServer::start().await;
    let job = json!({ "job": { "status": "succeeded" } }).to_string();
    let body = multipart_response("testboundary", &job, b"image");
    mock_prodia(&server, body).await;
    let config = ProdiaConfig::new("test-key").with_base_url(server.uri());
    let model = ProdiaProvider::new(config).image("sdxl");
    let mut opts = options(PROMPT);
    opts.size = Some(aimux_core::shared::Size::new(1024, 1024));
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let req_body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(req_body["config"]["width"], 1024);
    assert_eq!(req_body["config"]["height"], 1024);
}

#[tokio::test]
async fn should_pass_seed() {
    let server = MockServer::start().await;
    let job = json!({ "job": { "status": "succeeded" } }).to_string();
    let body = multipart_response("testboundary", &job, b"image");
    mock_prodia(&server, body).await;
    let config = ProdiaConfig::new("test-key").with_base_url(server.uri());
    let model = ProdiaProvider::new(config).image("sdxl");
    let mut opts = options(PROMPT);
    opts.seed = Some(42);
    model.do_generate(&opts).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let req_body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(req_body["config"]["seed"], 42);
}

#[tokio::test]
async fn should_pass_auth_headers() {
    let server = MockServer::start().await;
    let job = json!({ "job": { "status": "succeeded" } }).to_string();
    let body = multipart_response("testboundary", &job, b"image");
    mock_prodia(&server, body).await;
    let config = ProdiaConfig::new("test-key").with_base_url(server.uri());
    let model = ProdiaProvider::new(config).image("sdxl");
    model.do_generate(&options(PROMPT)).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        reqs[0]
            .headers
            .get("x-prodia-key")
            .unwrap()
            .to_str()
            .unwrap(),
        "test-key"
    );
}

#[tokio::test]
async fn should_return_provider_metadata() {
    let server = MockServer::start().await;
    let job = json!({ "status": "succeeded", "image": "url" }).to_string();
    let body = multipart_response("testboundary", &job, b"image");
    mock_prodia(&server, body).await;
    let config = ProdiaConfig::new("test-key").with_base_url(server.uri());
    let model = ProdiaProvider::new(config).image("sdxl");
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    let meta = result.provider_metadata.unwrap();
    let prodia = meta.get("prodia").unwrap();
    let images = prodia.get("images").unwrap().as_array().unwrap();
    assert_eq!(images.len(), 1);
}
