//! SSRF guard wiring tests for the provider download path.
//!
//! Providers must fetch response-body URLs through `send_validated_download`,
//! which rejects private/loopback/link-local targets while same-origin URLs
//! against the configured `base_url` (the trusted origin) stay allowed —
//! mirroring AI SDK's `validateUrl` + `trustedOrigin` semantics.
//!
//! These tests use a wiremock server as the "provider endpoint" and point the
//! response-body URL at literal loopback/private IPs that the guard blocks
//! before any connection is attempted.

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::image_model::{ImageCallOptions, ImageModel};
use aimux_providers::{RecraftConfig, RecraftProvider};

const PROMPT: &str = "A cute baby sea otter";

fn options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

/// Mount a generations response whose `data[0].url` points at `url`.
async fn mock_generations_with_url(server: &MockServer, url: &str) {
    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [ { "url": url } ]
        })))
        .mount(server)
        .await;
}

fn make_model(server: &MockServer) -> impl ImageModel {
    let config = RecraftConfig::new("test-recraft-key").with_base_url(server.uri());
    RecraftProvider::new(config).image("recraftv3")
}

#[tokio::test]
async fn rejects_response_body_url_pointing_at_loopback_metadata() {
    let server = MockServer::start().await;
    // 169.254.169.254 is the classic cloud-metadata SSRF target.
    mock_generations_with_url(&server, "http://169.254.169.254/latest/meta-data").await;

    let model = make_model(&server);
    let err = model
        .do_generate(&options(PROMPT))
        .await
        .expect_err("guard must block metadata IP");
    match err {
        aimux_core::AiMuxError::InvalidArgument(msg) => {
            assert!(msg.contains("non-public"), "unexpected message: {msg}");
        }
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[tokio::test]
async fn allows_same_origin_download_against_configured_base_url() {
    // A self-hosted deployment legitimately returns URLs on its own host.
    // trustedOrigin (the configured base_url) must keep this working.
    let server = MockServer::start().await;
    mock_generations_with_url(&server, &format!("{}/generated/x.png", server.uri())).await;
    Mock::given(method("GET"))
        .and(path("/generated/x.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fake-image-data"))
        .mount(&server)
        .await;

    let model = make_model(&server);
    let result = model.do_generate(&options(PROMPT)).await.unwrap();
    match result.images {
        aimux_core::image_model::ImageOutputs::Binary(imgs) => {
            assert_eq!(imgs.len(), 1);
            assert_eq!(imgs[0], b"fake-image-data");
        }
        other => panic!("expected Binary outputs, got {other:?}"),
    }
}
