//! Ports of `src/cohere-embedding-model.test.ts` from `@ai-sdk/cohere@4.0.52`.
//!
//! `should extract embedding`, `should extract usage`, `should pass the model
//! and the values` and `should pass headers` answer from a wiremock server;
//! the provider-option cases go through the mock fetch. `embed_values` in
//! `upstream_e2e.rs` covers the same request in one call.
//! `should expose the raw response` compares the whole snapshot through the
//! mock fetch; the older wiremock case `should_expose_raw_response` checks
//! part of it.
//!
//! Harness differences from the TypeScript (expectations are unchanged):
//! - The TypeScript test server answers from the fixture file. The mock
//!   answers with the fixture's content; `content-length` is the byte length
//!   of the compact JSON body, as the TypeScript server sends it.
//! - The mock records `user-agent`; the TypeScript test checks it with
//!   `vi.mock('./version')`. These cases do not check it.

use std::collections::HashMap;
use std::sync::Arc;

use crate::cohere::{CohereProvider, CohereProviderSettings, create_cohere};
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::provider::Provider;
use aimux_core::shared::SharedProviderOptions;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::test_support::mock_fetch::{Canned, MockFetch};

const EMBEDDING_FIXTURE: &str = include_str!("fixtures/cohere-embedding.json");
const SNAPSHOT_DO_EMBED_RAW_RESPONSE: &str = include_str!("snapshots/do_embed_raw_response.json");

const TEST_VALUES: [&str; 2] = ["sunny day at the beach", "rainy day in the city"];

fn provider(fetch: &Arc<MockFetch>) -> CohereProvider {
    create_cohere(CohereProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// A single-response mock for the embedding fixture.
fn embedding_fetch() -> Arc<MockFetch> {
    embedding_fetch_with(&[])
}

/// `prepareJsonFixtureResponse('cohere-embedding', headers)`.
fn embedding_fetch_with(extra_headers: &[(&str, &str)]) -> Arc<MockFetch> {
    let fixture: Value = serde_json::from_str(EMBEDDING_FIXTURE).expect("fixture is JSON");
    let bytes = serde_json::to_vec(&fixture).expect("serializable");
    let mut headers = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("content-length".to_string(), bytes.len().to_string()),
    ];
    headers.extend(
        extra_headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string())),
    );
    MockFetch::new(vec![Canned {
        status: 200,
        headers,
        body: bytes,
    }])
}

/// `{ values: testValues, providerOptions }`.
fn options(provider_options: Value) -> EmbeddingCallOptions {
    let mut options = EmbeddingCallOptions::new(TEST_VALUES[0]);
    options.values = TEST_VALUES.iter().copied().map(String::from).collect();
    options.provider_options =
        Some(serde_json::from_value::<SharedProviderOptions>(provider_options).unwrap());
    options
}

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

mod do_embed {
    use super::*;

    /// upstream: @ai-sdk/cohere src/cohere-embedding-model.test.ts › doEmbed › should extract embedding
    #[tokio::test]
    async fn should_extract_embedding() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embed"))
            .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
            .mount(&server)
            .await;

        let provider = create_cohere(CohereProviderSettings {
            api_key: Some("test-api-key".to_string()),
            base_url: Some(server.uri()),
            ..Default::default()
        })
        .unwrap();
        let model = provider.embedding_model("embed-english-v3.0").unwrap();

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

    /// upstream: @ai-sdk/cohere src/cohere-embedding-model.test.ts › doEmbed › should expose the raw response
    /// partial: `should expose the raw response`; full port in src/cohere/tests/embedding_model.rs.
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

        let provider = create_cohere(CohereProviderSettings {
            api_key: Some("test-api-key".to_string()),
            base_url: Some(server.uri()),
            ..Default::default()
        })
        .unwrap();
        let model = provider.embedding_model("embed-english-v3.0").unwrap();

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
        assert_eq!(
            result.response.unwrap().body,
            Some(embedding_response_body())
        );
    }

    /// upstream: @ai-sdk/cohere src/cohere-embedding-model.test.ts › doEmbed › should extract usage
    #[tokio::test]
    async fn should_extract_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embed"))
            .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
            .mount(&server)
            .await;

        let provider = create_cohere(CohereProviderSettings {
            api_key: Some("test-api-key".to_string()),
            base_url: Some(server.uri()),
            ..Default::default()
        })
        .unwrap();
        let model = provider.embedding_model("embed-english-v3.0").unwrap();

        let result = model
            .do_embed(&default_options(test_values()))
            .await
            .expect("should succeed");
        let usage = result.usage.expect("usage should be present");
        assert_eq!(usage.tokens, 10);
    }

    /// upstream: @ai-sdk/cohere src/cohere-embedding-model.test.ts › doEmbed › should pass the model and the values
    #[tokio::test]
    async fn should_pass_model_and_values() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embed"))
            .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
            .mount(&server)
            .await;

        let provider = create_cohere(CohereProviderSettings {
            api_key: Some("test-api-key".to_string()),
            base_url: Some(server.uri()),
            ..Default::default()
        })
        .unwrap();
        let model = provider.embedding_model("embed-english-v3.0").unwrap();

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

    /// upstream: @ai-sdk/cohere src/cohere-embedding-model.test.ts › doEmbed › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embed"))
            .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
            .mount(&server)
            .await;

        let provider = create_cohere(CohereProviderSettings {
            api_key: Some("test-api-key".to_string()),
            base_url: Some(server.uri()),
            headers: Some(HashMap::from([(
                "Custom-Provider-Header".to_string(),
                Some("provider-header-value".to_string()),
            )])),
            ..Default::default()
        })
        .unwrap();
        let model = provider.embedding_model("embed-english-v3.0").unwrap();

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
            h.get("custom-provider-header")
                .and_then(|v| v.to_str().ok()),
            Some("provider-header-value")
        );
        assert_eq!(
            h.get("authorization").and_then(|v| v.to_str().ok()),
            Some("Bearer test-api-key")
        );
        assert_eq!(
            h.get("custom-request-header").and_then(|v| v.to_str().ok()),
            Some("request-header-value")
        );
    }

    /// upstream: @ai-sdk/cohere src/cohere-embedding-model.test.ts › doEmbed › should expose the raw response
    #[tokio::test]
    async fn should_expose_the_raw_response() {
        let fetch = embedding_fetch_with(&[("test-header", "test-value")]);
        let mut call = options(json!({}));
        call.provider_options = None;
        let result = provider(&fetch)
            .embedding("embed-english-v3.0")
            .do_embed(&call)
            .await
            .unwrap();
        let snapshot: Value =
            serde_json::from_str(SNAPSHOT_DO_EMBED_RAW_RESPONSE).expect("snapshot is JSON");
        assert_eq!(
            serde_json::to_value(result.response.expect("response")).unwrap(),
            snapshot
        );
    }

    /// upstream: @ai-sdk/cohere src/cohere-embedding-model.test.ts › doEmbed › should pass the input_type setting
    #[tokio::test]
    async fn should_pass_the_input_type_setting() {
        let fetch = embedding_fetch();
        provider(&fetch)
            .embedding("embed-english-v3.0")
            .do_embed(&options(
                json!({"cohere": {"inputType": "search_document"}}),
            ))
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].json_body(),
            json!({
                "embedding_types": ["float"],
                "input_type": "search_document",
                "model": "embed-english-v3.0",
                "texts": ["sunny day at the beach", "rainy day in the city"]
            })
        );
    }

    /// upstream: @ai-sdk/cohere src/cohere-embedding-model.test.ts › doEmbed › should pass the output_dimension setting
    #[tokio::test]
    async fn should_pass_the_output_dimension_setting() {
        let fetch = embedding_fetch();
        provider(&fetch)
            .embedding("embed-v4.0")
            .do_embed(&options(json!({"cohere": {"outputDimension": 256}})))
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].json_body(),
            json!({
                "embedding_types": ["float"],
                "input_type": "search_query",
                "model": "embed-v4.0",
                "output_dimension": 256,
                "texts": ["sunny day at the beach", "rainy day in the city"]
            })
        );
    }
}
