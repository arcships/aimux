//! Rust translations of the AI SDK Google embedding model tests.
//!
//! Source: `reference/ai/packages/google/src/google-embedding-model.test.ts`
//! (550 lines, 14 cases — text-only cases translated; multimodal `content`
//! provider-option cases documented as remaining).

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_providers::{GoogleConfig, GoogleProvider};

const TEST_VALUES: &[&str] = &["sunny day at the beach", "rainy day in the city"];

fn dummy_embeddings() -> Vec<Vec<f32>> {
    vec![vec![0.1, 0.2, 0.3, 0.4, 0.5], vec![0.6, 0.7, 0.8, 0.9, 1.0]]
}

fn batch_response_body() -> Value {
    json!({
        "embeddings": dummy_embeddings().iter().map(|e| json!({ "values": e })).collect::<Vec<_>>()
    })
}

fn single_response_body() -> Value {
    json!({ "embedding": { "values": dummy_embeddings()[0] } })
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

/// TS: "should extract embedding" (batch endpoint)
#[tokio::test]
async fn should_extract_embedding() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-embedding-001:batchEmbedContents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(batch_response_body()))
        .mount(&server)
        .await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.embedding_model("gemini-embedding-001");

    let result = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");
    assert_eq!(result.embeddings, dummy_embeddings());
}

/// TS: "should expose the raw response"
#[tokio::test]
async fn should_expose_raw_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-embedding-001:batchEmbedContents"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("test-header", "test-value")
                .set_body_json(batch_response_body()),
        )
        .mount(&server)
        .await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.embedding_model("gemini-embedding-001");

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
        .and(path("/models/gemini-embedding-001:batchEmbedContents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(batch_response_body()))
        .mount(&server)
        .await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.embedding_model("gemini-embedding-001");

    let _ = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["requests"].as_array().unwrap().len(), 2);
    assert_eq!(body["requests"][0]["model"], "models/gemini-embedding-001");
    assert_eq!(
        body["requests"][0]["content"]["parts"][0]["text"],
        "sunny day at the beach"
    );
    assert_eq!(body["requests"][0]["content"]["role"], "user");
    assert_eq!(
        body["requests"][1]["content"]["parts"][0]["text"],
        "rainy day in the city"
    );
}

/// TS: "should pass the outputDimensionality setting"
#[tokio::test]
async fn should_pass_output_dimensionality() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-embedding-001:batchEmbedContents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(batch_response_body()))
        .mount(&server)
        .await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.embedding_model("gemini-embedding-001");

    let mut provider_options = HashMap::new();
    provider_options.insert("google".to_string(), json!({"outputDimensionality": 64}));
    let options = EmbeddingCallOptions {
        values: test_values(),
        abort_signal: None,
        provider_options: Some(provider_options),
        headers: None,
    };

    let _ = model.do_embed(&options).await.expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["requests"][0]["outputDimensionality"], 64);
}

/// TS: "should pass the taskType setting"
#[tokio::test]
async fn should_pass_task_type() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-embedding-001:batchEmbedContents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(batch_response_body()))
        .mount(&server)
        .await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.embedding_model("gemini-embedding-001");

    let mut provider_options = HashMap::new();
    provider_options.insert(
        "google".to_string(),
        json!({"taskType": "SEMANTIC_SIMILARITY"}),
    );
    let options = EmbeddingCallOptions {
        values: test_values(),
        abort_signal: None,
        provider_options: Some(provider_options),
        headers: None,
    };

    let _ = model.do_embed(&options).await.expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["requests"][0]["taskType"], "SEMANTIC_SIMILARITY");
}

/// TS: "should pass headers"
#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-embedding-001:batchEmbedContents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(batch_response_body()))
        .mount(&server)
        .await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.embedding_model("gemini-embedding-001");

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
        h.get("x-goog-api-key").and_then(|v| v.to_str().ok()),
        Some("test-api-key")
    );
    assert_eq!(
        h.get("custom-request-header").and_then(|v| v.to_str().ok()),
        Some("request-header-value")
    );
}

/// TS: "should use the batch embeddings endpoint"
#[tokio::test]
async fn should_use_batch_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-embedding-001:batchEmbedContents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(batch_response_body()))
        .mount(&server)
        .await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.embedding_model("gemini-embedding-001");

    let _ = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");
    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url.as_str().ends_with(":batchEmbedContents"));
}

/// TS: "should use the single embeddings endpoint"
#[tokio::test]
async fn should_use_single_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-embedding-001:embedContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(single_response_body()))
        .mount(&server)
        .await;

    let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
    let provider = GoogleProvider::new(config);
    let model = provider.embedding_model("gemini-embedding-001");

    let result = model
        .do_embed(&default_options(vec![TEST_VALUES[0].to_string()]))
        .await
        .expect("should succeed");
    assert_eq!(result.embeddings.len(), 1);
    assert_eq!(result.embeddings[0], dummy_embeddings()[0]);

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url.as_str().ends_with(":embedContent"));
}

/// TS: "should expose the Google batch embedding API limit"
#[test]
fn should_expose_max_embeddings_per_call() {
    let config = GoogleConfig::new("test-api-key");
    let provider = GoogleProvider::new(config);
    let model = provider.embedding_model("gemini-embedding-001");
    assert_eq!(model.max_embeddings_per_call(), Some(100));
}

// Remaining untranslated cases (multimodal `content` provider option):
// - "should merge multimodal content for single embedding"
// - "should merge per-value multimodal content for batch embedding"
// - "should handle null entries as text-only in batch embedding"
// - "should merge fileData content for single embedding"
// - "should merge fileData content for batch embedding"
// - "should throw error when content length does not match values length"
// These require the multimodal `content` provider option, which involves
// complex per-value part merging. Documented for future translation.
