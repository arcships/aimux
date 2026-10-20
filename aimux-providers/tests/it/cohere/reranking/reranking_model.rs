//! Ports of `src/reranking/cohere-reranking-model.test.ts` from
//! `@ai-sdk/cohere@4.0.52`.
//!
//! Only the cases that `tests/it/cohere/upstream_e2e.rs` does not already
//! cover are here: the first request case of each describe is ported there.
//!
//! Harness differences from the TypeScript (expectations are unchanged):
//! - `content-length` is the byte length of the compact JSON body, as the
//!   TypeScript test server sends it.
//! - The mock records `user-agent`, which the TypeScript `requestHeaders` do
//!   not list; the header comparison drops it.

use std::collections::BTreeMap;
use std::sync::Arc;

use aimux_core::reranking_model::{
    RerankingCallOptions, RerankingDocuments, RerankingModel, RerankingResult,
};
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_providers::cohere::{CohereProvider, CohereProviderSettings, create_cohere};
use serde_json::{Value, json};

use crate::mock_fetch::{Canned, MockFetch};

const RERANKING_FIXTURE: &str = include_str!("fixtures/cohere-reranking.1.json");

fn provider(fetch: &Arc<MockFetch>) -> CohereProvider {
    create_cohere(CohereProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// A single-response mock for the reranking fixture.
fn reranking_fetch() -> Arc<MockFetch> {
    let fixture: Value = serde_json::from_str(RERANKING_FIXTURE).expect("fixture is JSON");
    let bytes = serde_json::to_vec(&fixture).expect("serializable");
    MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("content-length".into(), bytes.len().to_string()),
        ],
        body: bytes,
    }])
}

/// `doRerank` with the TypeScript call options of both describes.
async fn rerank(documents: RerankingDocuments) -> (Arc<MockFetch>, RerankingResult) {
    let fetch = reranking_fetch();
    let mut options = RerankingCallOptions::new("rainy day", documents);
    options.top_n = Some(2);
    options.provider_options = Some(
        serde_json::from_value::<SharedProviderOptions>(
            json!({"cohere": {"maxTokensPerDoc": 1000, "priority": 1}}),
        )
        .unwrap(),
    );
    let result = provider(&fetch)
        .reranking("rerank-english-v3.0")
        .do_rerank(&options)
        .await
        .unwrap();
    (fetch, result)
}

fn json_documents() -> RerankingDocuments {
    RerankingDocuments::Object {
        values: vec![
            json!({"example": "sunny day at the beach"}),
            json!({"example": "rainy day in the city"}),
        ],
    }
}

fn text_documents() -> RerankingDocuments {
    RerankingDocuments::Text {
        values: vec![
            "sunny day at the beach".to_string(),
            "rainy day in the city".to_string(),
        ],
    }
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

/// The request headers the TypeScript test checks with `toMatchInlineSnapshot`.
/// The user agent is recorded by the mock and not compared here.
fn assert_sent_headers(fetch: &MockFetch, expected: &[(&str, &str)]) {
    let mut headers = fetch.seen()[0].headers.clone();
    headers.remove("user-agent").expect("user-agent is sent");
    assert_eq!(headers, map(expected));
}

/// The `response` of the result, as the TypeScript snapshot lists it.
fn expected_response() -> Value {
    json!({
        "body": {
            "id": "b44fe75b-e3d3-489a-b61e-1a1aede3ef72",
            "meta": {
                "api_version": {"version": "2"},
                "billed_units": {"search_units": 1}
            },
            "results": [
                {"index": 1, "relevance_score": 0.10183054},
                {"index": 0, "relevance_score": 0.03762639}
            ]
        },
        "headers": {
            "content-length": "212",
            "content-type": "application/json"
        },
        "id": "b44fe75b-e3d3-489a-b61e-1a1aede3ef72"
    })
}

fn expected_ranking() -> Value {
    json!([
        {"index": 1, "relevanceScore": 0.10183054},
        {"index": 0, "relevanceScore": 0.03762639}
    ])
}

mod do_rerank {
    mod json_documents {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/reranking/cohere-reranking-model.test.ts › doRerank › json documents › should send request with the correct headers
        #[tokio::test]
        async fn should_send_request_with_the_correct_headers() {
            let (fetch, _) = rerank(json_documents()).await;
            assert_sent_headers(
                &fetch,
                &[
                    ("authorization", "Bearer test-api-key"),
                    ("content-type", "application/json"),
                ],
            );
        }

        /// upstream: @ai-sdk/cohere src/reranking/cohere-reranking-model.test.ts › doRerank › json documents › should return result with warnings
        #[tokio::test]
        async fn should_return_result_with_warnings() {
            let (_, result) = rerank(json_documents()).await;
            let warnings = result.warnings.expect("warnings are present");
            assert!(matches!(
                warnings.as_slice(),
                [Warning::Compatibility { feature, details }]
                    if feature == "object documents"
                        && details.as_deref() == Some("Object documents are converted to strings.")
            ));
        }

        /// upstream: @ai-sdk/cohere src/reranking/cohere-reranking-model.test.ts › doRerank › json documents › should return result with the correct ranking
        #[tokio::test]
        async fn should_return_result_with_the_correct_ranking() {
            let (_, result) = rerank(json_documents()).await;
            assert_eq!(
                serde_json::to_value(&result.ranking).unwrap(),
                expected_ranking()
            );
        }

        /// upstream: @ai-sdk/cohere src/reranking/cohere-reranking-model.test.ts › doRerank › json documents › should not return provider metadata (use response body instead)
        #[tokio::test]
        async fn should_not_return_provider_metadata_use_response_body_instead() {
            let (_, result) = rerank(json_documents()).await;
            assert!(result.provider_metadata.is_none());
        }

        /// upstream: @ai-sdk/cohere src/reranking/cohere-reranking-model.test.ts › doRerank › json documents › should return result with the correct response
        #[tokio::test]
        async fn should_return_result_with_the_correct_response() {
            let (_, result) = rerank(json_documents()).await;
            assert_eq!(
                serde_json::to_value(&result.response).unwrap(),
                expected_response()
            );
        }
    }

    mod text_documents {
        use super::super::*;

        /// upstream: @ai-sdk/cohere src/reranking/cohere-reranking-model.test.ts › doRerank › text documents › should send request with the correct headers
        #[tokio::test]
        async fn should_send_request_with_the_correct_headers() {
            let (fetch, _) = rerank(text_documents()).await;
            assert_sent_headers(
                &fetch,
                &[
                    ("authorization", "Bearer test-api-key"),
                    ("content-type", "application/json"),
                ],
            );
        }

        /// upstream: @ai-sdk/cohere src/reranking/cohere-reranking-model.test.ts › doRerank › text documents › should return result without warnings
        #[tokio::test]
        async fn should_return_result_without_warnings() {
            let (_, result) = rerank(text_documents()).await;
            assert_eq!(serde_json::to_value(&result.warnings).unwrap(), json!([]));
        }

        /// upstream: @ai-sdk/cohere src/reranking/cohere-reranking-model.test.ts › doRerank › text documents › should return result with the correct ranking
        #[tokio::test]
        async fn should_return_result_with_the_correct_ranking() {
            let (_, result) = rerank(text_documents()).await;
            assert_eq!(
                serde_json::to_value(&result.ranking).unwrap(),
                expected_ranking()
            );
        }

        /// upstream: @ai-sdk/cohere src/reranking/cohere-reranking-model.test.ts › doRerank › text documents › should not return provider metadata (use response body instead)
        #[tokio::test]
        async fn should_not_return_provider_metadata_use_response_body_instead() {
            let (_, result) = rerank(text_documents()).await;
            assert!(result.provider_metadata.is_none());
        }

        /// upstream: @ai-sdk/cohere src/reranking/cohere-reranking-model.test.ts › doRerank › text documents › should return result with the correct response
        #[tokio::test]
        async fn should_return_result_with_the_correct_response() {
            let (_, result) = rerank(text_documents()).await;
            assert_eq!(
                serde_json::to_value(&result.response).unwrap(),
                expected_response()
            );
        }
    }
}
