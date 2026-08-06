//! Parallel AI search provider tests.
//!
//! Each test spins up a `wiremock` mock server, configures a JSON response,
//! creates a `ParallelAiSearchModel` pointing at the mock, calls `do_search`,
//! and asserts on the request body / headers / result.
//!
//! Tests do not hit the public network and do not read real credentials.

use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::Provider;
use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel};
use aimux_providers::{ParallelAiConfig, ParallelAiProvider};

const API_KEY: &str = "test-parallel-key";

// -- helpers -----------------------------------------------------------------

/// A realistic Parallel AI `/v1/search` success response.
fn search_body() -> Value {
    json!({
        "results": [
            {
                "url": "https://www.rust-lang.org",
                "title": "Rust Programming Language",
                "excerpts": ["A language empowering everyone to build reliable software."]
            },
            {
                "url": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
                "title": "Rust on Wikipedia",
                "excerpts": ["Rust is a multi-paradigm language.", "It focuses on safety."]
            }
        ]
    })
}

fn provider(server: &MockServer) -> ParallelAiProvider {
    let config = ParallelAiConfig::new(API_KEY).with_base_url(server.uri());
    ParallelAiProvider::new(config)
}

fn opts(query: &str) -> SearchCallOptions {
    SearchCallOptions::new(query)
}

async fn mount_search_mock(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body()))
        .mount(server)
        .await;
}

// -- result mapping ----------------------------------------------------------

#[tokio::test]
async fn do_search_returns_correct_results() {
    let server = MockServer::start().await;
    mount_search_mock(&server).await;

    let provider = provider(&server);
    let model = provider.search_model();

    let result = model.do_search(&opts("rust language")).await.unwrap();

    assert_eq!(result.results.len(), 2);
    assert_eq!(
        result.results[0].title.as_deref(),
        Some("Rust Programming Language")
    );
    assert_eq!(
        result.results[0].url.as_deref(),
        Some("https://www.rust-lang.org")
    );
    assert!(
        result.results[0]
            .content
            .as_deref()
            .unwrap()
            .contains("reliable software")
    );
    // Excerpts are joined with newlines.
    assert_eq!(
        result.results[1].content.as_deref(),
        Some("Rust is a multi-paradigm language.\nIt focuses on safety.")
    );
    assert!(result.results[0].score.is_none());
    assert!(result.answer.is_none());
}

// -- URL + auth header -------------------------------------------------------

#[tokio::test]
async fn do_search_hits_correct_url_with_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .and(header("x-api-key", API_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    model.do_search(&opts("rust language")).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/v1/search");
    assert_eq!(
        requests[0]
            .headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok()),
        Some("test-parallel-key")
    );
    assert_eq!(
        requests[0]
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
}

// -- request body ------------------------------------------------------------

#[tokio::test]
async fn do_search_sends_correct_request_body() {
    let server = MockServer::start().await;
    mount_search_mock(&server).await;

    let provider = provider(&server);
    let model = provider.search_model();

    model.do_search(&opts("rust language")).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["objective"], "rust language");
    assert_eq!(body["search_queries"], json!(["rust language"]));
    assert_eq!(body["mode"], "advanced");
}

// -- error mapping -----------------------------------------------------------

#[tokio::test]
async fn status_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "message": "Unauthorized", "type": "auth_error" }
        })))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    let result = model.do_search(&opts("rust language")).await;
    assert!(
        matches!(result, Err(AiMuxError::Auth(ref m)) if m == "Unauthorized"),
        "expected Auth error, got {result:?}"
    );
}

#[tokio::test]
async fn status_403_maps_to_provider_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "error": { "message": "Forbidden", "type": "forbidden" }
        })))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    let result = model.do_search(&opts("rust language")).await;
    assert!(
        matches!(result, Err(AiMuxError::Provider(ref m)) if m.contains("403") && m.contains("Forbidden")),
        "expected Provider error, got {result:?}"
    );
}

// -- Provider trait ----------------------------------------------------------

#[tokio::test]
async fn provider_name_is_parallel_ai() {
    let config = ParallelAiConfig::new(API_KEY);
    let provider = ParallelAiProvider::new(config);
    assert_eq!(provider.name(), "parallel_ai");
}

#[test]
fn language_model_returns_unsupported_error() {
    let config = ParallelAiConfig::new(API_KEY);
    let provider = ParallelAiProvider::new(config);
    match provider.language_model("parallel-search") {
        Err(AiMuxError::Unsupported(msg)) => {
            assert!(
                msg.contains("provider 'parallel_ai' does not provide language models"),
                "unexpected message: {msg}"
            );
        }
        _ => panic!("expected Unsupported error, got success or another error variant"),
    }
}

#[test]
fn model_id_is_parallel_search() {
    let config = ParallelAiConfig::new(API_KEY);
    let provider = ParallelAiProvider::new(config);
    let model = provider.search_model();
    assert_eq!(model.model_id(), "parallel-search");
    assert_eq!(model.provider(), "parallel_ai");
}
