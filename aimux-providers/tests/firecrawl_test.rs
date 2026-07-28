//! Firecrawl search provider tests.

use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::provider::Provider;
use aimux_core::search_model::{SearchCallOptions, SearchModel};

use aimux_providers::{FirecrawlConfig, FirecrawlProvider};

fn make_provider(server: &MockServer) -> FirecrawlProvider {
    let config = FirecrawlConfig::new("test-api-key").with_base_url(server.uri());
    FirecrawlProvider::new(config)
}

#[test]
fn provider_name_is_firecrawl() {
    let provider = FirecrawlProvider::new(FirecrawlConfig::new("test-key"));
    assert_eq!(provider.name(), "firecrawl");
}

#[test]
fn language_model_returns_unsupported() {
    let provider = FirecrawlProvider::new(FirecrawlConfig::new("test-key"));
    assert!(provider.language_model("any").is_err());
}

#[tokio::test]
async fn do_search_returns_results() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {
                "web": [
                    {"title": "Rust", "url": "https://rust-lang.org", "markdown": "Rust is..."}
                ]
            }
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
    assert_eq!(result.results[0].content.as_deref(), Some("Rust is..."));
}

#[tokio::test]
async fn uses_bearer_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/search"))
        .and(header("authorization", "Bearer my-key"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": {"web": []}})),
        )
        .mount(&server)
        .await;

    let config = FirecrawlConfig::new("my-key").with_base_url(server.uri());
    let provider = FirecrawlProvider::new(config);
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
        .and(path("/v2/search"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "success": false, "error": "Invalid API key"
        })))
        .mount(&server)
        .await;

    let provider = make_provider(&server);
    let model = provider.search_model();
    let result = model.do_search(&SearchCallOptions::new("test")).await;
    assert!(result.is_err());
}
