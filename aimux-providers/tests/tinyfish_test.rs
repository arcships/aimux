//! TinyFish search provider tests.
//!
//! Each test spins up a `wiremock` mock server, configures a JSON response,
//! creates a `TinyfishSearchModel` pointing at the mock, calls `do_search`,
//! and asserts on the request URL / auth header / result / error mapping.
//!
//! Tests do not hit the public network and do not read real credentials.

use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::Provider;
use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel};
use aimux_providers::{TinyfishConfig, TinyfishProvider};

const API_KEY: &str = "test-tinyfish-key";

// -- helpers -----------------------------------------------------------------

/// A realistic TinyFish success response.
///
/// Includes extra fields (`query`, `total_results`, per-result `position` /
/// `site_name`) that the model ignores, to verify unknown-but-legal fields
/// degrade safely.
fn search_response_body() -> Value {
    json!({
        "query": "rust",
        "total_results": 2,
        "results": [
            {
                "position": 1,
                "site_name": "rust-lang.org",
                "title": "Rust Programming Language",
                "snippet": "A language empowering everyone to build reliable software.",
                "url": "https://www.rust-lang.org"
            },
            {
                "position": 2,
                "site_name": "wikipedia.org",
                "title": "Rust (programming language) - Wikipedia",
                "snippet": "Rust is a multi-paradigm, general-purpose programming language.",
                "url": "https://en.wikipedia.org/wiki/Rust_(programming_language)"
            }
        ]
    })
}

fn provider(server: &MockServer) -> TinyfishProvider {
    let config = TinyfishConfig::new(API_KEY).with_base_url(server.uri());
    TinyfishProvider::new(config)
}

fn opts(query: &str, max_results: Option<u32>) -> SearchCallOptions {
    let mut o = SearchCallOptions::new(query);
    o.max_results = max_results;
    o
}

async fn mount_search_mock(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_response_body()))
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

    let result = model.do_search(&opts("rust", Some(10))).await.unwrap();

    assert_eq!(result.results.len(), 2);
    // title preserved; snippet → content; url preserved.
    assert_eq!(
        result.results[0].title.as_deref(),
        Some("Rust Programming Language")
    );
    assert_eq!(
        result.results[0].url.as_deref(),
        Some("https://www.rust-lang.org")
    );
    assert_eq!(
        result.results[0].content.as_deref(),
        Some("A language empowering everyone to build reliable software.")
    );
    assert_eq!(
        result.results[1].title.as_deref(),
        Some("Rust (programming language) - Wikipedia")
    );
    assert_eq!(
        result.results[1].url.as_deref(),
        Some("https://en.wikipedia.org/wiki/Rust_(programming_language)")
    );
    // No AI answer from this provider.
    assert!(result.answer.is_none());
}

#[tokio::test]
async fn do_search_exposes_response_body() {
    let server = MockServer::start().await;
    mount_search_mock(&server).await;

    let provider = provider(&server);
    let model = provider.search_model();

    let result = model.do_search(&opts("rust", Some(10))).await.unwrap();

    let response = result.response.expect("response");
    let body = response.body.expect("body");
    assert_eq!(body["query"], "rust");
    assert_eq!(body["results"][0]["title"], "Rust Programming Language");
    assert_eq!(body["total_results"], 2);
}

// -- URL + query params ------------------------------------------------------

#[tokio::test]
async fn do_search_hits_correct_url_with_query_params() {
    let server = MockServer::start().await;
    mount_search_mock(&server).await;

    let provider = provider(&server);
    let model = provider.search_model();

    model.do_search(&opts("rust", Some(10))).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    // Root path.
    assert_eq!(requests[0].url.path(), "/", "expected root path");
    let query = requests[0].url.query().expect("query string");
    assert!(
        query.contains("query=rust"),
        "expected `query=rust` in {query}"
    );
    assert!(query.contains("count=10"), "expected `count=10` in {query}");
}

// -- auth header --------------------------------------------------------------

#[tokio::test]
async fn do_search_sends_correct_auth_header() {
    let server = MockServer::start().await;
    // The mock only matches GET / with the correct X-API-Key header, so a
    // missing/wrong auth header would fail to match.
    Mock::given(method("GET"))
        .and(path("/"))
        .and(header("x-api-key", API_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_response_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    model.do_search(&opts("rust", Some(10))).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(
        headers.get("x-api-key").and_then(|v| v.to_str().ok()),
        Some(API_KEY)
    );
}

// -- error mapping -----------------------------------------------------------

#[tokio::test]
async fn status_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "message": "Invalid API key.", "type": "authentication_error" }
        })))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    let result = model.do_search(&opts("rust", Some(10))).await;
    assert!(
        matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.status_code == Some(401) && m.message == "Invalid API key."),
        "expected Auth error, got {result:?}"
    );
}

// -- Provider trait ----------------------------------------------------------

#[tokio::test]
async fn provider_name_is_tinyfish() {
    let config = TinyfishConfig::new(API_KEY);
    let provider = TinyfishProvider::new(config);
    assert_eq!(provider.name(), "tinyfish");
}

#[test]
fn language_model_returns_unsupported_error() {
    let config = TinyfishConfig::new(API_KEY);
    let provider = TinyfishProvider::new(config);
    match provider.language_model("tinyfish-search") {
        Err(AiMuxError::UnsupportedFunctionality(msg)) => {
            assert!(
                msg.contains("provider 'tinyfish' does not provide language models"),
                "unexpected message: {msg}"
            );
        }
        _ => panic!("expected Unsupported error, got success or another error variant"),
    }
}
