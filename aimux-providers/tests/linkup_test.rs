//! Linkup search provider tests.
//!
//! Each test spins up a `wiremock` mock server, configures a JSON response,
//! creates a `LinkupSearchModel` pointing at the mock, calls `do_search`, and
//! asserts on the request body / headers / result.
//!
//! Tests do not hit the public network and do not read real credentials.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::Provider;
use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel};
use aimux_providers::{LinkupConfig, LinkupProvider};

const API_KEY: &str = "test-linkup-key";

// -- helpers -----------------------------------------------------------------

/// A realistic Linkup `/v1/search` (outputType=searchResults) success response.
fn search_results_body() -> Value {
    json!({
        "results": [
            {
                "name": "Rust Programming Language",
                "url": "https://www.rust-lang.org",
                "content": "A language empowering everyone to build reliable software."
            },
            {
                "name": "Rust on Wikipedia",
                "url": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
                "content": "Rust is a multi-paradigm, general-purpose programming language."
            }
        ]
    })
}

/// A Linkup sourcedAnswer success response (outputType=sourcedAnswer).
fn sourced_answer_body() -> Value {
    json!({
        "answer": "Rust is a systems programming language.",
        "sources": [
            {
                "name": "Rust Programming Language",
                "url": "https://www.rust-lang.org",
                "content": "A language empowering everyone to build reliable software."
            }
        ]
    })
}

fn provider(server: &MockServer) -> LinkupProvider {
    let config = LinkupConfig::new(API_KEY).with_base_url(server.uri());
    LinkupProvider::new(config)
}

fn opts(query: &str) -> SearchCallOptions {
    SearchCallOptions::new(query)
}

fn linkup_provider_options(depth: &str, output_type: &str) -> HashMap<String, Value> {
    let mut po = HashMap::new();
    po.insert(
        "linkup".to_string(),
        json!({ "depth": depth, "outputType": output_type }),
    );
    po
}

async fn mount_search_mock(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_results_body()))
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
    assert!(result.results[0].score.is_none());
    assert!(result.answer.is_none());
}

#[tokio::test]
async fn do_search_sourced_answer_maps_answer_and_sources() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(sourced_answer_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    let result = model.do_search(&opts("rust language")).await.unwrap();

    assert_eq!(result.results.len(), 1);
    assert_eq!(
        result.results[0].url.as_deref(),
        Some("https://www.rust-lang.org")
    );
    assert_eq!(
        result.answer.as_deref(),
        Some("Rust is a systems programming language.")
    );
}

// -- URL + auth header -------------------------------------------------------

#[tokio::test]
async fn do_search_hits_correct_url_with_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .and(header("authorization", format!("Bearer {API_KEY}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_results_body()))
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
            .get("authorization")
            .and_then(|v| v.to_str().ok()),
        Some("Bearer test-linkup-key")
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

    let mut options = opts("rust language");
    options.include_domains = Some(vec!["rust-lang.org".to_string()]);
    options.exclude_domains = Some(vec!["example.com".to_string()]);
    model.do_search(&options).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["q"], "rust language");
    assert_eq!(body["depth"], "standard");
    assert_eq!(body["outputType"], "searchResults");
    assert_eq!(body["includeDomains"], json!(["rust-lang.org"]));
    assert_eq!(body["excludeDomains"], json!(["example.com"]));
}

#[tokio::test]
async fn do_search_forwards_depth_and_output_type_provider_options() {
    let server = MockServer::start().await;
    mount_search_mock(&server).await;

    let provider = provider(&server);
    let model = provider.search_model();

    let mut options = opts("rust language");
    options.provider_options = Some(linkup_provider_options("deep", "sourcedAnswer"));
    model.do_search(&options).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["depth"], "deep");
    assert_eq!(body["outputType"], "sourcedAnswer");
}

// -- error mapping -----------------------------------------------------------

#[tokio::test]
async fn status_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "message": "Invalid API key", "type": "authentication_error" }
        })))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    let result = model.do_search(&opts("rust language")).await;
    assert!(
        matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.status_code == Some(401) && m.message == "Invalid API key"),
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
        matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.to_string().contains("403") && m.to_string().contains("Forbidden")),
        "expected Provider error, got {result:?}"
    );
}

// -- Provider trait ----------------------------------------------------------

#[tokio::test]
async fn provider_name_is_linkup() {
    let config = LinkupConfig::new(API_KEY);
    let provider = LinkupProvider::new(config);
    assert_eq!(provider.name(), "linkup");
}

#[test]
fn language_model_returns_unsupported_error() {
    let config = LinkupConfig::new(API_KEY);
    let provider = LinkupProvider::new(config);
    match provider.language_model("linkup-search") {
        Err(AiMuxError::UnsupportedFunctionality(msg)) => {
            assert!(
                msg.contains("provider 'linkup' does not provide language models"),
                "unexpected message: {msg}"
            );
        }
        _ => panic!("expected Unsupported error, got success or another error variant"),
    }
}

#[test]
fn model_id_is_linkup_search() {
    let config = LinkupConfig::new(API_KEY);
    let provider = LinkupProvider::new(config);
    let model = provider.search_model();
    assert_eq!(model.model_id(), "linkup-search");
    assert_eq!(model.provider(), "linkup");
}
