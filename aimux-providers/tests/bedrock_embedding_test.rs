//! Rust translations of the AI SDK Amazon Bedrock embedding model tests.
//!
//! Source: `reference/ai/packages/amazon-bedrock/src/amazon-bedrock-embedding-model.test.ts`
//! (536 lines, 12 cases).
//!
//! Uses Bearer token auth to avoid SigV4 signing complexity in tests (the TS
//! tests use a `fakeFetchWithAuth` that injects auth headers).

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_providers::{BedrockAuth, BedrockProvider, BedrockProviderConfig};

const TEST_VALUES: &[&str] = &["sunny day at the beach", "rainy day in the city"];

fn mock_embeddings() -> Vec<Vec<f32>> {
    vec![
        vec![-0.09, 0.05, -0.02, 0.01, 0.04],
        vec![-0.08, 0.06, -0.03, 0.02, 0.03],
    ]
}

fn test_provider(base_url: String) -> BedrockProvider {
    let config = BedrockProviderConfig {
        base_url,
        auth: BedrockAuth::BearerToken("test-auth".to_string()),
        region: "us-east-1".to_string(),
        retry_config: aimux_provider_utils::RetryConfig::default(),
        api_key_source: None,
    };
    BedrockProvider::new(config)
}

fn default_options(values: Vec<String>) -> EmbeddingCallOptions {
    EmbeddingCallOptions {
        values,
        abort_signal: None,
        provider_options: None,
        headers: None,
    }
}

/// TS: "should expose model-specific max embeddings per call"
#[test]
fn should_expose_max_embeddings_per_call() {
    let provider = test_provider("https://example.com".to_string());

    let titan_model = provider.embedding_model("amazon.titan-embed-text-v2:0");
    assert_eq!(titan_model.max_embeddings_per_call(), Some(1));

    let cohere_model = provider.embedding_model("cohere.embed-english-v3");
    assert_eq!(cohere_model.max_embeddings_per_call(), Some(96));

    let cohere_v4_us_model = provider.embedding_model("us.cohere.embed-v4:0");
    assert_eq!(cohere_v4_us_model.max_embeddings_per_call(), Some(96));

    let nova_model = provider.embedding_model("amazon.nova-2-multimodal-embeddings-v1:0");
    assert_eq!(nova_model.max_embeddings_per_call(), Some(1));
}

/// TS: "should handle single input value and return embeddings" (Titan)
#[tokio::test]
async fn should_handle_titan_single_input() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/model/amazon.titan-embed-text-v2:0/invoke"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({
                    "embedding": mock_embeddings()[0],
                    "inputTextTokenCount": 8
                })),
        )
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("amazon.titan-embed-text-v2:0");

    let result = model
        .do_embed(&default_options(vec![TEST_VALUES[0].to_string()]))
        .await
        .expect("should succeed");

    assert_eq!(result.embeddings.len(), 1);
    assert_eq!(result.embeddings[0], mock_embeddings()[0]);

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["inputText"], TEST_VALUES[0]);
}

/// TS: "should handle single input value and extract usage" (Titan)
#[tokio::test]
async fn should_extract_titan_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/model/amazon.titan-embed-text-v2:0/invoke"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "embedding": mock_embeddings()[0],
            "inputTextTokenCount": 8
        })))
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("amazon.titan-embed-text-v2:0");

    let result = model
        .do_embed(&default_options(vec![TEST_VALUES[0].to_string()]))
        .await
        .expect("should succeed");

    let usage = result.usage.expect("usage present");
    assert_eq!(usage.tokens, 8);
}

/// TS: "should support Cohere embedding models" (Cohere v3 response)
#[tokio::test]
async fn should_support_cohere_v3() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/model/cohere.embed-english-v3/invoke"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-amzn-bedrock-input-token-count", "6")
                .set_body_json(json!({
                    "embeddings": [mock_embeddings()[0]]
                })),
        )
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("cohere.embed-english-v3");

    let result = model
        .do_embed(&default_options(vec![TEST_VALUES[0].to_string()]))
        .await
        .expect("should succeed");

    assert_eq!(result.embeddings.len(), 1);
    assert_eq!(result.embeddings[0], mock_embeddings()[0]);
    assert_eq!(result.usage.unwrap().tokens, 6);

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["input_type"], "search_query");
    assert_eq!(body["texts"], json!([TEST_VALUES[0]]));
}

/// TS: "should support Cohere v4 embedding models"
#[tokio::test]
async fn should_support_cohere_v4() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/model/cohere.embed-v4:0/invoke"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-amzn-bedrock-input-token-count", "6")
                .set_body_json(json!({
                    "embeddings": { "float": [mock_embeddings()[0]] }
                })),
        )
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("cohere.embed-v4:0");

    let result = model
        .do_embed(&default_options(vec![TEST_VALUES[0].to_string()]))
        .await
        .expect("should succeed");

    assert_eq!(result.embeddings.len(), 1);
    assert_eq!(result.embeddings[0], mock_embeddings()[0]);
    assert_eq!(result.usage.unwrap().tokens, 6);
}

/// TS: "should send multiple values for Cohere embedding models"
#[tokio::test]
async fn should_send_multiple_values_cohere_v4() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/model/cohere.embed-v4:0/invoke"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-amzn-bedrock-input-token-count", "12")
                .set_body_json(json!({
                    "embeddings": { "float": mock_embeddings() }
                })),
        )
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("cohere.embed-v4:0");

    let result = model
        .do_embed(&default_options(
            TEST_VALUES
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
        ))
        .await
        .expect("should succeed");

    assert_eq!(result.embeddings, mock_embeddings());
    assert_eq!(result.usage.unwrap().tokens, 12);

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["input_type"], "search_query");
    assert_eq!(
        body["texts"],
        json!(["sunny day at the beach", "rainy day in the city"])
    );
}

/// TS: "should support Cohere models behind cross-region inference profile ids"
#[tokio::test]
async fn should_support_cross_region_cohere() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/model/us.cohere.embed-v4:0/invoke"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-amzn-bedrock-input-token-count", "6")
                .set_body_json(json!({
                    "embeddings": { "float": [mock_embeddings()[0]] }
                })),
        )
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("us.cohere.embed-v4:0");

    let result = model
        .do_embed(&default_options(vec![TEST_VALUES[0].to_string()]))
        .await
        .expect("should succeed");
    assert_eq!(result.embeddings.len(), 1);
    assert_eq!(result.usage.unwrap().tokens, 6);
}

/// TS: "should pass outputDimension for Cohere v4 embedding models"
#[tokio::test]
async fn should_pass_output_dimension_cohere_v4() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/model/cohere.embed-v4:0/invoke"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-amzn-bedrock-input-token-count", "6")
                .set_body_json(json!({
                    "embeddings": { "float": [mock_embeddings()[0]] }
                })),
        )
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("cohere.embed-v4:0");

    let mut provider_options = HashMap::new();
    provider_options.insert("bedrock".to_string(), json!({"outputDimension": 256}));
    let options = EmbeddingCallOptions {
        values: vec![TEST_VALUES[0].to_string()],
        abort_signal: None,
        provider_options: Some(provider_options),
        headers: None,
    };

    let _ = model.do_embed(&options).await.expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["output_dimension"], 256);
}

/// TS: "should send SINGLE_EMBEDDING payload for Nova embeddings"
#[tokio::test]
async fn should_support_nova_embeddings() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/model/amazon.nova-2-multimodal-embeddings-v1:0/invoke",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "embeddings": [
                { "embeddingType": "TEXT", "embedding": mock_embeddings()[0] }
            ],
            "inputTokenCount": 8
        })))
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("amazon.nova-2-multimodal-embeddings-v1:0");

    let result = model
        .do_embed(&default_options(vec![TEST_VALUES[0].to_string()]))
        .await
        .expect("should succeed");
    assert_eq!(result.embeddings[0], mock_embeddings()[0]);
    assert_eq!(result.usage.unwrap().tokens, 8);

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["taskType"], "SINGLE_EMBEDDING");
    assert_eq!(
        body["singleEmbeddingParams"]["embeddingPurpose"],
        "GENERIC_INDEX"
    );
    assert_eq!(body["singleEmbeddingParams"]["embeddingDimension"], 1024);
    assert_eq!(
        body["singleEmbeddingParams"]["text"]["truncationMode"],
        "END"
    );
    assert_eq!(
        body["singleEmbeddingParams"]["text"]["value"],
        TEST_VALUES[0]
    );
}

/// TS: "should pass embeddingDimension for Nova embeddings"
#[tokio::test]
async fn should_pass_nova_embedding_dimension() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/model/amazon.nova-2-multimodal-embeddings-v1:0/invoke",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "embeddings": [
                { "embeddingType": "TEXT", "embedding": mock_embeddings()[0] }
            ],
            "inputTokenCount": 8
        })))
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("amazon.nova-2-multimodal-embeddings-v1:0");

    let mut provider_options = HashMap::new();
    provider_options.insert("bedrock".to_string(), json!({"embeddingDimension": 256}));
    let options = EmbeddingCallOptions {
        values: vec![TEST_VALUES[0].to_string()],
        abort_signal: None,
        provider_options: Some(provider_options),
        headers: None,
    };

    let _ = model.do_embed(&options).await.expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["singleEmbeddingParams"]["embeddingDimension"], 256);
}
