use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable};

use crate::shared::{Endpoint, EndpointConfig, ResolveEndpoint};
use crate::test_support::mock_fetch::{Canned, MockFetch, RouteFetch};
use crate::vertex::{VertexEmbeddingModel, VertexProviderSettings, create_google_vertex};

const DEFAULT_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/test-project/locations/us-central1/publishers/google/models/textembedding-gecko@001:predict";
const GEMINI_EMBEDDING_2_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/test-project/locations/us-central1/publishers/google/models/gemini-embedding-2:embedContent";
const CUSTOM_URL: &str = "https://custom-endpoint.com/models/textembedding-gecko@001:predict";
const BASE_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/test-project/locations/us-central1/publishers/google";
const MOCK_MODEL_ID: &str = "textembedding-gecko@001";
const TEST_VALUES: [&str; 2] = ["test text one", "test text two"];

/// The pinned user-agent part of the package (upstream mocks `0.0.0-test`).
const USER_AGENT_PART: &str = "ai-sdk-google-vertex/5.0.100";

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/google-vertex-embedding.json")).unwrap()
}

/// A base URL and headers fixed for every request (upstream's `mockConfig`).
struct StaticEndpoint {
    base_url: String,
    headers: HeaderMapOpt,
}

#[async_trait::async_trait]
impl ResolveEndpoint for StaticEndpoint {
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        Ok(Endpoint {
            base_url: self.base_url.clone(),
            headers: self.headers.clone(),
        })
    }
}

/// `new GoogleVertexEmbeddingModel(modelId, { provider: 'google-vertex', baseURL, headers: () => ({}), fetch })`.
fn model_with(model_id: &str, base_url: &str, fetch: FetchFunction) -> VertexEmbeddingModel {
    VertexEmbeddingModel::from_config(
        model_id.to_string(),
        EndpointConfig::new(
            "google-vertex".to_string(),
            StaticEndpoint {
                base_url: base_url.to_string(),
                headers: HeaderMapOpt::new(),
            },
            Some(fetch),
        ),
    )
}

/// A JSON response as the AI SDK test server sends it (compact body,
/// `content-type` and `content-length`).
fn json_response(body: &Value, headers: &[(&str, &str)]) -> Canned {
    let body = serde_json::to_vec(body).unwrap();
    let mut all = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("content-length".to_string(), body.len().to_string()),
    ];
    all.extend(headers.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    Canned {
        status: 200,
        headers: all,
        body,
    }
}

/// The AI SDK test server: each URL answers its response, anything else 404.
fn test_server(routes: Vec<(&'static str, Canned)>) -> Arc<RouteFetch> {
    RouteFetch::new(move |seen| {
        routes
            .iter()
            .find(|(url, _)| *url == seen.url)
            .map(|(_, canned)| canned.clone())
            .unwrap_or(Canned {
                status: 404,
                headers: Vec::new(),
                body: b"Not found".to_vec(),
            })
    })
}

/// `prepareJsonFixtureResponse('google-vertex-embedding', headers)`.
fn fixture_server(headers: &[(&str, &str)]) -> Arc<RouteFetch> {
    test_server(vec![(DEFAULT_URL, json_response(&fixture(), headers))])
}

fn mock_provider_options() -> Value {
    json!({
        "outputDimensionality": 768,
        "taskType": "SEMANTIC_SIMILARITY",
        "title": "test title",
        "autoTruncate": false,
    })
}

fn options(values: &[&str], provider_options: Value) -> EmbeddingCallOptions {
    let mut options = EmbeddingCallOptions::new(values[0]);
    options.values = values.iter().map(ToString::to_string).collect();
    options.provider_options =
        Some(serde_json::from_value::<SharedProviderOptions>(provider_options).unwrap());
    options
}

/// aimux embeddings are `f32`: compares them with the expected JSON numbers
/// narrowed to `f32`.
fn assert_embeddings(actual: &[Vec<f32>], expected: Value) {
    let expected: Vec<Vec<f32>> = expected
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row.as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap() as f32)
                .collect()
        })
        .collect();
    assert_eq!(actual, expected);
}

fn fixture_embeddings() -> Value {
    json!([
        [
            -0.017999587580561638,
            -0.006893285550177097,
            -0.036766719073057175,
            -0.017558680847287178,
            -0.019938766956329346,
        ],
        [
            -0.06007182598114014,
            0.004907649010419846,
            -0.00690646655857563,
            -0.007314121350646019,
            -0.048464205116033554,
        ],
    ])
}

fn mock_options_body() -> Value {
    json!({
        "instances": [
            {"content": "test text one", "task_type": "SEMANTIC_SIMILARITY", "title": "test title"},
            {"content": "test text two", "task_type": "SEMANTIC_SIMILARITY", "title": "test title"},
        ],
        "parameters": {"autoTruncate": false, "outputDimensionality": 768},
    })
}

mod google_vertex_embedding_model {
    use super::*;

    mod embedding {
        use super::*;

        /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › embedding › should extract embeddings
        #[tokio::test]
        async fn should_extract_embeddings() {
            let server = fixture_server(&[]);
            let model = model_with(MOCK_MODEL_ID, BASE_URL, server.transport());
            let result = model
                .do_embed(&options(
                    &TEST_VALUES,
                    json!({"google": mock_provider_options()}),
                ))
                .await
                .unwrap();
            assert_embeddings(&result.embeddings, fixture_embeddings());
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › embedding › should return full result snapshot
        #[tokio::test]
        async fn should_return_full_result_snapshot() {
            let server = fixture_server(&[]);
            let model = model_with(MOCK_MODEL_ID, BASE_URL, server.transport());
            let result = model
                .do_embed(&options(
                    &TEST_VALUES,
                    json!({"google": mock_provider_options()}),
                ))
                .await
                .unwrap();
            let mut snapshot: Value =
                serde_json::from_str(include_str!("snapshots/embedding_full_result.json")).unwrap();
            assert_embeddings(&result.embeddings, snapshot["embeddings"].take());
            let mut actual = serde_json::to_value(&result).unwrap();
            actual["embeddings"] = Value::Null;
            assert_eq!(actual, snapshot);
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › embedding › should extract usage
        #[tokio::test]
        async fn should_extract_usage() {
            let server = fixture_server(&[]);
            let model = model_with(MOCK_MODEL_ID, BASE_URL, server.transport());
            let result = model
                .do_embed(&options(
                    &TEST_VALUES,
                    json!({"google": mock_provider_options()}),
                ))
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(result.usage).unwrap(),
                json!({"tokens": 11})
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › embedding › should pass the model parameters correctly
        #[tokio::test]
        async fn should_pass_the_model_parameters_correctly() {
            let server = fixture_server(&[]);
            let model = model_with(MOCK_MODEL_ID, BASE_URL, server.transport());
            model
                .do_embed(&options(
                    &TEST_VALUES,
                    json!({"google": mock_provider_options()}),
                ))
                .await
                .unwrap();
            assert_eq!(server.seen()[0].json_body(), mock_options_body());
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › embedding › should accept googleVertex as provider options key
        #[tokio::test]
        async fn should_accept_google_vertex_as_provider_options_key() {
            let server = fixture_server(&[]);
            let model = model_with(MOCK_MODEL_ID, BASE_URL, server.transport());
            model
                .do_embed(&options(
                    &TEST_VALUES,
                    json!({"googleVertex": mock_provider_options()}),
                ))
                .await
                .unwrap();
            assert_eq!(server.seen()[0].json_body(), mock_options_body());
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › embedding › should pass the taskType setting in instances
        #[tokio::test]
        async fn should_pass_the_task_type_setting_in_instances() {
            let server = fixture_server(&[]);
            let model = model_with(MOCK_MODEL_ID, BASE_URL, server.transport());
            model
                .do_embed(&options(
                    &TEST_VALUES,
                    json!({"google": {"taskType": "SEMANTIC_SIMILARITY"}}),
                ))
                .await
                .unwrap();
            assert_eq!(
                server.seen()[0].json_body(),
                json!({
                    "instances": [
                        {"content": "test text one", "task_type": "SEMANTIC_SIMILARITY"},
                        {"content": "test text two", "task_type": "SEMANTIC_SIMILARITY"},
                    ],
                    "parameters": {},
                })
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › embedding › should pass the title setting in instances
        #[tokio::test]
        async fn should_pass_the_title_setting_in_instances() {
            let server = fixture_server(&[]);
            let model = model_with(MOCK_MODEL_ID, BASE_URL, server.transport());
            model
                .do_embed(&options(
                    &TEST_VALUES,
                    json!({"google": {"title": "test title"}}),
                ))
                .await
                .unwrap();
            assert_eq!(
                server.seen()[0].json_body(),
                json!({
                    "instances": [
                        {"content": "test text one", "title": "test title"},
                        {"content": "test text two", "title": "test title"},
                    ],
                    "parameters": {},
                })
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › embedding › should use embedContent for gemini-embedding-2
        #[tokio::test]
        async fn should_use_embed_content_for_gemini_embedding_2() {
            let server = test_server(vec![
                (DEFAULT_URL, json_response(&fixture(), &[])),
                (
                    GEMINI_EMBEDDING_2_URL,
                    json_response(
                        &json!({
                            "embedding": {"values": [0.1, 0.2, 0.3]},
                            "usageMetadata": {"promptTokenCount": 4},
                        }),
                        &[],
                    ),
                ),
            ]);
            let model = model_with("gemini-embedding-2", BASE_URL, server.transport());
            let result = model
                .do_embed(&options(
                    &["test text one"],
                    json!({"google": mock_provider_options()}),
                ))
                .await
                .unwrap();
            let seen = server.seen();
            assert_eq!(seen[0].url, GEMINI_EMBEDDING_2_URL);
            assert_eq!(
                seen[0].json_body(),
                json!({
                    "content": {"parts": [{"text": "test text one"}]},
                    "embedContentConfig": {
                        "autoTruncate": false,
                        "outputDimensionality": 768,
                        "taskType": "SEMANTIC_SIMILARITY",
                        "title": "test title",
                    },
                })
            );
            assert_embeddings(&result.embeddings, json!([[0.1, 0.2, 0.3]]));
            assert_eq!(
                serde_json::to_value(result.usage).unwrap(),
                json!({"tokens": 4})
            );
        }

        mod max_embeddings_per_call {
            use super::*;

            /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › embedding › maxEmbeddingsPerCall › should limit predict endpoint models to 250 values per call
            #[test]
            fn should_limit_predict_endpoint_models_to_250_values_per_call() {
                let model = model_with(
                    "gemini-embedding-001",
                    BASE_URL,
                    MockFetch::new(Vec::new()).transport(),
                );
                assert_eq!(model.max_embeddings_per_call(), Some(250));
            }

            /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › embedding › maxEmbeddingsPerCall › should limit %s to one value per call
            #[test]
            fn should_limit_model_to_one_value_per_call() {
                for model_id in ["gemini-embedding-2", "gemini-embedding-2-preview"] {
                    let model =
                        model_with(model_id, BASE_URL, MockFetch::new(Vec::new()).transport());
                    assert_eq!(model.max_embeddings_per_call(), Some(1), "{model_id}");
                }
            }
        }
    }

    mod response_headers {
        use super::*;

        /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › response headers › should expose response headers
        #[tokio::test]
        async fn should_expose_response_headers() {
            let server = fixture_server(&[("test-header", "test-value")]);
            let model = model_with(MOCK_MODEL_ID, BASE_URL, server.transport());
            let result = model
                .do_embed(&options(
                    &TEST_VALUES,
                    json!({"google": mock_provider_options()}),
                ))
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(result.response.unwrap().headers).unwrap(),
                json!({
                    "content-length": "429",
                    "content-type": "application/json",
                    "test-header": "test-value",
                })
            );
        }
    }

    mod response_metadata {
        use super::*;

        /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › response metadata › should expose the raw response
        #[tokio::test]
        async fn should_expose_the_raw_response() {
            let server = fixture_server(&[("test-header", "test-value")]);
            let model = model_with(MOCK_MODEL_ID, BASE_URL, server.transport());
            let result = model
                .do_embed(&options(
                    &TEST_VALUES,
                    json!({"google": mock_provider_options()}),
                ))
                .await
                .unwrap();
            let snapshot: Value =
                serde_json::from_str(include_str!("snapshots/embedding_raw_response.json"))
                    .unwrap();
            assert_eq!(serde_json::to_value(result.response).unwrap(), snapshot);
        }
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › should pass headers correctly
    #[tokio::test]
    async fn should_pass_headers_correctly() {
        let server = fixture_server(&[]);
        // aimux has one entry point, and it authenticates: an access token
        // stands in for upstream's credential-less base provider, and its
        // `authorization` header is left out of the comparison.
        let provider = create_google_vertex(VertexProviderSettings {
            project: Some("test-project".into()),
            location: Some("us-central1".into()),
            access_token: Some(Resolvable::Value("test-token".into())),
            headers: Some(Resolvable::Value(
                [(
                    "X-Custom-Header".to_string(),
                    Some("custom-value".to_string()),
                )]
                .into(),
            )),
            fetch: Some(server.transport()),
            ..Default::default()
        })
        .unwrap();
        let mut call = options(&TEST_VALUES, json!({"google": mock_provider_options()}));
        call.headers = Some([("X-Request-Header".to_string(), "request-value".to_string())].into());
        provider
            .embedding(MOCK_MODEL_ID)
            .do_embed(&call)
            .await
            .unwrap();
        let seen = server.seen();
        let mut headers: BTreeMap<String, String> = seen[0].headers.clone();
        let user_agent = headers.remove("user-agent").unwrap_or_default();
        headers.remove("authorization");
        assert_eq!(
            serde_json::to_value(headers).unwrap(),
            json!({
                "content-type": "application/json",
                "x-custom-header": "custom-value",
                "x-request-header": "request-value",
            })
        );
        assert!(user_agent.contains(USER_AGENT_PART), "{user_agent}");
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › should throw TooManyEmbeddingValuesForCallError when too many values provided
    #[tokio::test]
    async fn should_throw_too_many_embedding_values_for_call_error() {
        let server = fixture_server(&[]);
        let model = model_with(MOCK_MODEL_ID, BASE_URL, server.transport());
        let too_many_values = vec!["test"; 251];
        let error = model
            .do_embed(&options(
                &too_many_values,
                json!({"google": mock_provider_options()}),
            ))
            .await
            .unwrap_err();
        // aimux reports `TooManyEmbeddingValuesForCallError` as an invalid
        // argument with the same meaning.
        assert!(
            matches!(&error, AiMuxError::InvalidArgument { message, .. }
                if message.starts_with("Too many embedding values")),
            "{error:?}"
        );
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › should use custom baseURL when provided
    #[tokio::test]
    async fn should_use_custom_base_url_when_provided() {
        let server = test_server(vec![(CUSTOM_URL, json_response(&fixture(), &[]))]);
        let model = model_with(
            "textembedding-gecko@001",
            "https://custom-endpoint.com",
            server.transport(),
        );
        let result = model
            .do_embed(&options(
                &TEST_VALUES,
                json!({"google": {"outputDimensionality": 768}}),
            ))
            .await
            .unwrap();
        assert_embeddings(&result.embeddings, fixture_embeddings());
        assert_eq!(
            server.seen()[0].url,
            "https://custom-endpoint.com/models/textembedding-gecko@001:predict"
        );
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-embedding-model.test.ts › GoogleVertexEmbeddingModel › should use custom fetch when provided and include proper request content
    #[tokio::test]
    async fn should_use_custom_fetch_when_provided_and_include_proper_request_content() {
        let custom_fetch = MockFetch::new(vec![Canned {
            status: 200,
            headers: Vec::new(),
            body: serde_json::to_vec(&fixture()).unwrap(),
        }]);
        let model = model_with(
            "textembedding-gecko@001",
            "https://custom-endpoint.com",
            custom_fetch.transport(),
        );
        let result = model
            .do_embed(&options(
                &TEST_VALUES,
                json!({"google": {"outputDimensionality": 768}}),
            ))
            .await
            .unwrap();
        assert_embeddings(&result.embeddings, fixture_embeddings());
        let seen = custom_fetch.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].url, CUSTOM_URL);
        assert_eq!(
            seen[0].json_body(),
            json!({
                "instances": [
                    {"content": "test text one"},
                    {"content": "test text two"},
                ],
                "parameters": {"outputDimensionality": 768},
            })
        );
    }
}
