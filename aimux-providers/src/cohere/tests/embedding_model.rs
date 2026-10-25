//! Ports of `src/cohere-embedding-model.test.ts` from `@ai-sdk/cohere@4.0.52`.
//!
//! Only the cases that the other tests do not already cover are here:
//! `should extract embedding`, `should extract usage`, `should pass the model
//! and the values` and `should pass headers` are in
//! `tests/cohere_embedding_test.rs` and `tests/cohere_upstream_e2e_test.rs`.
//! `should expose the raw response` compares the whole snapshot here; the
//! older case in `tests/cohere_embedding_test.rs` checks part of it.
//!
//! Harness differences from the TypeScript (expectations are unchanged):
//! - The TypeScript test server answers from the fixture file. The mock
//!   answers with the fixture's content; `content-length` is the byte length
//!   of the compact JSON body, as the TypeScript server sends it.
//! - The mock records `user-agent`; the TypeScript test checks it with
//!   `vi.mock('./version')`. These cases do not check it.

use std::sync::Arc;

use crate::cohere::{CohereProvider, CohereProviderSettings, create_cohere};
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::shared::SharedProviderOptions;
use serde_json::{Value, json};

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

mod do_embed {
    use super::*;

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
