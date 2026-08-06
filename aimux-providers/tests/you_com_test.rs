//! You.com search provider tests.
//!
//! Each test spins up a `wiremock` mock server, configures a JSON response,
//! creates a `YouComSearchModel` pointing at the mock, calls `do_search`, and
//! asserts on the request URL / auth header / result / error mapping.
//!
//! Tests do not hit the public network and do not read real credentials.

use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::Provider;
use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel};
use aimux_providers::{YouComConfig, YouComProvider};

const API_KEY: &str = "test-ydc-key";

// -- helpers -----------------------------------------------------------------

/// A realistic You.com `/v1/search` success response.
///
/// The first result has both `description` and `snippet` (description wins);
/// the second has `description: null` so `snippet` is used as the content
/// fallback.
fn search_response_body() -> Value {
    json!({
        "results": [
            {
                "url": "https://www.rust-lang.org",
                "title": "Rust Programming Language",
                "description": "A language empowering everyone to build reliable software.",
                "snippet": "rust-lang.org homepage"
            },
            {
                "url": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
                "title": "Rust (programming language) - Wikipedia",
                "description": null,
                "snippet": "Rust is a multi-paradigm, general-purpose programming language."
            }
        ]
    })
}

fn provider(server: &MockServer) -> YouComProvider {
    let config = YouComConfig::new(API_KEY).with_base_url(server.uri());
    YouComProvider::new(config)
}

fn opts(query: &str, max_results: Option<u32>) -> SearchCallOptions {
    let mut o = SearchCallOptions::new(query);
    o.max_results = max_results;
    o
}

async fn mount_search_mock(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/search"))
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
    assert_eq!(
        result.results[0].title.as_deref(),
        Some("Rust Programming Language")
    );
    assert_eq!(
        result.results[0].url.as_deref(),
        Some("https://www.rust-lang.org")
    );
    // description takes precedence over snippet.
    assert_eq!(
        result.results[0].content.as_deref(),
        Some("A language empowering everyone to build reliable software.")
    );
    // description is null → snippet is used as the content fallback.
    assert_eq!(
        result.results[1].title.as_deref(),
        Some("Rust (programming language) - Wikipedia")
    );
    assert_eq!(
        result.results[1].content.as_deref(),
        Some("Rust is a multi-paradigm, general-purpose programming language.")
    );
    assert!(result.answer.is_none());
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
    assert_eq!(
        requests[0].url.path(),
        "/v1/search",
        "expected path /v1/search"
    );
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
    // The mock only matches GET /v1/search with the correct X-API-Key header.
    Mock::given(method("GET"))
        .and(path("/v1/search"))
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
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "message": "Unauthorized", "type": "auth_error" }
        })))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    let result = model.do_search(&opts("rust", Some(10))).await;
    assert!(
        matches!(result, Err(AiMuxError::Auth(ref m)) if m == "Unauthorized"),
        "expected Auth error, got {result:?}"
    );
}

// -- Provider trait ----------------------------------------------------------

#[tokio::test]
async fn provider_name_is_you_com() {
    let config = YouComConfig::new(API_KEY);
    let provider = YouComProvider::new(config);
    assert_eq!(provider.name(), "you_com");
}

#[test]
fn language_model_returns_unsupported_error() {
    let config = YouComConfig::new(API_KEY);
    let provider = YouComProvider::new(config);
    match provider.language_model("youcom-search") {
        Err(AiMuxError::Unsupported(msg)) => {
            assert!(
                msg.contains("provider 'you_com' does not provide language models"),
                "unexpected message: {msg}"
            );
        }
        _ => panic!("expected Unsupported error, got success or another error variant"),
    }
}
