//! Tavily search provider tests.

use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::provider::Provider;
use aimux_core::search_model::{SearchCallOptions, SearchModel};

use aimux_providers::{TavilyConfig, TavilyProvider};

fn make_provider(server: &MockServer) -> TavilyProvider {
    let config = TavilyConfig::new("test-api-key").with_base_url(server.uri());
    TavilyProvider::new(config)
}

#[test]
fn provider_name_is_tavily() {
    let provider = TavilyProvider::new(TavilyConfig::new("test-key"));
    assert_eq!(provider.name(), "tavily");
}

#[test]
fn language_model_returns_unsupported() {
    let provider = TavilyProvider::new(TavilyConfig::new("test-key"));
    assert!(provider.language_model("any").is_err());
}

#[tokio::test]
async fn do_search_returns_results() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "query": "rust lang",
            "answer": "Rust is a systems programming language.",
            "results": [
                {"title": "Rust", "url": "https://rust-lang.org", "content": "Rust is...", "score": 0.95}
            ]
        })))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.search_model();
    let result = model
        .do_search(&SearchCallOptions::new("rust lang"))
        .await
        .expect("should succeed");

    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].title.as_deref(), Some("Rust"));
    assert_eq!(
        result.results[0].url.as_deref(),
        Some("https://rust-lang.org")
    );
    assert_eq!(result.results[0].score, Some(0.95));
    assert_eq!(
        result.answer.as_deref(),
        Some("Rust is a systems programming language.")
    );
}

#[tokio::test]
async fn uses_bearer_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .and(header("authorization", "Bearer my-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"results": []})))
        .mount(&server)
        .await;

    let config = TavilyConfig::new("my-key").with_base_url(server.uri());
    let provider = TavilyProvider::new(config);
    let model = provider.search_model();
    model
        .do_search(&SearchCallOptions::new("test"))
        .await
        .expect("should succeed");
}

#[tokio::test]
async fn status_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "detail": {"error": "Invalid API key"}
        })))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.search_model();
    let result = model.do_search(&SearchCallOptions::new("test")).await;
    assert!(result.is_err());
}
