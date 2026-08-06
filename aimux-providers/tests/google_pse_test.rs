//! Google PSE (Custom Search) provider tests.
//!
//! Each test spins up a `wiremock` mock server, configures a JSON response,
//! creates a `GooglePseSearchModel` pointing at the mock, calls `do_search`,
//! and asserts on the request URL / query params / result.
//!
//! Tests do not hit the public network and do not read real credentials.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::Provider;
use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel};
use aimux_providers::{GooglePseConfig, GooglePseProvider};

const API_KEY: &str = "test-google-key";
const CX: &str = "test-cx-id";

// -- helpers -----------------------------------------------------------------

/// A realistic Google Custom Search success response.
fn search_body() -> Value {
    json!({
        "items": [
            {
                "title": "Rust Programming Language",
                "link": "https://www.rust-lang.org",
                "snippet": "A language empowering everyone to build reliable software."
            },
            {
                "title": "Rust on Wikipedia",
                "link": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
                "snippet": "Rust is a multi-paradigm, general-purpose programming language."
            }
        ]
    })
}

fn provider(server: &MockServer) -> GooglePseProvider {
    let config = GooglePseConfig::new(API_KEY)
        .with_cx(CX)
        .with_base_url(format!("{}/customsearch/v1", server.uri()));
    GooglePseProvider::new(config)
}

fn opts(query: &str) -> SearchCallOptions {
    SearchCallOptions::new(query)
}

async fn mount_search_mock(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/customsearch/v1"))
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
    assert!(result.results[0].score.is_none());
    assert!(result.answer.is_none());
}

// -- URL + auth query params -------------------------------------------------

#[tokio::test]
async fn do_search_hits_correct_url_with_auth_query_params() {
    let server = MockServer::start().await;
    // The mock matches GET /customsearch/v1 with the key, cx, and q params, so
    // a wrong URL or missing auth would fail to match.
    Mock::given(method("GET"))
        .and(path("/customsearch/v1"))
        .and(query_param("key", API_KEY))
        .and(query_param("cx", CX))
        .and(query_param("q", "rust language"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    model.do_search(&opts("rust language")).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/customsearch/v1");
    let query = requests[0].url.query().unwrap_or("");
    assert!(query.contains("key=test-google-key"), "query: {query}");
    assert!(query.contains("cx=test-cx-id"), "query: {query}");
    assert!(
        query.contains("q=rust+language") || query.contains("q=rust%20language"),
        "query: {query}"
    );

    // Google PSE authenticates via query params — no Authorization header.
    assert!(
        requests[0].headers.get("authorization").is_none(),
        "google_pse must not send an Authorization header"
    );
}

// -- num (max_results) -------------------------------------------------------

#[tokio::test]
async fn do_search_forwards_max_results_as_num() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/customsearch/v1"))
        .and(query_param("num", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    let mut options = opts("rust language");
    options.max_results = Some(5);
    model.do_search(&options).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let query = requests[0].url.query().unwrap_or("");
    assert!(query.contains("num=5"), "query: {query}");
}

// -- cx from provider_options ------------------------------------------------

#[tokio::test]
async fn cx_resolved_from_provider_options_when_config_has_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/customsearch/v1"))
        .and(query_param("cx", "options-cx"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body()))
        .mount(&server)
        .await;

    // Config without cx.
    let config =
        GooglePseConfig::new(API_KEY).with_base_url(format!("{}/customsearch/v1", server.uri()));
    let provider = GooglePseProvider::new(config);
    let model = provider.search_model();

    let mut options = opts("rust language");
    let mut po = HashMap::new();
    po.insert("google_pse".to_string(), json!({ "cx": "options-cx" }));
    options.provider_options = Some(po);
    model.do_search(&options).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let query = requests[0].url.query().unwrap_or("");
    assert!(query.contains("cx=options-cx"), "query: {query}");
}

#[tokio::test]
async fn missing_cx_returns_invalid_argument_error() {
    let server = MockServer::start().await;
    // Config without cx; no provider_options either.
    let config =
        GooglePseConfig::new(API_KEY).with_base_url(format!("{}/customsearch/v1", server.uri()));
    let provider = GooglePseProvider::new(config);
    let model = provider.search_model();

    let result = model.do_search(&opts("rust language")).await;
    assert!(
        matches!(result, Err(AiMuxError::InvalidArgument(ref m)) if m.contains("cx")),
        "expected InvalidArgument error mentioning cx, got {result:?}"
    );

    // No request should have been issued.
    let requests = server.received_requests().await.unwrap();
    assert!(requests.is_empty());
}

// -- error mapping -----------------------------------------------------------

#[tokio::test]
async fn status_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/customsearch/v1"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "code": 401, "message": "API key not valid" }
        })))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    let result = model.do_search(&opts("rust language")).await;
    assert!(
        matches!(result, Err(AiMuxError::Auth(ref m)) if m == "API key not valid"),
        "expected Auth error, got {result:?}"
    );
}

#[tokio::test]
async fn status_403_maps_to_provider_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/customsearch/v1"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "error": { "code": 403, "message": "Daily limit exceeded" }
        })))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    let result = model.do_search(&opts("rust language")).await;
    assert!(
        matches!(result, Err(AiMuxError::Provider(ref m)) if m.contains("403") && m.contains("Daily limit exceeded")),
        "expected Provider error, got {result:?}"
    );
}

// -- Provider trait ----------------------------------------------------------

#[tokio::test]
async fn provider_name_is_google_pse() {
    let config = GooglePseConfig::new(API_KEY).with_cx(CX);
    let provider = GooglePseProvider::new(config);
    assert_eq!(provider.name(), "google_pse");
}

#[test]
fn language_model_returns_unsupported_error() {
    let config = GooglePseConfig::new(API_KEY).with_cx(CX);
    let provider = GooglePseProvider::new(config);
    match provider.language_model("google-pse-search") {
        Err(AiMuxError::Unsupported(msg)) => {
            assert!(
                msg.contains("provider 'google_pse' does not provide language models"),
                "unexpected message: {msg}"
            );
        }
        _ => panic!("expected Unsupported error, got success or another error variant"),
    }
}

#[test]
fn model_id_is_google_pse_search() {
    let config = GooglePseConfig::new(API_KEY).with_cx(CX);
    let provider = GooglePseProvider::new(config);
    let model = provider.search_model();
    assert_eq!(model.model_id(), "google-pse-search");
    assert_eq!(model.provider(), "google_pse");
}
