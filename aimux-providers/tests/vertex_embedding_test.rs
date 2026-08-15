//! Rust translations of the AI SDK Google Vertex embedding model tests.
//!
//! Source: `reference/ai/packages/google-vertex/src/google-vertex-embedding-model.test.ts`
//! (460 lines, 12 cases).

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_providers::{VertexAuth, VertexProvider, VertexProviderConfig};

use aimux_provider_utils::RetryConfig;

const TEST_VALUES: &[&str] = &["test text one", "test text two"];

fn mock_provider_options() -> HashMap<String, Value> {
    let mut opts = HashMap::new();
    opts.insert(
        "google".to_string(),
        json!({
            "outputDimensionality": 768,
            "taskType": "SEMANTIC_SIMILARITY",
            "title": "test title",
            "autoTruncate": false
        }),
    );
    opts
}

/// The fixture response body from `__fixtures__/google-vertex-embedding.json`.
fn predict_response_body() -> Value {
    json!({
        "predictions": [
            {
                "embeddings": {
                    "values": [-0.017999587580561638, -0.006893285550177097, -0.036766719073057175, -0.017558680847287178, -0.019938766956329346],
                    "statistics": { "token_count": 6 }
                }
            },
            {
                "embeddings": {
                    "values": [-0.06007182598114014, 0.004907649010419846, -0.00690646655857563, -0.007314121350646019, -0.048464205116033554],
                    "statistics": { "token_count": 5 }
                }
            }
        ]
    })
}

fn embed_content_response_body() -> Value {
    json!({
        "embedding": { "values": [0.1, 0.2, 0.3] },
        "usageMetadata": { "promptTokenCount": 4 }
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
        provider_options: Some(mock_provider_options()),
        headers: None,
    }
}

fn test_provider(base_url: String) -> VertexProvider {
    let config = VertexProviderConfig {
        base_url,
        project: Some("test-project".to_string()),
        location: Some("us-central1".to_string()),
        auth: VertexAuth::BearerToken("test-token".to_string()),
        api_key_source: None,
        retry_config: RetryConfig::default(),
    };
    VertexProvider::new(config)
}

/// TS: "should extract embeddings"
// f32 test fixtures mirror provider output verbatim
#[allow(clippy::excessive_precision)]
#[tokio::test]
async fn should_extract_embeddings() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/textembedding-gecko@001:predict"))
        .respond_with(ResponseTemplate::new(200).set_body_json(predict_response_body()))
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("textembedding-gecko@001");

    let result = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");
    assert_eq!(result.embeddings.len(), 2);
    assert_eq!(
        result.embeddings[0],
        vec![
            -0.017999587580561638,
            -0.006893285550177097,
            -0.036766719073057175,
            -0.017558680847287178,
            -0.019938766956329346
        ]
    );
    assert_eq!(
        result.embeddings[1],
        vec![
            -0.06007182598114014,
            0.004907649010419846,
            -0.00690646655857563,
            -0.007314121350646019,
            -0.048464205116033554
        ]
    );
}

/// TS: "should extract usage"
#[tokio::test]
async fn should_extract_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/textembedding-gecko@001:predict"))
        .respond_with(ResponseTemplate::new(200).set_body_json(predict_response_body()))
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("textembedding-gecko@001");

    let result = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");
    let usage = result.usage.expect("usage should be present");
    // token_count: 6 + 5 = 11
    assert_eq!(usage.tokens, 11);
}

/// TS: "should pass the model parameters correctly"
#[tokio::test]
async fn should_pass_model_parameters() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/textembedding-gecko@001:predict"))
        .respond_with(ResponseTemplate::new(200).set_body_json(predict_response_body()))
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("textembedding-gecko@001");

    let _ = model
        .do_embed(&default_options(test_values()))
        .await
        .expect("should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["instances"].as_array().unwrap().len(), 2);
    assert_eq!(body["instances"][0]["content"], "test text one");
    assert_eq!(body["instances"][0]["task_type"], "SEMANTIC_SIMILARITY");
    assert_eq!(body["instances"][0]["title"], "test title");
    assert_eq!(body["parameters"]["autoTruncate"], false);
    assert_eq!(body["parameters"]["outputDimensionality"], 768);
}

/// TS: "should accept googleVertex as provider options key"
#[tokio::test]
async fn should_accept_google_vertex_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/textembedding-gecko@001:predict"))
        .respond_with(ResponseTemplate::new(200).set_body_json(predict_response_body()))
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("textembedding-gecko@001");

    let mut provider_options = HashMap::new();
    provider_options.insert(
        "googleVertex".to_string(),
        json!({
            "outputDimensionality": 768,
            "taskType": "SEMANTIC_SIMILARITY",
            "title": "test title",
            "autoTruncate": false
        }),
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
    assert_eq!(body["instances"][0]["task_type"], "SEMANTIC_SIMILARITY");
    assert_eq!(body["parameters"]["outputDimensionality"], 768);
}

/// TS: "should pass the taskType setting in instances"
#[tokio::test]
async fn should_pass_task_type_only() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/textembedding-gecko@001:predict"))
        .respond_with(ResponseTemplate::new(200).set_body_json(predict_response_body()))
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("textembedding-gecko@001");

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
    assert_eq!(body["instances"][0]["task_type"], "SEMANTIC_SIMILARITY");
    // parameters should be empty object when no outputDimensionality/autoTruncate
    assert_eq!(body["parameters"].as_object().unwrap().len(), 0);
}

/// TS: "should use embedContent for gemini-embedding-2"
#[tokio::test]
async fn should_use_embed_content_for_gemini_2() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-embedding-2:embedContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embed_content_response_body()))
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("gemini-embedding-2");

    let result = model
        .do_embed(&default_options(vec![TEST_VALUES[0].to_string()]))
        .await
        .expect("should succeed");
    assert_eq!(result.embeddings, vec![vec![0.1, 0.2, 0.3]]);
    let usage = result.usage.expect("usage present");
    assert_eq!(usage.tokens, 4);

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["content"]["parts"][0]["text"], "test text one");
    assert_eq!(body["embedContentConfig"]["autoTruncate"], false);
    assert_eq!(body["embedContentConfig"]["outputDimensionality"], 768);
    assert_eq!(
        body["embedContentConfig"]["taskType"],
        "SEMANTIC_SIMILARITY"
    );
    assert_eq!(body["embedContentConfig"]["title"], "test title");
}

/// TS: "should limit gemini-embedding-2 to one value per call"
#[test]
fn gemini_embedding_2_max_per_call() {
    let config = VertexProviderConfig {
        base_url: "https://example.com".to_string(),
        project: Some("test-project".to_string()),
        location: Some("us-central1".to_string()),
        auth: VertexAuth::BearerToken("test".to_string()),
        api_key_source: None,
        retry_config: RetryConfig::default(),
    };
    let provider = VertexProvider::new(config);
    let model = provider.embedding_model("gemini-embedding-2");
    assert_eq!(model.max_embeddings_per_call(), Some(1));
}

/// TS: "should expose response headers"
#[tokio::test]
async fn should_expose_response_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/textembedding-gecko@001:predict"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("test-header", "test-value")
                .set_body_json(predict_response_body()),
        )
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let model = provider.embedding_model("textembedding-gecko@001");

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
