//! Rust translations of the AI SDK OpenAI embedding model tests.
//!
//! Source: `reference/ai/packages/openai/src/embedding/openai-embedding-model.test.ts`
//! (164 lines, 7 cases).
//!
//! Each test uses `wiremock` to spin up a mock HTTP server, configures a JSON
//! response, creates an `OpenAIEmbeddingModel` pointing at the mock, calls
//! `do_embed`, and asserts on the result.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_providers::{OpenAIConfig, OpenAIProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

const TEST_VALUES: &[&str] = &["sunny day at the beach", "rainy day in the city"];

/// The fixture response body from `__fixtures__/openai-embedding.json`.
fn embedding_response_body() -> Value {
    json!({
        "object": "list",
        "data": [
            {
                "object": "embedding",
                "index": 0,
                "embedding": [0.0057293195, -0.012727811, 0.020042092, -0.013437585, 0.022833068]
            },
            {
                "object": "embedding",
                "index": 1,
                "embedding": [-0.037104916, -0.05178114, -0.008340587, 0.001164541, -0.0035253682]
            }
        ],
        "model": "text-embedding-3-small",
        "usage": {
            "prompt_tokens": 12,
            "total_tokens": 12
        }
    })
}

fn test_values() -> Vec<String> {
    TEST_VALUES.iter().map(|s| s.to_string()).collect()
}

fn default_options(values: Vec<String>) -> EmbeddingCallOptions {
    EmbeddingCallOptions {
        values,
        abort_signal: None,
        provider_options: None,
        headers: None,
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doEmbed
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should extract embedding"
#[tokio::test]
async fn should_extract_embedding() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.embedding_model("text-embedding-3-large");

    let result = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");

    assert_eq!(result.embeddings.len(), 2);
    assert_eq!(
        result.embeddings[0],
        vec![
            0.0057293195,
            -0.012727811,
            0.020042092,
            -0.013437585,
            0.022833068
        ]
    );
    assert_eq!(
        result.embeddings[1],
        vec![
            -0.037104916,
            -0.05178114,
            -0.008340587,
            0.001164541,
            -0.0035253682
        ]
    );
}

/// TS: "should expose the raw response headers"
#[tokio::test]
async fn should_expose_raw_response_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("test-header", "test-value")
                .set_body_json(embedding_response_body()),
        )
        .mount(&server)
        .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.embedding_model("text-embedding-3-large");

    let result = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");

    let headers = result
        .response
        .as_ref()
        .and_then(|r| r.headers.as_ref())
        .expect("response headers should be present");
    assert_eq!(
        headers.get("test-header").map(|s| s.as_str()),
        Some("test-value")
    );
}

/// TS: "should expose the raw response body"
#[tokio::test]
async fn should_expose_raw_response_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.embedding_model("text-embedding-3-large");

    let result = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");

    let body = result
        .response
        .as_ref()
        .and_then(|r| r.body.as_ref())
        .expect("response body should be present");
    assert_eq!(body["usage"]["prompt_tokens"], 12);
    assert_eq!(body["data"].as_array().unwrap().len(), 2);
}

/// TS: "should extract usage"
#[tokio::test]
async fn should_extract_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.embedding_model("text-embedding-3-large");

    let result = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");

    let usage = result.usage.expect("usage should be present");
    assert_eq!(usage.tokens, 12);
}

/// TS: "should pass the model and the values"
#[tokio::test]
async fn should_pass_model_and_values() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.embedding_model("text-embedding-3-large");

    let _ = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();

    assert_eq!(body["model"], "text-embedding-3-large");
    assert_eq!(
        body["input"],
        json!(["sunny day at the beach", "rainy day in the city"])
    );
    assert_eq!(body["encoding_format"], "float");
    // dimensions and user should be absent when not provided
    assert!(body.get("dimensions").is_none());
    assert!(body.get("user").is_none());
}

/// TS: "should pass the dimensions setting"
#[tokio::test]
async fn should_pass_dimensions_setting() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.embedding_model("text-embedding-3-large");

    let mut provider_options = HashMap::new();
    provider_options.insert("openai".to_string(), json!({"dimensions": 64}));
    let options = EmbeddingCallOptions {
        values: test_values(),
        abort_signal: None,
        provider_options: Some(provider_options),
        headers: None,
    };

    let _ = model.do_embed(&options).await.expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();

    assert_eq!(body["dimensions"], 64);
    assert_eq!(body["model"], "text-embedding-3-large");
    assert_eq!(
        body["input"],
        json!(["sunny day at the beach", "rainy day in the city"])
    );
    assert_eq!(body["encoding_format"], "float");
}

/// TS: "should pass headers" — verifies that Authorization, OpenAI-Organization,
/// OpenAI-Project, config-level headers, and request-level headers are all sent.
#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let mut config_headers = HashMap::new();
    config_headers.insert(
        "Custom-Provider-Header".to_string(),
        "provider-header-value".to_string(),
    );

    let config = OpenAIConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_org_id("test-organization")
        .with_project("test-project")
        .with_headers(config_headers);
    let provider = OpenAIProvider::new(config);
    let model = provider.embedding_model("text-embedding-3-large");

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
        h.get("openai-organization").and_then(|v| v.to_str().ok()),
        Some("test-organization")
    );
    assert_eq!(
        h.get("openai-project").and_then(|v| v.to_str().ok()),
        Some("test-project")
    );
    assert_eq!(
        h.get("custom-provider-header")
            .and_then(|v| v.to_str().ok()),
        Some("provider-header-value")
    );
    assert_eq!(
        h.get("custom-request-header").and_then(|v| v.to_str().ok()),
        Some("request-header-value")
    );
}
