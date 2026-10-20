//! Ports of `@ai-sdk/amazon-bedrock` 5.0.100
//! `src/amazon-bedrock-embedding-model.test.ts`.
//!
//! Upstream builds `AmazonBedrockEmbeddingModel` directly with a base URL,
//! plain config headers and `fetch: injectFetchHeaders(..)`, so no request is
//! signed. The port does the same: `BedrockEmbeddingModel::from_config` with a
//! fixed endpoint, the config headers and the
//! [`inject_fetch_headers`](super::support::inject_fetch_headers) transport over the
//! mock transport (which stands in for the test server).
//!
//! Two cases stay in `aimux-providers/tests/bedrock_embedding_test.rs`, whose
//! tests already cover them: "should expose model-specific max embeddings per
//! call" and "should handle single input value and extract usage".

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Value, json};

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt};

use crate::bedrock::{AmazonBedrockEmbeddingModelFamily, BedrockEmbeddingModel};
use crate::shared::{Endpoint, EndpointConfig, ResolveEndpoint};
use crate::test_support::mock_fetch::{Canned, MockFetch};

use super::support::inject_fetch_headers;

const BASE_URL: &str = "https://bedrock-runtime.us-east-1.amazonaws.com";

const TEST_VALUES: [&str; 2] = ["sunny day at the beach", "rainy day in the city"];

const COHERE_V4_APPLICATION_PROFILE_ARN: &str =
    "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/qibm5eutlkcy";

fn mock_embeddings() -> Vec<Vec<f32>> {
    vec![
        vec![-0.09, 0.05, -0.02, 0.01, 0.04],
        vec![-0.08, 0.06, -0.03, 0.02, 0.03],
    ]
}

/// The `/model/{id}/invoke` URL of `model_id` (`encodeURIComponent`).
fn invoke_url(model_id: &str) -> String {
    format!(
        "{BASE_URL}/model/{}/invoke",
        model_id.replace(':', "%3A").replace('/', "%2F")
    )
}

/// A fixed base URL and fixed headers: the `{ baseUrl, headers }` config of
/// the upstream model.
struct FixedEndpoint {
    base_url: String,
    headers: HeaderMapOpt,
}

#[async_trait]
impl ResolveEndpoint for FixedEndpoint {
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        Ok(Endpoint {
            base_url: self.base_url.clone(),
            headers: self.headers.clone(),
        })
    }
}

/// `new AmazonBedrockEmbeddingModel(model_id, { baseUrl, headers, fetch })`.
fn embedding_model(
    model_id: &str,
    headers: &[(&str, &str)],
    fetch: FetchFunction,
) -> BedrockEmbeddingModel {
    BedrockEmbeddingModel::from_config(
        model_id.to_string(),
        EndpointConfig::new(
            "amazon-bedrock".to_string(),
            FixedEndpoint {
                base_url: BASE_URL.to_string(),
                headers: headers
                    .iter()
                    .map(|(name, value)| ((*name).to_string(), Some((*value).to_string())))
                    .collect(),
            },
            Some(fetch),
        ),
    )
}

/// The upstream `mockConfigHeaders`.
const MOCK_CONFIG_HEADERS: &[(&str, &str)] = &[
    ("config-header", "config-value"),
    ("shared-header", "config-shared"),
];

/// `fakeFetchWithAuth` over `mock`.
fn fake_fetch_with_auth(mock: &std::sync::Arc<MockFetch>) -> FetchFunction {
    inject_fetch_headers(&[("x-amz-auth", "test-auth")], mock.transport())
}

/// A binary JSON response with extra headers.
fn json_response(body: &Value, headers: &[(&str, &str)]) -> Canned {
    let mut canned = Canned::json(body);
    canned.headers.extend(
        headers
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string())),
    );
    canned
}

/// The Titan response of the upstream `embedUrl`.
fn titan_response() -> Canned {
    json_response(
        &json!({ "embedding": mock_embeddings()[0], "inputTextTokenCount": 8 }),
        &[],
    )
}

fn options(values: &[&str]) -> EmbeddingCallOptions {
    EmbeddingCallOptions {
        values: values.iter().map(|value| (*value).to_string()).collect(),
        ..EmbeddingCallOptions::new("")
    }
}

fn provider_options(namespace: &str, options: Value) -> SharedProviderOptions {
    aimux_core::shared::provider_namespace(namespace, options).expect("options are an object")
}

mod do_embed {
    use super::*;

    // "should expose model-specific max embeddings per call": the marked test
    // in aimux-providers/tests/bedrock_embedding_test.rs.

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-embedding-model.test.ts › doEmbed › should handle single input value and return embeddings
    #[tokio::test]
    async fn should_handle_single_input_value_and_return_embeddings() {
        let mock = MockFetch::new(vec![titan_response()]);
        let model = embedding_model(
            "amazon.titan-embed-text-v2:0",
            MOCK_CONFIG_HEADERS,
            fake_fetch_with_auth(&mock),
        );

        let result = model
            .do_embed(&options(&[TEST_VALUES[0]]))
            .await
            .expect("embedding succeeds");

        assert_eq!(result.embeddings.len(), 1);
        assert_eq!(result.embeddings[0], vec![-0.09, 0.05, -0.02, 0.01, 0.04]);

        let seen = mock.seen();
        assert_eq!(seen[0].url, invoke_url("amazon.titan-embed-text-v2:0"));
        assert_eq!(seen[0].json_body(), json!({ "inputText": TEST_VALUES[0] }));
    }

    // "should handle single input value and extract usage": the marked test
    // in aimux-providers/tests/bedrock_embedding_test.rs.

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-embedding-model.test.ts › doEmbed › should support Cohere embedding models
    #[tokio::test]
    async fn should_support_cohere_embedding_models() {
        let mock = MockFetch::new(vec![json_response(
            &json!({ "embeddings": [mock_embeddings()[0]] }),
            &[("x-amzn-bedrock-input-token-count", "6")],
        )]);
        let cohere_model = embedding_model(
            "cohere.embed-english-v3",
            MOCK_CONFIG_HEADERS,
            fake_fetch_with_auth(&mock),
        );

        let result = cohere_model
            .do_embed(&options(&[TEST_VALUES[0]]))
            .await
            .expect("embedding succeeds");

        assert_eq!(result.embeddings.len(), 1);
        assert_eq!(result.embeddings[0], vec![-0.09, 0.05, -0.02, 0.01, 0.04]);
        assert_eq!(result.usage.expect("usage").tokens, 6);

        let seen = mock.seen();
        assert_eq!(seen[0].url, invoke_url("cohere.embed-english-v3"));
        assert_eq!(
            seen[0].json_body(),
            json!({ "input_type": "search_query", "texts": [TEST_VALUES[0]] })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-embedding-model.test.ts › doEmbed › should support Cohere v4 embedding models
    #[tokio::test]
    async fn should_support_cohere_v4_embedding_models() {
        let mock = MockFetch::new(vec![json_response(
            &json!({ "embeddings": { "float": [mock_embeddings()[0]] } }),
            &[("x-amzn-bedrock-input-token-count", "6")],
        )]);
        let cohere_v4_model = embedding_model(
            "cohere.embed-v4:0",
            MOCK_CONFIG_HEADERS,
            fake_fetch_with_auth(&mock),
        );

        let result = cohere_v4_model
            .do_embed(&options(&[TEST_VALUES[0]]))
            .await
            .expect("embedding succeeds");

        assert_eq!(result.embeddings.len(), 1);
        assert_eq!(result.embeddings[0], vec![-0.09, 0.05, -0.02, 0.01, 0.04]);
        assert_eq!(result.usage.expect("usage").tokens, 6);

        let seen = mock.seen();
        assert_eq!(seen[0].url, invoke_url("cohere.embed-v4:0"));
        assert_eq!(
            seen[0].json_body(),
            json!({ "input_type": "search_query", "texts": [TEST_VALUES[0]] })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-embedding-model.test.ts › doEmbed › should send multiple values for Cohere embedding models
    #[tokio::test]
    async fn should_send_multiple_values_for_cohere_embedding_models() {
        let mock = MockFetch::new(vec![json_response(
            &json!({ "embeddings": { "float": mock_embeddings() } }),
            &[("x-amzn-bedrock-input-token-count", "12")],
        )]);
        let cohere_v4_model = embedding_model(
            "cohere.embed-v4:0",
            MOCK_CONFIG_HEADERS,
            fake_fetch_with_auth(&mock),
        );

        let result = cohere_v4_model
            .do_embed(&options(&TEST_VALUES))
            .await
            .expect("embedding succeeds");

        assert_eq!(result.embeddings, mock_embeddings());
        assert_eq!(result.usage.expect("usage").tokens, 12);

        let seen = mock.seen();
        assert_eq!(seen[0].url, invoke_url("cohere.embed-v4:0"));
        assert_eq!(
            seen[0].json_body(),
            json!({ "input_type": "search_query", "texts": TEST_VALUES })
        );
    }

    // Not ported (sub-feature): "should return NaN usage for Cohere models
    // when the token count header is absent". `EmbeddingUsage::tokens` is a
    // `u32`, which has no NaN (or other "unknown") value; aimux reports 0.

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-embedding-model.test.ts › doEmbed › should support Cohere models behind cross-region inference profile ids
    #[tokio::test]
    async fn should_support_cohere_models_behind_cross_region_inference_profile_ids() {
        let mock = MockFetch::new(vec![json_response(
            &json!({ "embeddings": { "float": [mock_embeddings()[0]] } }),
            &[("x-amzn-bedrock-input-token-count", "6")],
        )]);
        let cohere_v4_us_profile_model = embedding_model(
            "us.cohere.embed-v4:0",
            MOCK_CONFIG_HEADERS,
            fake_fetch_with_auth(&mock),
        );

        let result = cohere_v4_us_profile_model
            .do_embed(&options(&[TEST_VALUES[0]]))
            .await
            .expect("embedding succeeds");

        assert_eq!(result.embeddings.len(), 1);
        assert_eq!(result.usage.expect("usage").tokens, 6);

        let seen = mock.seen();
        assert_eq!(seen[0].url, invoke_url("us.cohere.embed-v4:0"));
        assert_eq!(
            seen[0].json_body(),
            json!({ "input_type": "search_query", "texts": [TEST_VALUES[0]] })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-embedding-model.test.ts › doEmbed › should support Cohere models behind application inference profile ARNs
    #[tokio::test]
    async fn should_support_cohere_models_behind_application_inference_profile_arns() {
        let mock = MockFetch::new(vec![json_response(
            &json!({ "embeddings": { "float": mock_embeddings() } }),
            &[("x-amzn-bedrock-input-token-count", "12")],
        )]);
        let cohere_v4_application_profile_model = embedding_model(
            COHERE_V4_APPLICATION_PROFILE_ARN,
            MOCK_CONFIG_HEADERS,
            fake_fetch_with_auth(&mock),
        )
        .with_model_family(Some(AmazonBedrockEmbeddingModelFamily::Cohere));

        let result = cohere_v4_application_profile_model
            .do_embed(&options(&[TEST_VALUES[0]]))
            .await
            .expect("embedding succeeds");

        assert_eq!(result.embeddings, mock_embeddings());
        assert_eq!(result.usage.expect("usage").tokens, 12);

        let seen = mock.seen();
        assert_eq!(seen[0].url, invoke_url(COHERE_V4_APPLICATION_PROFILE_ARN));
        assert_eq!(
            seen[0].json_body(),
            json!({ "input_type": "search_query", "texts": [TEST_VALUES[0]] })
        );
        assert_eq!(
            cohere_v4_application_profile_model.max_embeddings_per_call(),
            Some(96)
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-embedding-model.test.ts › doEmbed › should pass outputDimension for Cohere v4 embedding models
    #[tokio::test]
    async fn should_pass_output_dimension_for_cohere_v4_embedding_models() {
        let mock = MockFetch::new(vec![json_response(
            &json!({ "embeddings": { "float": [mock_embeddings()[0]] } }),
            &[("x-amzn-bedrock-input-token-count", "6")],
        )]);
        let cohere_v4_model = embedding_model(
            "cohere.embed-v4:0",
            MOCK_CONFIG_HEADERS,
            fake_fetch_with_auth(&mock),
        );

        let result = cohere_v4_model
            .do_embed(&EmbeddingCallOptions {
                provider_options: Some(provider_options(
                    "bedrock",
                    json!({ "outputDimension": 256 }),
                )),
                ..options(&[TEST_VALUES[0]])
            })
            .await
            .expect("embedding succeeds");

        assert_eq!(result.embeddings.len(), 1);
        assert_eq!(result.embeddings[0], vec![-0.09, 0.05, -0.02, 0.01, 0.04]);

        let seen = mock.seen();
        assert_eq!(seen[0].url, invoke_url("cohere.embed-v4:0"));
        assert_eq!(
            seen[0].json_body(),
            json!({
                "input_type": "search_query",
                "texts": [TEST_VALUES[0]],
                "output_dimension": 256,
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-embedding-model.test.ts › doEmbed › should properly combine headers from all sources
    #[tokio::test]
    async fn should_properly_combine_headers_from_all_sources() {
        let mock = MockFetch::new(vec![titan_response()]);
        let model_with_headers = embedding_model(
            "amazon.titan-embed-text-v2:0",
            &[
                ("model-header", "model-value"),
                ("shared-header", "model-shared"),
            ],
            inject_fetch_headers(
                &[
                    ("signed-header", "signed-value"),
                    ("authorization", "AWS4-HMAC-SHA256..."),
                ],
                mock.transport(),
            ),
        );

        model_with_headers
            .do_embed(&EmbeddingCallOptions {
                headers: Some(HashMap::from([
                    ("options-header".to_string(), "options-value".to_string()),
                    ("shared-header".to_string(), "options-shared".to_string()),
                ])),
                ..options(&[TEST_VALUES[0]])
            })
            .await
            .expect("embedding succeeds");

        let request_headers = &mock.seen()[0].headers;
        assert_eq!(request_headers["options-header"], "options-value");
        assert_eq!(request_headers["model-header"], "model-value");
        assert_eq!(request_headers["signed-header"], "signed-value");
        assert_eq!(request_headers["authorization"], "AWS4-HMAC-SHA256...");
        assert_eq!(request_headers["shared-header"], "options-shared");
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-embedding-model.test.ts › doEmbed › should work with partial headers
    #[tokio::test]
    async fn should_work_with_partial_headers() {
        let mock = MockFetch::new(vec![titan_response()]);
        let model_with_partial_headers = embedding_model(
            "amazon.titan-embed-text-v2:0",
            &[("model-header", "model-value")],
            inject_fetch_headers(
                &[
                    ("signed-header", "signed-value"),
                    ("authorization", "AWS4-HMAC-SHA256..."),
                ],
                mock.transport(),
            ),
        );

        model_with_partial_headers
            .do_embed(&options(&[TEST_VALUES[0]]))
            .await
            .expect("embedding succeeds");

        let request_headers = &mock.seen()[0].headers;
        assert_eq!(request_headers["model-header"], "model-value");
        assert_eq!(request_headers["signed-header"], "signed-value");
        assert_eq!(request_headers["authorization"], "AWS4-HMAC-SHA256...");
    }
}

mod should_support_nova_embeddings {
    use super::*;

    const NOVA_MODEL_ID: &str = "amazon.nova-2-multimodal-embeddings-v1:0";

    fn nova_response() -> Canned {
        json_response(
            &json!({
                "embeddings": [{ "embeddingType": "TEXT", "embedding": mock_embeddings()[0] }],
                "inputTokenCount": 8,
            }),
            &[],
        )
    }

    fn nova_model(mock: &std::sync::Arc<MockFetch>) -> BedrockEmbeddingModel {
        embedding_model(
            NOVA_MODEL_ID,
            &[("config-header", "config-value")],
            fake_fetch_with_auth(mock),
        )
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-embedding-model.test.ts › should support Nova embeddings › should send SINGLE_EMBEDDING payload for Nova embeddings
    #[tokio::test]
    async fn should_send_single_embedding_payload_for_nova_embeddings() {
        let mock = MockFetch::new(vec![nova_response()]);

        let result = nova_model(&mock)
            .do_embed(&options(&[TEST_VALUES[0]]))
            .await
            .expect("embedding succeeds");

        assert_eq!(result.embeddings[0], vec![-0.09, 0.05, -0.02, 0.01, 0.04]);

        let seen = mock.seen();
        assert_eq!(seen[0].url, invoke_url(NOVA_MODEL_ID));
        assert_eq!(
            seen[0].json_body(),
            json!({
                "taskType": "SINGLE_EMBEDDING",
                "singleEmbeddingParams": {
                    "embeddingPurpose": "GENERIC_INDEX",
                    "embeddingDimension": 1024,
                    "text": {
                        "truncationMode": "END",
                        "value": TEST_VALUES[0],
                    },
                },
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-embedding-model.test.ts › should support Nova embeddings › should pass embeddingDimension for Nova embeddings
    #[tokio::test]
    async fn should_pass_embedding_dimension_for_nova_embeddings() {
        let mock = MockFetch::new(vec![nova_response()]);

        let result = nova_model(&mock)
            .do_embed(&EmbeddingCallOptions {
                provider_options: Some(provider_options(
                    "bedrock",
                    json!({ "embeddingDimension": 256 }),
                )),
                ..options(&[TEST_VALUES[0]])
            })
            .await
            .expect("embedding succeeds");

        assert_eq!(result.embeddings[0], vec![-0.09, 0.05, -0.02, 0.01, 0.04]);

        let seen = mock.seen();
        assert_eq!(seen[0].url, invoke_url(NOVA_MODEL_ID));
        assert_eq!(
            seen[0].json_body(),
            json!({
                "taskType": "SINGLE_EMBEDDING",
                "singleEmbeddingParams": {
                    "embeddingPurpose": "GENERIC_INDEX",
                    "embeddingDimension": 256,
                    "text": {
                        "truncationMode": "END",
                        "value": TEST_VALUES[0],
                    },
                },
            })
        );
    }
}

// ── Older ports (previously `tests/bedrock_embedding_test.rs`) ──
//
// Rust translations of the AI SDK Amazon Bedrock embedding model tests.
//
// Source: `reference/ai/packages/amazon-bedrock/src/amazon-bedrock-embedding-model.test.ts`
// (536 lines, 12 cases).
//
// Uses Bearer token auth to avoid SigV4 signing complexity in tests (the TS
// tests use a `fakeFetchWithAuth` that injects auth headers).

mod bedrock_embedding {
    use crate::bedrock::{
        AmazonBedrockProvider, AmazonBedrockProviderSettings, create_amazon_bedrock,
    };
    use aimux_core::embedding_model::EmbeddingCallOptions;
    use aimux_core::provider::Provider;
    use serde_json::{Value, json};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_VALUES: &[&str] = &["sunny day at the beach", "rainy day in the city"];

    fn mock_embeddings() -> Vec<Vec<f32>> {
        vec![
            vec![-0.09, 0.05, -0.02, 0.01, 0.04],
            vec![-0.08, 0.06, -0.03, 0.02, 0.03],
        ]
    }

    fn test_provider(base_url: String) -> AmazonBedrockProvider {
        create_amazon_bedrock(AmazonBedrockProviderSettings {
            base_url: Some(base_url),
            api_key: Some("test-auth".to_string().into()),
            region: Some("us-east-1".to_string()),
            ..Default::default()
        })
        .unwrap()
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

    /// partial: `should handle single input value and return embeddings`; full port in src/bedrock/tests/embedding_model.rs.
    #[tokio::test]
    async fn should_handle_titan_single_input() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/model/amazon.titan-embed-text-v2%3A0/invoke"))
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
        let model = provider
            .embedding_model("amazon.titan-embed-text-v2:0")
            .unwrap();

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

    /// partial: `should support Cohere embedding models`; full port in src/bedrock/tests/embedding_model.rs.
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
        let model = provider.embedding_model("cohere.embed-english-v3").unwrap();

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

    /// partial: `should support Cohere v4 embedding models`; full port in src/bedrock/tests/embedding_model.rs.
    #[tokio::test]
    async fn should_support_cohere_v4() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/model/cohere.embed-v4%3A0/invoke"))
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
        let model = provider.embedding_model("cohere.embed-v4:0").unwrap();

        let result = model
            .do_embed(&default_options(vec![TEST_VALUES[0].to_string()]))
            .await
            .expect("should succeed");

        assert_eq!(result.embeddings.len(), 1);
        assert_eq!(result.embeddings[0], mock_embeddings()[0]);
        assert_eq!(result.usage.unwrap().tokens, 6);
    }

    /// partial: `should send multiple values for Cohere embedding models`; full port in src/bedrock/tests/embedding_model.rs.
    #[tokio::test]
    async fn should_send_multiple_values_cohere_v4() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/model/cohere.embed-v4%3A0/invoke"))
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
        let model = provider.embedding_model("cohere.embed-v4:0").unwrap();

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

    /// partial: `should support Cohere models behind cross-region inference profile ids`; full port in src/bedrock/tests/embedding_model.rs.
    #[tokio::test]
    async fn should_support_cross_region_cohere() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/model/us.cohere.embed-v4%3A0/invoke"))
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
        let model = provider.embedding_model("us.cohere.embed-v4:0").unwrap();

        let result = model
            .do_embed(&default_options(vec![TEST_VALUES[0].to_string()]))
            .await
            .expect("should succeed");
        assert_eq!(result.embeddings.len(), 1);
        assert_eq!(result.usage.unwrap().tokens, 6);
    }

    /// partial: `should send SINGLE_EMBEDDING payload for Nova embeddings`; full port in src/bedrock/tests/embedding_model.rs.
    #[tokio::test]
    async fn should_support_nova_embeddings() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/model/amazon.nova-2-multimodal-embeddings-v1%3A0/invoke",
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
        let model = provider
            .embedding_model("amazon.nova-2-multimodal-embeddings-v1:0")
            .unwrap();

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

    mod do_embed {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-embedding-model.test.ts › doEmbed › should expose model-specific max embeddings per call
        #[test]
        fn should_expose_max_embeddings_per_call() {
            let provider = test_provider("https://example.com".to_string());

            let titan_model = provider
                .embedding_model("amazon.titan-embed-text-v2:0")
                .unwrap();
            assert_eq!(titan_model.max_embeddings_per_call(), Some(1));

            let cohere_model = provider.embedding_model("cohere.embed-english-v3").unwrap();
            assert_eq!(cohere_model.max_embeddings_per_call(), Some(96));

            let cohere_v4_us_model = provider.embedding_model("us.cohere.embed-v4:0").unwrap();
            assert_eq!(cohere_v4_us_model.max_embeddings_per_call(), Some(96));

            let nova_model = provider
                .embedding_model("amazon.nova-2-multimodal-embeddings-v1:0")
                .unwrap();
            assert_eq!(nova_model.max_embeddings_per_call(), Some(1));
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-embedding-model.test.ts › doEmbed › should handle single input value and extract usage
        #[tokio::test]
        async fn should_extract_titan_usage() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/model/amazon.titan-embed-text-v2%3A0/invoke"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "embedding": mock_embeddings()[0],
                    "inputTextTokenCount": 8
                })))
                .mount(&server)
                .await;

            let provider = test_provider(server.uri());
            let model = provider
                .embedding_model("amazon.titan-embed-text-v2:0")
                .unwrap();

            let result = model
                .do_embed(&default_options(vec![TEST_VALUES[0].to_string()]))
                .await
                .expect("should succeed");

            let usage = result.usage.expect("usage present");
            assert_eq!(usage.tokens, 8);
        }
    }
}

// ── Older ports (previously `tests/bedrock_upstream_e2e_test.rs`) ──

mod bedrock_upstream_e2e {
    use crate::bedrock::tests::upstream_e2e_support::*;

    use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};

    use crate::test_support::mock_fetch::{Canned, MockFetch};
    use serde_json::json;

    /// partial: `should handle single input value and return embeddings`; full port in src/bedrock/tests/embedding_model.rs.
    #[tokio::test]
    async fn embed_single_value_and_usage() {
        let response = json!({"embedding":[-0.09,0.05,-0.02,0.01,0.04],"inputTextTokenCount":8});
        let fetch = MockFetch::new(vec![Canned::json(&response)]);
        let result = provider(&fetch)
            .embedding("amazon.titan-embed-text-v2:0")
            .do_embed(&EmbeddingCallOptions::new("sunny day at the beach"))
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].url,
            "https://bedrock.test/model/amazon.titan-embed-text-v2%3A0/invoke"
        );
        assert_eq!(
            fetch.seen()[0].json_body(),
            json!({"inputText":"sunny day at the beach"})
        );
        assert_eq!(
            result.embeddings,
            vec![vec![-0.09, 0.05, -0.02, 0.01, 0.04]]
        );
        assert_eq!(result.usage.unwrap().tokens, 8);
        assert!(result.response.is_none());
    }
}
