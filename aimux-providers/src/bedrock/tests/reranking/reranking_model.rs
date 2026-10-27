//! Ports of `@ai-sdk/amazon-bedrock` 5.0.100
//! `src/reranking/amazon-bedrock-reranking-model.test.ts`.
//!
//! Upstream builds `AmazonBedrockRerankingModel` directly with a base URL, a
//! region, plain config headers and `fetch: injectFetchHeaders(..)`, so no
//! request is signed. The port does the same: `BedrockRerankingModel::from_config`
//! with a fixed endpoint, the config headers, an `Auth` that names only the
//! region, and the `inject_fetch_headers` transport over the mock transport
//! (which stands in for the test server). Each upstream `beforeEach` request is
//! made by the test itself.
//!
//! The test server's recorded request headers leave out `user-agent`; it is
//! compared separately.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::reranking_model::{
    RerankingCallOptions, RerankingDocuments, RerankingModel, RerankingResult,
};
use aimux_provider_utils::HeaderMapOpt;

use crate::bedrock::BedrockRerankingModel;
use crate::bedrock::sigv4_fetch::Auth;
use crate::shared::{Endpoint, EndpointConfig, ResolveEndpoint};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

use super::super::support::{BEDROCK_USER_AGENT_PART, inject_fetch_headers};

const RERANK_URL: &str = "https://bedrock-agent-runtime.us-east-1.amazonaws.com/rerank";

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

/// The upstream `model`: `cohere.rerank-v3-5:0` on the us-east-1 Agent
/// Runtime, region `us-west-2`, with `fakeFetchWithAuth`.
fn model(mock: &Arc<MockFetch>) -> BedrockRerankingModel {
    BedrockRerankingModel::from_config(
        "cohere.rerank-v3-5:0".to_string(),
        EndpointConfig::new(
            "amazon-bedrock".to_string(),
            FixedEndpoint {
                base_url: "https://bedrock-agent-runtime.us-east-1.amazonaws.com".to_string(),
                headers: HeaderMapOpt::from([
                    (
                        "config-header".to_string(),
                        Some("config-value".to_string()),
                    ),
                    (
                        "shared-header".to_string(),
                        Some("config-shared".to_string()),
                    ),
                ]),
            },
            Some(inject_fetch_headers(
                &[("x-amz-auth", "test-auth")],
                mock.transport(),
            )),
        ),
        Arc::new(Auth {
            region: Some("us-west-2".to_string()),
            api_key: None,
            access_key_id: None,
            secret_access_key: None,
            session_token: None,
            credential_provider: None,
        }),
    )
}

/// `prepareJsonFixtureResponse(filename)`.
fn prepare_json_fixture_response() -> Canned {
    let body = include_str!("fixtures/amazon-bedrock-reranking.1.json");
    Canned {
        status: 200,
        headers: vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("content-length".to_string(), body.len().to_string()),
        ],
        body: body.as_bytes().to_vec(),
    }
}

/// The `beforeEach` call of a describe: rerank `documents` for `rainy day`
/// with the upstream provider options.
async fn do_rerank(documents: RerankingDocuments) -> (RerankingResult, Seen) {
    let mock = MockFetch::new(vec![prepare_json_fixture_response()]);
    let result = model(&mock)
        .do_rerank(&RerankingCallOptions {
            top_n: Some(2),
            provider_options: Some(
                aimux_core::shared::provider_namespace(
                    "bedrock",
                    json!({
                        "nextToken": "test-token",
                        "additionalModelRequestFields": { "test": "test-value" },
                    }),
                )
                .expect("options are an object"),
            ),
            ..RerankingCallOptions::new("rainy day", documents)
        })
        .await
        .expect("reranking succeeds");
    let mut seen = mock.seen();
    assert_eq!(seen.len(), 1);
    let seen = seen.remove(0);
    assert_eq!(seen.url, RERANK_URL);
    (result, seen)
}

/// The request headers the test server records: all but `user-agent`, which
/// is checked to carry the package part.
fn recorded_headers(seen: &Seen) -> BTreeMap<String, String> {
    let mut headers = seen.headers.clone();
    let user_agent = headers.remove("user-agent").expect("user-agent is sent");
    assert!(
        user_agent
            .split(' ')
            .any(|part| part == BEDROCK_USER_AGENT_PART),
        "user-agent {user_agent} lacks {BEDROCK_USER_AGENT_PART}"
    );
    headers
}

fn expected_ranking() -> Value {
    json!([
        { "index": 0, "relevanceScore": 0.5110583305358887 },
        { "index": 5, "relevanceScore": 0.30241215229034424 },
    ])
}

fn expected_response() -> Value {
    json!({
        "body": {
            "results": [
                { "index": 0, "relevanceScore": 0.5110583305358887 },
                { "index": 5, "relevanceScore": 0.30241215229034424 },
            ],
        },
        "headers": {
            "content-length": "171",
            "content-type": "application/json",
        },
    })
}

fn expected_headers() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("config-header".to_string(), "config-value".to_string()),
        ("content-type".to_string(), "application/json".to_string()),
        ("shared-header".to_string(), "config-shared".to_string()),
        ("x-amz-auth".to_string(), "test-auth".to_string()),
    ])
}

/// The request body of both describes, with `sources`.
fn expected_body(sources: Value) -> Value {
    json!({
        "nextToken": "test-token",
        "queries": [
            {
                "textQuery": { "text": "rainy day" },
                "type": "TEXT",
            },
        ],
        "rerankingConfiguration": {
            "bedrockRerankingConfiguration": {
                "modelConfiguration": {
                    "additionalModelRequestFields": { "test": "test-value" },
                    "modelArn": "arn:aws:bedrock:us-west-2::foundation-model/cohere.rerank-v3-5:0",
                },
                "numberOfResults": 2,
            },
            "type": "BEDROCK_RERANKING_MODEL",
        },
        "sources": sources,
    })
}

mod do_rerank {
    use super::*;

    mod json_documents {
        use super::*;

        fn documents() -> RerankingDocuments {
            RerankingDocuments::Object {
                values: vec![
                    json!({ "example": "sunny day at the beach" }),
                    json!({ "example": "rainy day in the city" }),
                ],
            }
        }

        /// upstream: @ai-sdk/amazon-bedrock src/reranking/amazon-bedrock-reranking-model.test.ts › doRerank › json documents › should send request with stringified json documents
        #[tokio::test]
        async fn should_send_request_with_stringified_json_documents() {
            let (_result, seen) = do_rerank(documents()).await;

            assert_eq!(
                seen.json_body(),
                expected_body(json!([
                    {
                        "inlineDocumentSource": {
                            "jsonDocument": { "example": "sunny day at the beach" },
                            "type": "JSON",
                        },
                        "type": "INLINE",
                    },
                    {
                        "inlineDocumentSource": {
                            "jsonDocument": { "example": "rainy day in the city" },
                            "type": "JSON",
                        },
                        "type": "INLINE",
                    },
                ]))
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/reranking/amazon-bedrock-reranking-model.test.ts › doRerank › json documents › should send the AWS-required `bedrockRerankingConfiguration` wire key
        #[tokio::test]
        async fn should_send_the_aws_required_bedrock_reranking_configuration_wire_key() {
            let (_result, seen) = do_rerank(documents()).await;

            let body = seen.json_body();
            assert!(
                body.pointer("/rerankingConfiguration/bedrockRerankingConfiguration")
                    .is_some(),
                "{body}"
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/reranking/amazon-bedrock-reranking-model.test.ts › doRerank › json documents › should send request with the correct headers
        #[tokio::test]
        async fn should_send_request_with_the_correct_headers() {
            let (_result, seen) = do_rerank(documents()).await;

            assert_eq!(recorded_headers(&seen), expected_headers());
        }

        /// upstream: @ai-sdk/amazon-bedrock src/reranking/amazon-bedrock-reranking-model.test.ts › doRerank › json documents › should return result with warnings
        #[tokio::test]
        async fn should_return_result_with_warnings() {
            let (result, _seen) = do_rerank(documents()).await;

            assert!(result.warnings.is_none(), "{:?}", result.warnings);
        }

        /// upstream: @ai-sdk/amazon-bedrock src/reranking/amazon-bedrock-reranking-model.test.ts › doRerank › json documents › should return result with the correct ranking
        #[tokio::test]
        async fn should_return_result_with_the_correct_ranking() {
            let (result, _seen) = do_rerank(documents()).await;

            assert_eq!(
                serde_json::to_value(&result.ranking).expect("ranking serializes"),
                expected_ranking()
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/reranking/amazon-bedrock-reranking-model.test.ts › doRerank › json documents › should not return provider metadata (use response body instead)
        #[tokio::test]
        async fn should_not_return_provider_metadata_use_response_body_instead() {
            let (result, _seen) = do_rerank(documents()).await;

            assert!(
                result.provider_metadata.is_none(),
                "{:?}",
                result.provider_metadata
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/reranking/amazon-bedrock-reranking-model.test.ts › doRerank › json documents › should return result with the correct response
        #[tokio::test]
        async fn should_return_result_with_the_correct_response() {
            let (result, _seen) = do_rerank(documents()).await;

            assert_eq!(
                serde_json::to_value(&result.response).expect("response serializes"),
                expected_response()
            );
        }
    }

    mod text_documents {
        use super::*;

        fn documents() -> RerankingDocuments {
            RerankingDocuments::Text {
                values: vec![
                    "sunny day at the beach".to_string(),
                    "rainy day in the city".to_string(),
                ],
            }
        }

        /// upstream: @ai-sdk/amazon-bedrock src/reranking/amazon-bedrock-reranking-model.test.ts › doRerank › text documents › should send request with text documents
        #[tokio::test]
        async fn should_send_request_with_text_documents() {
            let (_result, seen) = do_rerank(documents()).await;

            assert_eq!(
                seen.json_body(),
                expected_body(json!([
                    {
                        "inlineDocumentSource": {
                            "textDocument": { "text": "sunny day at the beach" },
                            "type": "TEXT",
                        },
                        "type": "INLINE",
                    },
                    {
                        "inlineDocumentSource": {
                            "textDocument": { "text": "rainy day in the city" },
                            "type": "TEXT",
                        },
                        "type": "INLINE",
                    },
                ]))
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/reranking/amazon-bedrock-reranking-model.test.ts › doRerank › text documents › should send request with the correct headers
        #[tokio::test]
        async fn should_send_request_with_the_correct_headers() {
            let (_result, seen) = do_rerank(documents()).await;

            assert_eq!(recorded_headers(&seen), expected_headers());
        }

        /// upstream: @ai-sdk/amazon-bedrock src/reranking/amazon-bedrock-reranking-model.test.ts › doRerank › text documents › should return result without warnings
        #[tokio::test]
        async fn should_return_result_without_warnings() {
            let (result, _seen) = do_rerank(documents()).await;

            assert!(result.warnings.is_none(), "{:?}", result.warnings);
        }

        /// upstream: @ai-sdk/amazon-bedrock src/reranking/amazon-bedrock-reranking-model.test.ts › doRerank › text documents › should return result with the correct ranking
        #[tokio::test]
        async fn should_return_result_with_the_correct_ranking() {
            let (result, _seen) = do_rerank(documents()).await;

            assert_eq!(
                serde_json::to_value(&result.ranking).expect("ranking serializes"),
                expected_ranking()
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/reranking/amazon-bedrock-reranking-model.test.ts › doRerank › text documents › should not return provider metadata (use response body instead)
        #[tokio::test]
        async fn should_not_return_provider_metadata_use_response_body_instead() {
            let (result, _seen) = do_rerank(documents()).await;

            assert!(
                result.provider_metadata.is_none(),
                "{:?}",
                result.provider_metadata
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/reranking/amazon-bedrock-reranking-model.test.ts › doRerank › text documents › should return result with the correct response
        #[tokio::test]
        async fn should_return_result_with_the_correct_response() {
            let (result, _seen) = do_rerank(documents()).await;

            assert_eq!(
                serde_json::to_value(&result.response).expect("response serializes"),
                expected_response()
            );
        }
    }
}

// ── Older ports (previously `tests/bedrock_upstream_e2e_test.rs`) ──

mod bedrock_upstream_e2e {
    use crate::bedrock::tests::upstream_e2e_support::*;

    use aimux_core::reranking_model::{RerankingCallOptions, RerankingDocuments, RerankingModel};

    use crate::test_support::mock_fetch::{Canned, MockFetch};
    use serde_json::json;

    /// partial: `should send request with stringified json documents`; full port in src/bedrock/tests/reranking/reranking_model.rs.
    #[tokio::test]
    async fn rerank_json_documents_and_parse_ranking() {
        let response = json!({"results":[{"index":0,"relevanceScore":0.5110583305358887},{"index":5,"relevanceScore":0.30241215229034424}]});
        let fetch = MockFetch::new(vec![Canned::json(&response)]);
        let mut opts = RerankingCallOptions::new(
            "rainy day",
            RerankingDocuments::Object {
                values: vec![
                    json!({"example":"sunny day at the beach"}),
                    json!({"example":"rainy day in the city"}),
                ],
            },
        );
        opts.top_n = Some(2);
        opts.provider_options = Some(serde_json::from_value(json!({"bedrock":{"nextToken":"test-token","additionalModelRequestFields":{"test":"test-value"}}})).unwrap());
        let result = provider(&fetch)
            .reranking("cohere.rerank-v3-5:0")
            .do_rerank(&opts)
            .await
            .unwrap();
        assert_eq!(fetch.seen()[0].url, "https://bedrock.test/rerank");
        assert_eq!(
            fetch.seen()[0].json_body(),
            json!({"nextToken":"test-token","queries":[{"textQuery":{"text":"rainy day"},"type":"TEXT"}],"rerankingConfiguration":{"bedrockRerankingConfiguration":{"modelConfiguration":{"modelArn":"arn:aws:bedrock:us-west-2::foundation-model/cohere.rerank-v3-5:0","additionalModelRequestFields":{"test":"test-value"}},"numberOfResults":2},"type":"BEDROCK_RERANKING_MODEL"},"sources":[{"type":"INLINE","inlineDocumentSource":{"type":"JSON","jsonDocument":{"example":"sunny day at the beach"}}},{"type":"INLINE","inlineDocumentSource":{"type":"JSON","jsonDocument":{"example":"rainy day in the city"}}}]})
        );
        assert_eq!(result.ranking.len(), 2);
        assert_eq!(result.ranking[0].index, 0);
        assert_eq!(result.ranking[0].relevance_score, 0.5110583305358887);
        assert_eq!(result.ranking[1].index, 5);
        assert_eq!(result.ranking[1].relevance_score, 0.30241215229034424);
        assert!(result.provider_metadata.is_none());
        assert_eq!(result.response.unwrap().body.unwrap(), response);
    }
}
