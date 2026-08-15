//! Rust translations of the AI SDK Mistral embedding model tests.
//!
//! Source: `reference/ai/packages/mistral/src/mistral-embedding-model.test.ts`
//! (127 lines, 5 cases).

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_providers::{MistralConfig, MistralProvider};

const TEST_VALUES: &[&str] = &["sunny day at the beach", "rainy day in the city"];

fn dummy_embeddings() -> Vec<Vec<f32>> {
    vec![vec![0.1, 0.2, 0.3, 0.4, 0.5], vec![0.6, 0.7, 0.8, 0.9, 1.0]]
}

fn embedding_response_body(embeddings: &[Vec<f32>], usage: Option<Value>) -> Value {
    json!({
        "id": "b322cfc2b9d34e2f8e14fc99874faee5",
        "object": "list",
        "data": embeddings.iter().enumerate().map(|(i, e)| {
            json!({ "object": "embedding", "embedding": e, "index": i })
        }).collect::<Vec<_>>(),
        "model": "mistral-embed",
        "usage": usage.unwrap_or_else(|| json!({ "prompt_tokens": 8, "total_tokens": 8 })),
    })
}

fn test_values() -> Vec<String> {
    TEST_VALUES
        .iter()
        .map(std::string::ToString::to_string)
        .collect()
}

fn default_options(values: Vec<String>) -> EmbeddingCallOptions {
    EmbeddingCallOptions {
        values,
        abort_signal: None,
        provider_options: None,
        headers: None,
    }
}

/// TS: "should extract embedding"
#[tokio::test]
async fn should_extract_embedding() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(embedding_response_body(&dummy_embeddings(), None)),
        )
        .mount(&server)
        .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.embedding_model("mistral-embed");

    let result = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");
    assert_eq!(result.embeddings, dummy_embeddings());
}

/// TS: "should extract usage"
#[tokio::test]
async fn should_extract_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(embedding_response_body(
                &dummy_embeddings(),
                Some(json!({ "prompt_tokens": 20, "total_tokens": 20 })),
            )),
        )
        .mount(&server)
        .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.embedding_model("mistral-embed");

    let result = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");
    let usage = result.usage.expect("usage should be present");
    assert_eq!(usage.tokens, 20);
}

/// TS: "should expose the raw response"
#[tokio::test]
async fn should_expose_raw_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("test-header", "test-value")
                .set_body_json(embedding_response_body(&dummy_embeddings(), None)),
        )
        .mount(&server)
        .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.embedding_model("mistral-embed");

    let result = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");
    let headers = result
        .response
        .as_ref()
        .and_then(|r| r.headers.as_ref())
        .expect("headers present");
    assert_eq!(
        headers.get("test-header").map(std::string::String::as_str),
        Some("test-value")
    );
}

/// TS: "should pass the model and the values"
#[tokio::test]
async fn should_pass_model_and_values() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(embedding_response_body(&dummy_embeddings(), None)),
        )
        .mount(&server)
        .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.embedding_model("mistral-embed");

    let _ = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "mistral-embed");
    assert_eq!(
        body["input"],
        json!(["sunny day at the beach", "rainy day in the city"])
    );
    assert_eq!(body["encoding_format"], "float");
}

/// TS: "should pass headers"
#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(embedding_response_body(&dummy_embeddings(), None)),
        )
        .mount(&server)
        .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.embedding_model("mistral-embed");

    let mut request_headers = HashMap::new();
    request_headers.insert(
        "Custom-Request-Header".to_string(),
        "request-header-value".to_string(),
    );

    let options = EmbeddingCallOptions {
        values: test_values(),
        abort_signal: None,
        provider_options: None,
        headers: Some(request_headers),
    };

    let _ = model.do_embed(&options).await.expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let h = &requests[0].headers;
    assert_eq!(
        h.get("authorization").and_then(|v| v.to_str().ok()),
        Some("Bearer test-api-key")
    );
    assert_eq!(
        h.get("custom-request-header").and_then(|v| v.to_str().ok()),
        Some("request-header-value")
    );
}
