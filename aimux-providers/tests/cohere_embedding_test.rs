//! Rust translations of the AI SDK Cohere embedding model tests.
//!
//! Source: `reference/ai/packages/cohere/src/cohere-embedding-model.test.ts`
//! (185 lines, 6 cases).

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_providers::{CohereConfig, CohereProvider};

const TEST_VALUES: &[&str] = &["sunny day at the beach", "rainy day in the city"];

/// The fixture response body from `__fixtures__/cohere-embedding.json`.
fn embedding_response_body() -> Value {
    json!({
        "id": "test-id",
        "embeddings": {
            "float": [
                [0.03302002, 0.020904541, -0.019744873, -0.0625, 0.04437256],
                [-0.04660034, 0.00037765503, -0.061157227, -0.08239746, -0.010360718]
            ]
        },
        "meta": {
            "billed_units": { "input_tokens": 10 }
        }
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
        max_retries: None,
        timeout: None,
    }
}

/// TS: "should extract embedding"
#[tokio::test]
async fn should_extract_embedding() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.embedding_model("embed-english-v3.0");

    let result = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");
    assert_eq!(result.embeddings.len(), 2);
    assert_eq!(
        result.embeddings[0],
        vec![0.03302002, 0.020904541, -0.019744873, -0.0625, 0.04437256]
    );
    assert_eq!(
        result.embeddings[1],
        vec![
            -0.04660034,
            0.00037765503,
            -0.061157227,
            -0.08239746,
            -0.010360718
        ]
    );
}

/// TS: "should expose the raw response"
#[tokio::test]
async fn should_expose_raw_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embed"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("test-header", "test-value")
                .set_body_json(embedding_response_body()),
        )
        .mount(&server)
        .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.embedding_model("embed-english-v3.0");

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

/// TS: "should extract usage"
#[tokio::test]
async fn should_extract_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.embedding_model("embed-english-v3.0");

    let result = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");
    let usage = result.usage.expect("usage should be present");
    assert_eq!(usage.tokens, 10);
}

/// TS: "should pass the model and the values"
#[tokio::test]
async fn should_pass_model_and_values() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.embedding_model("embed-english-v3.0");

    let _ = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "embed-english-v3.0");
    assert_eq!(body["embedding_types"], json!(["float"]));
    assert_eq!(body["input_type"], "search_query");
    assert_eq!(
        body["texts"],
        json!(["sunny day at the beach", "rainy day in the city"])
    );
}

/// TS: "should pass the input_type setting"
#[tokio::test]
async fn should_pass_input_type_setting() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.embedding_model("embed-english-v3.0");

    let mut provider_options = HashMap::new();
    provider_options.insert(
        "cohere".to_string(),
        json!({"inputType": "search_document"}),
    );
    let options = EmbeddingCallOptions {
        values: test_values(),
        abort_signal: None,
        provider_options: Some(provider_options),
        headers: None,
        max_retries: None,
        timeout: None,
    };

    let _ = model.do_embed(&options).await.expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["input_type"], "search_document");
}

/// TS: "should pass the output_dimension setting"
#[tokio::test]
async fn should_pass_output_dimension_setting() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.embedding_model("embed-v4.0");

    let mut provider_options = HashMap::new();
    provider_options.insert("cohere".to_string(), json!({"outputDimension": 256}));
    let options = EmbeddingCallOptions {
        values: test_values(),
        abort_signal: None,
        provider_options: Some(provider_options),
        headers: None,
        max_retries: None,
        timeout: None,
    };

    let _ = model.do_embed(&options).await.expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["output_dimension"], 256);
    assert_eq!(body["model"], "embed-v4.0");
}

/// TS: "should pass headers"
#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
        .mount(&server)
        .await;

    let config = CohereConfig::new("test-api-key").with_base_url(server.uri());
    let provider = CohereProvider::new(config);
    let model = provider.embedding_model("embed-english-v3.0");

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
        max_retries: None,
        timeout: None,
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
