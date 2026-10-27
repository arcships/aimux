use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::{Value, json};

use crate::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};
use crate::test_support::mock_fetch::{Canned, MockFetch};
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::shared::SharedProviderOptions;

const TEST_VALUES: [&str; 2] = ["sunny day at the beach", "rainy day in the city"];
const EMBEDDINGS_URL: &str = "https://api.openai.com/v1/embeddings";
const BASE_URL: &str = "https://api.openai.com/v1";
const PINNED_USER_AGENT: &str = "ai-sdk-openai/4.0.83";

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/openai-embedding.json")).unwrap()
}

/// The test server's `json-value` response: the compact JSON body with its
/// `content-type` and `content-length`, plus the given headers.
fn json_fixture_response(headers: &[(&str, &str)]) -> Canned {
    let mut canned = Canned::json(&fixture());
    canned
        .headers
        .push(("content-length".into(), canned.body.len().to_string()));
    for (name, value) in headers {
        canned.headers.push(((*name).into(), (*value).into()));
    }
    canned
}

fn provider_with(mock: &Arc<MockFetch>, settings: OpenAIProviderSettings) -> OpenAIProvider {
    create_openai(OpenAIProviderSettings {
        api_key: Some("test-api-key".into()),
        base_url: Some(BASE_URL.into()),
        fetch: Some(mock.transport()),
        ..settings
    })
    .unwrap()
}

fn provider(mock: &Arc<MockFetch>) -> OpenAIProvider {
    provider_with(mock, OpenAIProviderSettings::default())
}

fn options() -> EmbeddingCallOptions {
    EmbeddingCallOptions {
        values: TEST_VALUES.iter().map(ToString::to_string).collect(),
        ..EmbeddingCallOptions::new("")
    }
}

fn provider_options(value: Value) -> SharedProviderOptions {
    serde_json::from_value(value).unwrap()
}

// model limits › should expose the aggregate token limit: not ported
// (sub-feature: `EmbeddingModel` has no max-input-bytes-per-call limit).

mod do_embed {
    use super::*;

    /// upstream: @ai-sdk/openai src/embedding/openai-embedding-model.test.ts › doEmbed › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let mock = MockFetch::new(vec![json_fixture_response(&[(
            "test-header",
            "test-value",
        )])]);
        let model = provider(&mock).embedding("text-embedding-3-large");

        let result = model.do_embed(&options()).await.unwrap();

        assert_eq!(
            result.response.unwrap().headers.unwrap(),
            HashMap::from([
                ("content-length".to_string(), "327".to_string()),
                ("content-type".to_string(), "application/json".to_string()),
                ("test-header".to_string(), "test-value".to_string()),
            ])
        );
    }

    /// upstream: @ai-sdk/openai src/embedding/openai-embedding-model.test.ts › doEmbed › should expose the raw response body
    #[tokio::test]
    async fn should_expose_the_raw_response_body() {
        let mock = MockFetch::new(vec![json_fixture_response(&[])]);
        let model = provider(&mock).embedding("text-embedding-3-large");

        let result = model.do_embed(&options()).await.unwrap();

        let expected: Value =
            serde_json::from_str(include_str!("snapshots/raw_response_body.json")).unwrap();
        assert_eq!(serde_json::to_value(result.response).unwrap(), expected);
    }

    /// upstream: @ai-sdk/openai src/embedding/openai-embedding-model.test.ts › doEmbed › should pass the model and the values
    #[tokio::test]
    async fn should_pass_the_model_and_the_values() {
        let mock = MockFetch::new(vec![json_fixture_response(&[])]);
        let model = provider(&mock).embedding("text-embedding-3-large");

        model.do_embed(&options()).await.unwrap();

        let seen = mock.seen();
        assert_eq!(seen[0].url, EMBEDDINGS_URL);
        assert_eq!(
            seen[0].json_body(),
            json!({
                "encoding_format": "float",
                "input": ["sunny day at the beach", "rainy day in the city"],
                "model": "text-embedding-3-large",
            })
        );
    }

    /// upstream: @ai-sdk/openai src/embedding/openai-embedding-model.test.ts › doEmbed › should pass the dimensions setting
    #[tokio::test]
    async fn should_pass_the_dimensions_setting() {
        let mock = MockFetch::new(vec![json_fixture_response(&[])]);
        let model = provider(&mock).embedding("text-embedding-3-large");

        model
            .do_embed(&EmbeddingCallOptions {
                provider_options: Some(provider_options(json!({ "openai": { "dimensions": 64 } }))),
                ..options()
            })
            .await
            .unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "dimensions": 64,
                "encoding_format": "float",
                "input": ["sunny day at the beach", "rainy day in the city"],
                "model": "text-embedding-3-large",
            })
        );
    }

    /// upstream: @ai-sdk/openai src/embedding/openai-embedding-model.test.ts › doEmbed › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let mock = MockFetch::new(vec![json_fixture_response(&[])]);
        let provider = provider_with(
            &mock,
            OpenAIProviderSettings {
                organization: Some("test-organization".into()),
                project: Some("test-project".into()),
                headers: Some(HashMap::from([(
                    "Custom-Provider-Header".to_string(),
                    Some("provider-header-value".to_string()),
                )])),
                ..Default::default()
            },
        );

        provider
            .embedding("text-embedding-3-large")
            .do_embed(&EmbeddingCallOptions {
                headers: Some(HashMap::from([(
                    "Custom-Request-Header".to_string(),
                    "request-header-value".to_string(),
                )])),
                ..options()
            })
            .await
            .unwrap();

        let mut headers = mock.seen()[0].headers.clone();
        let user_agent = headers.remove("user-agent").unwrap_or_default();
        assert_eq!(
            headers,
            BTreeMap::from([
                (
                    "authorization".to_string(),
                    "Bearer test-api-key".to_string()
                ),
                ("content-type".to_string(), "application/json".to_string()),
                (
                    "custom-provider-header".to_string(),
                    "provider-header-value".to_string()
                ),
                (
                    "custom-request-header".to_string(),
                    "request-header-value".to_string()
                ),
                (
                    "openai-organization".to_string(),
                    "test-organization".to_string()
                ),
                ("openai-project".to_string(), "test-project".to_string()),
            ])
        );
        assert!(user_agent.contains(PINNED_USER_AGENT), "{user_agent}");
    }
}
