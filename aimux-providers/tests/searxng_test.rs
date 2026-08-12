//! SearXNG search provider tests.
//!
//! Each test spins up a `wiremock` mock server, configures a JSON response,
//! creates a `SearxngSearchModel` pointing at the mock, calls `do_search`, and
//! asserts on the request URL / query / result.
//!
//! Tests do not hit the public network and do not read real credentials.
//! SearXNG is unauthenticated, so no API key is involved.

use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::Provider;
use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel};
use aimux_providers::{SearxngConfig, SearxngProvider};

// -- helpers -----------------------------------------------------------------

/// A realistic SearXNG `/search` success response.
fn search_body() -> Value {
    json!({
        "results": [
            {
                "url": "https://www.rust-lang.org",
                "title": "Rust Programming Language",
                "content": "A language empowering everyone to build reliable software.",
                "engine": "google",
                "score": 1.5
            },
            {
                "url": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
                "title": "Rust on Wikipedia",
                "content": "Rust is a multi-paradigm language.",
                "engine": "duckduckgo",
                "score": 0.9
            }
        ]
    })
}

fn provider(server: &MockServer) -> SearxngProvider {
    let config = SearxngConfig::new(server.uri());
    SearxngProvider::new(config)
}

fn opts(query: &str) -> SearchCallOptions {
    SearchCallOptions::new(query)
}

async fn mount_search_mock(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("format", "json"))
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
    assert!((result.results[0].score.unwrap() - 1.5).abs() < 1e-9);
    assert!((result.results[1].score.unwrap() - 0.9).abs() < 1e-9);
    assert!(result.answer.is_none());
}

// -- URL + query params ------------------------------------------------------

#[tokio::test]
async fn do_search_hits_correct_url_with_query_params() {
    let server = MockServer::start().await;
    // The mock matches GET /search with format=json and q=rust language, so a
    // wrong URL or missing param would fail to match.
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("q", "rust language"))
        .and(query_param("format", "json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    model.do_search(&opts("rust language")).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/search");
    let query = requests[0].url.query().unwrap_or("");
    assert!(
        query.contains("q=rust+language") || query.contains("q=rust%20language"),
        "expected q param in query {query}"
    );
    assert!(
        query.contains("format=json"),
        "expected format=json in {query}"
    );

    // SearXNG is unauthenticated — no Authorization header.
    assert!(
        requests[0].headers.get("authorization").is_none(),
        "searxng must not send an Authorization header"
    );
}

// -- error mapping -----------------------------------------------------------

#[tokio::test]
async fn status_403_maps_to_provider_error() {
    let server = MockServer::start().await;
    // 403 typically means the json output format is not enabled.
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(403).set_body_string("format not enabled"))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    let result = model.do_search(&opts("rust language")).await;
    assert!(
        matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.to_string().contains("403")),
        "expected Provider error, got {result:?}"
    );
}

#[tokio::test]
async fn status_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    let result = model.do_search(&opts("rust language")).await;
    assert!(
        matches!(result, Err(ref e) if e.status_code() == Some(401)),
        "expected Auth error, got {result:?}"
    );
}

// -- Provider trait ----------------------------------------------------------

#[tokio::test]
async fn provider_name_is_searxng() {
    let config = SearxngConfig::new("http://localhost:8080");
    let provider = SearxngProvider::new(config);
    assert_eq!(provider.name(), "searxng");
}

#[test]
fn language_model_returns_unsupported_error() {
    let config = SearxngConfig::new("http://localhost:8080");
    let provider = SearxngProvider::new(config);
    match provider.language_model("searxng-search") {
        Err(AiMuxError::UnsupportedFunctionality(msg)) => {
            assert!(
                msg.contains("provider 'searxng' does not provide language models"),
                "unexpected message: {msg}"
            );
        }
        _ => panic!("expected Unsupported error, got success or another error variant"),
    }
}

#[test]
fn model_id_is_searxng_search() {
    let config = SearxngConfig::new("http://localhost:8080");
    let provider = SearxngProvider::new(config);
    let model = provider.search_model();
    assert_eq!(model.model_id(), "searxng-search");
    assert_eq!(model.provider(), "searxng");
}

// -- from_env ----------------------------------------------------------------

#[test]
fn from_env_requires_searxng_url() {
    // Ensure the variable is unset for this test.
    // SAFETY: this is the only test in this binary that touches `SEARXNG_URL`,
    // so there is no concurrent access from other tests.
    unsafe {
        std::env::remove_var("SEARXNG_URL");
    }
    let result = SearxngConfig::from_env();
    assert!(
        matches!(result, Err(AiMuxError::InvalidArgument(ref m)) if m.contains("SEARXNG_URL")),
        "expected InvalidArgument error, got {result:?}"
    );
}
