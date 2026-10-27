//! Port of `@ai-sdk/openai-compatible` 3.0.62
//! `src/embedding/openai-compatible-embedding-model.test.ts`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};

use crate::openai_compatible::{
    OpenAICompatibleEmbeddingModel, OpenAICompatibleProvider, OpenAICompatibleProviderSettings,
    create_openai_compatible,
};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

/// The user agent aimux pins (upstream mocks `0.0.0-test`); compared apart
/// from the other request headers, which upstream's test server lists
/// without it.
const USER_AGENT: &str = "ai-sdk-openai-compatible/3.0.62 ai-sdk-provider-utils/5.0.53";

fn dummy_embeddings() -> Vec<Vec<f32>> {
    vec![vec![0.1, 0.2, 0.3, 0.4, 0.5], vec![0.6, 0.7, 0.8, 0.9, 1.0]]
}

const TEST_VALUES: [&str; 2] = ["sunny day at the beach", "rainy day in the city"];

fn test_values() -> Vec<String> {
    TEST_VALUES.iter().map(ToString::to_string).collect()
}

/// `{ values: testValues }`.
fn embed_options() -> EmbeddingCallOptions {
    EmbeddingCallOptions {
        values: test_values(),
        ..EmbeddingCallOptions::new("")
    }
}

fn with_provider_options(
    mut options: EmbeddingCallOptions,
    provider_options: Value,
) -> EmbeddingCallOptions {
    options.provider_options = Some(serde_json::from_value(provider_options).unwrap());
    options
}

fn header_map(entries: &[(&str, &str)]) -> HashMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect()
}

/// `createOpenAICompatible({ baseURL: 'https://my.api.com/v1/', name:
/// 'test-provider', headers })` on the mock transport.
fn create_provider(mock: &Arc<MockFetch>, headers: &[(&str, &str)]) -> OpenAICompatibleProvider {
    create_openai_compatible(OpenAICompatibleProviderSettings {
        base_url: "https://my.api.com/v1/".into(),
        name: "test-provider".into(),
        headers: Some(
            headers
                .iter()
                .map(|(name, value)| ((*name).to_string(), Some((*value).to_string())))
                .collect(),
        ),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// The module-level `provider` of the upstream file.
fn provider(mock: &Arc<MockFetch>) -> OpenAICompatibleProvider {
    create_provider(mock, &[("Authorization", "Bearer test-api-key")])
}

/// The module-level `model` of the upstream file.
fn model(mock: &Arc<MockFetch>) -> OpenAICompatibleEmbeddingModel {
    provider(mock).embedding("text-embedding-3-large")
}

/// The request headers without `user-agent`, which is checked against the
/// pinned value.
fn request_headers(seen: &Seen) -> BTreeMap<String, String> {
    let mut headers = seen.headers.clone();
    assert_eq!(
        headers.remove("user-agent").as_deref(),
        Some(USER_AGENT),
        "pinned user agent"
    );
    headers
}

mod do_embed {
    use super::*;

    /// The arguments of upstream's `prepareJsonResponse`.
    struct JsonResponse {
        embeddings: Vec<Vec<f32>>,
        usage: Value,
        headers: &'static [(&'static str, &'static str)],
    }

    impl Default for JsonResponse {
        fn default() -> Self {
            Self {
                embeddings: dummy_embeddings(),
                usage: json!({ "prompt_tokens": 8, "total_tokens": 8 }),
                headers: &[],
            }
        }
    }

    /// The `json-value` response of the test server: the body as
    /// `JSON.stringify` writes it, with its `content-type` and
    /// `content-length` headers.
    fn prepare_json_response(response: JsonResponse) -> Arc<MockFetch> {
        let data: Vec<String> = response
            .embeddings
            .iter()
            .enumerate()
            .map(|(index, embedding)| {
                // `Display` writes `1.0` as `1`, as `JSON.stringify` does.
                let numbers: Vec<String> = embedding.iter().map(ToString::to_string).collect();
                format!(
                    "{{\"object\":\"embedding\",\"index\":{index},\"embedding\":[{}]}}",
                    numbers.join(",")
                )
            })
            .collect();
        let body = format!(
            "{{\"object\":\"list\",\"data\":[{}],\"model\":\"text-embedding-3-large\",\"usage\":{}}}",
            data.join(","),
            serde_json::to_string(&response.usage).unwrap()
        );
        let mut headers = vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("content-length".to_string(), body.len().to_string()),
        ];
        headers.extend(
            response
                .headers
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string())),
        );
        MockFetch::new(vec![Canned {
            status: 200,
            headers,
            body: body.into_bytes(),
        }])
    }

    /// upstream: @ai-sdk/openai-compatible src/embedding/openai-compatible-embedding-model.test.ts › doEmbed › should extract embedding
    #[tokio::test]
    async fn should_extract_embedding() {
        let mock = prepare_json_response(JsonResponse::default());

        let result = model(&mock).do_embed(&embed_options()).await.unwrap();

        assert_eq!(result.embeddings, dummy_embeddings());
    }

    /// upstream: @ai-sdk/openai-compatible src/embedding/openai-compatible-embedding-model.test.ts › doEmbed › should expose the raw response headers
    #[tokio::test]
    async fn should_expose_the_raw_response_headers() {
        let mock = prepare_json_response(JsonResponse {
            headers: &[("test-header", "test-value")],
            ..Default::default()
        });

        let result = model(&mock).do_embed(&embed_options()).await.unwrap();

        assert_eq!(
            result.response.unwrap().headers.unwrap(),
            header_map(&[
                // default headers:
                ("content-length", "236"),
                ("content-type", "application/json"),
                // custom header
                ("test-header", "test-value"),
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/embedding/openai-compatible-embedding-model.test.ts › doEmbed › should extract usage
    #[tokio::test]
    async fn should_extract_usage() {
        let mock = prepare_json_response(JsonResponse {
            usage: json!({ "prompt_tokens": 20, "total_tokens": 20 }),
            ..Default::default()
        });

        let result = model(&mock).do_embed(&embed_options()).await.unwrap();

        assert_eq!(
            serde_json::to_value(&result.usage).unwrap(),
            json!({ "tokens": 20 })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/embedding/openai-compatible-embedding-model.test.ts › doEmbed › should pass the model and the values
    #[tokio::test]
    async fn should_pass_the_model_and_the_values() {
        let mock = prepare_json_response(JsonResponse::default());

        model(&mock).do_embed(&embed_options()).await.unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "model": "text-embedding-3-large",
                "input": TEST_VALUES,
                "encoding_format": "float",
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/embedding/openai-compatible-embedding-model.test.ts › doEmbed › should pass the dimensions setting
    #[tokio::test]
    async fn should_pass_the_dimensions_setting() {
        let mock = prepare_json_response(JsonResponse::default());

        provider(&mock)
            .embedding("text-embedding-3-large")
            .do_embed(&with_provider_options(
                embed_options(),
                json!({ "openaiCompatible": { "dimensions": 64 } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "model": "text-embedding-3-large",
                "input": TEST_VALUES,
                "encoding_format": "float",
                "dimensions": 64,
            })
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/embedding/openai-compatible-embedding-model.test.ts › doEmbed › should pass settings with deprecated openai-compatible key and emit warning
    #[tokio::test]
    async fn should_pass_settings_with_deprecated_openai_compatible_key_and_emit_warning() {
        let mock = prepare_json_response(JsonResponse::default());

        let result = provider(&mock)
            .embedding("text-embedding-3-large")
            .do_embed(&with_provider_options(
                embed_options(),
                json!({ "openai-compatible": { "dimensions": 64 } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "model": "text-embedding-3-large",
                "input": TEST_VALUES,
                "encoding_format": "float",
                "dimensions": 64,
            })
        );

        assert_eq!(
            serde_json::to_value(&result.warnings).unwrap(),
            json!([{
                "message": "Use 'openaiCompatible' instead.",
                "setting": "providerOptions key 'openai-compatible'",
                "type": "deprecated",
            }])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/embedding/openai-compatible-embedding-model.test.ts › doEmbed › should emit deprecated warning when raw provider name key is used
    #[tokio::test]
    async fn should_emit_deprecated_warning_when_raw_provider_name_key_is_used() {
        let mock = prepare_json_response(JsonResponse::default());

        let result = provider(&mock)
            .embedding("text-embedding-3-large")
            .do_embed(&with_provider_options(
                embed_options(),
                json!({ "test-provider": { "dimensions": 64 } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.warnings).unwrap(),
            json!([{
                "message": "Use 'testProvider' instead.",
                "setting": "providerOptions key 'test-provider'",
                "type": "deprecated",
            }])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/embedding/openai-compatible-embedding-model.test.ts › doEmbed › should not emit deprecated warning when camelCase provider name key is used
    #[tokio::test]
    async fn should_not_emit_deprecated_warning_when_camel_case_provider_name_key_is_used() {
        let mock = prepare_json_response(JsonResponse::default());

        let result = provider(&mock)
            .embedding("text-embedding-3-large")
            .do_embed(&with_provider_options(
                embed_options(),
                json!({ "testProvider": { "dimensions": 64 } }),
            ))
            .await
            .unwrap();

        assert_eq!(serde_json::to_value(&result.warnings).unwrap(), json!([]));
    }

    /// upstream: @ai-sdk/openai-compatible src/embedding/openai-compatible-embedding-model.test.ts › doEmbed › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let mock = prepare_json_response(JsonResponse::default());

        let provider = create_provider(
            &mock,
            &[
                ("Authorization", "Bearer test-api-key"),
                ("Custom-Provider-Header", "provider-header-value"),
            ],
        );

        let mut options = embed_options();
        options.headers = Some(header_map(&[(
            "Custom-Request-Header",
            "request-header-value",
        )]));
        provider
            .embedding("text-embedding-3-large")
            .do_embed(&options)
            .await
            .unwrap();

        assert_eq!(
            request_headers(&mock.seen()[0]),
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
            ])
        );
    }
}

// ── Older ports (previously `tests/openai_compatible_upstream_e2e_test.rs`) ──
//
// End-to-end subset of the pinned openai-compatible upstream tests.

mod upstream_e2e {
    use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
    use serde_json::json;

    use crate::openai_compatible::tests::support::*;
    use crate::test_support::mock_fetch::{Canned, MockFetch};

    /// partial: `should extract embedding`; full port in src/openai_compatible/tests/embedding/embedding_model.rs.
    /// partial: `should pass the model and the values`; full port in src/openai_compatible/tests/embedding/embedding_model.rs.
    #[tokio::test]
    async fn embedding_request_and_result() {
        let body = json!({"object":"list","data":[
            {"object":"embedding","index":0,"embedding":[0.1,0.2,0.3,0.4,0.5]},
            {"object":"embedding","index":1,"embedding":[0.6,0.7,0.8,0.9,1.0]}],
            "model":"test-embedding","usage":{"prompt_tokens":8,"total_tokens":8}});
        let fetch = MockFetch::new(vec![Canned::json(&body)]);
        let mut opts = EmbeddingCallOptions::new("sunny day at the beach");
        opts.values.push("rainy day in the city".into());
        let result = provider(&fetch)
            .embedding("test-embedding")
            .do_embed(&opts)
            .await
            .unwrap();
        assert_request(
            &fetch,
            "/embeddings",
            json!({"model":"test-embedding",
            "input":["sunny day at the beach","rainy day in the city"],"encoding_format":"float"}),
        );
        assert_eq!(
            result.embeddings,
            vec![vec![0.1, 0.2, 0.3, 0.4, 0.5], vec![0.6, 0.7, 0.8, 0.9, 1.0]]
        );
        assert_eq!(result.usage.unwrap().tokens, 8);
        let response = result.response.unwrap();
        assert_eq!(response.body, Some(body));
        assert_eq!(
            response.headers.unwrap()["content-type"],
            "application/json"
        );
        assert!(result.warnings.is_empty());
    }
}
