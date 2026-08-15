//! DataForSEO search provider tests.
//!
//! Each test spins up a `wiremock` mock server, configures a JSON response,
//! creates a `DataforseoSearchModel` pointing at the mock, calls `do_search`,
//! and asserts on the request body / auth header / result / error mapping.
//!
//! Tests do not hit the public network and do not read real credentials.

use base64::Engine;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::Provider;
use aimux_core::error::AiMuxError;
use aimux_core::search_model::{SearchCallOptions, SearchModel};
use aimux_providers::{DataforseoConfig, DataforseoProvider};

const LOGIN: &str = "test-login";
const PASSWORD: &str = "test-password";

// -- helpers -----------------------------------------------------------------

/// A realistic DataForSEO live/advanced success response.
///
/// The organic list is nested under `tasks[].result[].organic[]`; extra fields
/// (`version`, `status_code`, task `id`, `type`, `se_domain`, …) are ignored
/// by the model.
fn search_response_body() -> Value {
    json!({
        "version": "0.1.20250101",
        "status_code": 20000,
        "status_message": "Ok.",
        "tasks": [{
            "id": "07281559-1535-0428",
            "status_code": 20000,
            "result": [{
                "keyword": "rust",
                "type": "organic",
                "se_domain": "google.com",
                "location_code": 2840,
                "language_code": "en",
                "organic": [
                    {
                        "rank_group": 1,
                        "rank_absolute": 1,
                        "title": "Rust Programming Language",
                        "url": "https://www.rust-lang.org",
                        "description": "A language empowering everyone to build reliable software."
                    },
                    {
                        "rank_group": 2,
                        "rank_absolute": 2,
                        "title": "Rust (programming language) - Wikipedia",
                        "url": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
                        "description": "Rust is a multi-paradigm, general-purpose programming language."
                    }
                ]
            }]
        }]
    })
}

fn provider(server: &MockServer) -> DataforseoProvider {
    let config = DataforseoConfig::new(LOGIN, PASSWORD).with_base_url(server.uri());
    DataforseoProvider::new(config)
}

fn opts(query: &str, max_results: Option<u32>) -> SearchCallOptions {
    let mut o = SearchCallOptions::new(query);
    o.max_results = max_results;
    o
}

async fn mount_search_mock(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v3/serp/google/organic/live/advanced"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_response_body()))
        .mount(server)
        .await;
}

/// The expected `Authorization: Basic …` header value for the test credentials.
fn expected_basic_auth() -> String {
    let credentials = format!("{LOGIN}:{PASSWORD}");
    let encoded = base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes());
    format!("Basic {encoded}")
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
    // title / url / description→content, in provider order.
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
    assert!(result.answer.is_none());
}

// -- URL + request body ------------------------------------------------------

#[tokio::test]
async fn do_search_hits_correct_url_with_array_body() {
    let server = MockServer::start().await;
    mount_search_mock(&server).await;

    let provider = provider(&server);
    let model = provider.search_model();

    model.do_search(&opts("rust", Some(10))).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url.path(),
        "/v3/serp/google/organic/live/advanced",
        "expected the live/advanced endpoint path"
    );
    // The body is a JSON array of one task object.
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(body.is_array(), "expected an array body, got {body}");
    assert_eq!(body[0]["keyword"], "rust");
    assert_eq!(body[0]["max_credits"], 1);
    assert_eq!(body[0]["depth"], 10);
    assert_eq!(
        requests[0]
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
}

// -- auth header (HTTP Basic) ------------------------------------------------

#[tokio::test]
async fn do_search_sends_correct_basic_auth_header() {
    let server = MockServer::start().await;
    let expected = expected_basic_auth();
    // The mock only matches the POST with the correct Basic auth header.
    Mock::given(method("POST"))
        .and(path("/v3/serp/google/organic/live/advanced"))
        .and(header("authorization", expected.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_response_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    model.do_search(&opts("rust", Some(10))).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(
        headers.get("authorization").and_then(|v| v.to_str().ok()),
        Some(expected.as_str())
    );
    // Credentials must not leak into any other header.
    assert!(
        !headers.iter().any(|(_, v)| v
            .to_str()
            .map(|s| s.contains(LOGIN) || s.contains(PASSWORD))
            .unwrap_or(false)),
        "credentials leaked into a header"
    );
}

// -- error mapping -----------------------------------------------------------

#[tokio::test]
async fn status_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    // DataForSEO vendor-specific error shape: { status_code, status_message }.
    Mock::given(method("POST"))
        .and(path("/v3/serp/google/organic/live/advanced"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "status_code": 40100,
            "status_message": "Invalid login or password."
        })))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let model = provider.search_model();

    let result = model.do_search(&opts("rust", Some(10))).await;
    assert!(
        matches!(result, Err(AiMuxError::ApiCall(ref m)) if m.status_code == Some(401) && m.message == "Invalid login or password."),
        "expected Auth error, got {result:?}"
    );
}

// -- Provider trait ----------------------------------------------------------

#[tokio::test]
async fn provider_name_is_dataforseo() {
    let config = DataforseoConfig::new(LOGIN, PASSWORD);
    let provider = DataforseoProvider::new(config);
    assert_eq!(provider.name(), "dataforseo");
}

#[test]
fn language_model_returns_unsupported_error() {
    let config = DataforseoConfig::new(LOGIN, PASSWORD);
    let provider = DataforseoProvider::new(config);
    match provider.language_model("dataforseo-search") {
        Err(AiMuxError::UnsupportedFunctionality(msg)) => {
            assert!(
                msg.contains("provider 'dataforseo' does not provide language models"),
                "unexpected message: {msg}"
            );
        }
        _ => panic!("expected Unsupported error, got success or another error variant"),
    }
}
