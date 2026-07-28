//! Rust translations of the AI SDK Voyage embedding model tests.
//!
//! Source: `reference/ai/packages/voyage/src/voyage-embedding-model.test.ts`
//! (174 lines, 6 cases).

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_providers::{VoyageConfig, VoyageProvider};

const TEST_VALUES: &[&str] = &["sunny day at the beach", "rainy day in the city"];

/// The fixture response body from `__fixtures__/voyage-embedding.json`.
fn embedding_response_body() -> Value {
    json!({
        "object": "list",
        "data": [
            { "object": "embedding", "embedding": [0.000344163, -0.022529466, 0.010127448, 0.063431956, 0.016145896], "index": 0 },
            { "object": "embedding", "embedding": [0.018987041, -0.029901529, -0.005134966, 0.082804598, -0.008740067], "index": 1 }
        ],
        "model": "voyage-3.5",
        "usage": { "total_tokens": 12 }
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

/// TS: "should extract embedding"
#[tokio::test]
#[allow(clippy::excessive_precision)] // f32 test fixtures mirror provider output verbatim
async fn should_extract_embedding() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = VoyageConfig::new("test-api-key").with_base_url(server.uri());
    let provider = VoyageProvider::new(config);
    let model = provider.embedding_model("voyage-3.5");

    let result = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");
    assert_eq!(result.embeddings.len(), 2);
    assert_eq!(
        result.embeddings[0],
        vec![
            0.000344163,
            -0.022529466,
            0.010127448,
            0.063431956,
            0.016145896
        ]
    );
    assert_eq!(
        result.embeddings[1],
        vec![
            0.018987041,
            -0.029901529,
            -0.005134966,
            0.082804598,
            -0.008740067
        ]
    );
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
                .set_body_json(embedding_response_body()),
        )
        .mount(&server)
        .await;

    let config = VoyageConfig::new("test-api-key").with_base_url(server.uri());
    let provider = VoyageProvider::new(config);
    let model = provider.embedding_model("voyage-3.5");

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
        headers.get("test-header").map(|s| s.as_str()),
        Some("test-value")
    );
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

    let config = VoyageConfig::new("test-api-key").with_base_url(server.uri());
    let provider = VoyageProvider::new(config);
    let model = provider.embedding_model("voyage-3.5");

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

    let config = VoyageConfig::new("test-api-key").with_base_url(server.uri());
    let provider = VoyageProvider::new(config);
    let model = provider.embedding_model("voyage-3.5");

    let _ = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "voyage-3.5");
    assert_eq!(
        body["input"],
        json!(["sunny day at the beach", "rainy day in the city"])
    );
    // input_type, truncation, output_dimension, output_dtype absent when not set
    assert!(body.get("input_type").is_none());
}

/// TS: "should pass the input_type setting"
#[tokio::test]
async fn should_pass_input_type_setting() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = VoyageConfig::new("test-api-key").with_base_url(server.uri());
    let provider = VoyageProvider::new(config);
    let model = provider.embedding_model("voyage-3.5");

    let mut provider_options = HashMap::new();
    provider_options.insert("voyage".to_string(), json!({"inputType": "document"}));
    let options = EmbeddingCallOptions {
        values: test_values(),
        abort_signal: None,
        provider_options: Some(provider_options),
        headers: None,
    };

    let _ = model.do_embed(&options).await.expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["input_type"], "document");
}

/// TS: "should pass the output_dimension setting"
#[tokio::test]
async fn should_pass_output_dimension_setting() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = VoyageConfig::new("test-api-key").with_base_url(server.uri());
    let provider = VoyageProvider::new(config);
    let model = provider.embedding_model("voyage-3.5");

    let mut provider_options = HashMap::new();
    provider_options.insert("voyage".to_string(), json!({"outputDimension": 256}));
    let options = EmbeddingCallOptions {
        values: test_values(),
        abort_signal: None,
        provider_options: Some(provider_options),
        headers: None,
    };

    let _ = model.do_embed(&options).await.expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["output_dimension"], 256);
}

/// TS: "should pass headers"
#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = VoyageConfig::new("test-api-key").with_base_url(server.uri());
    let provider = VoyageProvider::new(config);
    let model = provider.embedding_model("voyage-3.5");

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
