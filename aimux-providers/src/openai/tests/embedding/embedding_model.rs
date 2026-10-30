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

// ── Older ports (previously `tests/openai_embedding_test.rs`) ──
//
// Rust translations of the AI SDK OpenAI embedding model tests.
//
// Source: `reference/ai/packages/openai/src/embedding/openai-embedding-model.test.ts`
// (164 lines, 7 cases).
//
// Each test uses `wiremock` to spin up a mock HTTP server, configures a JSON
// response, creates an `OpenAIEmbeddingModel` pointing at the mock, calls
// `do_embed`, and asserts on the result.

mod older_embedding {
    use std::collections::HashMap;

    use serde_json::{Value, json};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::{OpenAIProviderSettings, create_openai};
    use aimux_core::embedding_model::EmbeddingCallOptions;
    use aimux_core::provider::Provider;

    // ── helpers ─────────────────────────────────────────────────────────────────

    const TEST_VALUES: &[&str] = &["sunny day at the beach", "rainy day in the city"];

    /// The fixture response body from `__fixtures__/openai-embedding.json`.
    fn embedding_response_body() -> Value {
        json!({
            "object": "list",
            "data": [
                {
                    "object": "embedding",
                    "index": 0,
                    "embedding": [0.0057293195, -0.012727811, 0.020042092, -0.013437585, 0.022833068]
                },
                {
                    "object": "embedding",
                    "index": 1,
                    "embedding": [-0.037104916, -0.05178114, -0.008340587, 0.001164541, -0.0035253682]
                }
            ],
            "model": "text-embedding-3-small",
            "usage": {
                "prompt_tokens": 12,
                "total_tokens": 12
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

    // ════════════════════════════════════════════════════════════════════════════
    // doEmbed
    // ════════════════════════════════════════════════════════════════════════════

    mod do_embed {
        use super::*;

        /// upstream: @ai-sdk/openai src/embedding/openai-embedding-model.test.ts › doEmbed › should extract embedding
        #[tokio::test]
        async fn should_extract_embedding() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/embeddings"))
                .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
                .mount(&server)
                .await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.embedding_model("text-embedding-3-large").unwrap();

            let result = model
                .do_embed(&default_options(test_values()))
                .await
                .expect("should succeed");

            assert_eq!(result.embeddings.len(), 2);
            assert_eq!(
                result.embeddings[0],
                vec![
                    0.0057293195,
                    -0.012727811,
                    0.020042092,
                    -0.013437585,
                    0.022833068
                ]
            );
            assert_eq!(
                result.embeddings[1],
                vec![
                    -0.037104916,
                    -0.05178114,
                    -0.008340587,
                    0.001164541,
                    -0.0035253682
                ]
            );
        }

        /// upstream: @ai-sdk/openai src/embedding/openai-embedding-model.test.ts › doEmbed › should extract usage
        #[tokio::test]
        async fn should_extract_usage() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/embeddings"))
                .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
                .mount(&server)
                .await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.embedding_model("text-embedding-3-large").unwrap();

            let result = model
                .do_embed(&default_options(test_values()))
                .await
                .expect("should succeed");

            let usage = result.usage.expect("usage should be present");
            assert_eq!(usage.tokens, 12);
        }
    }

    /// partial: `should expose the raw response headers`; full port in src/openai/tests/embedding/embedding_model.rs.
    #[tokio::test]
    async fn should_expose_raw_response_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("test-header", "test-value")
                    .set_body_json(embedding_response_body()),
            )
            .mount(&server)
            .await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.embedding_model("text-embedding-3-large").unwrap();

        let result = model
            .do_embed(&default_options(test_values()))
            .await
            .expect("should succeed");

        let headers = result
            .response
            .as_ref()
            .and_then(|r| r.headers.as_ref())
            .expect("response headers should be present");
        assert_eq!(
            headers.get("test-header").map(std::string::String::as_str),
            Some("test-value")
        );
    }

    /// partial: `should expose the raw response body`; full port in src/openai/tests/embedding/embedding_model.rs.
    #[tokio::test]
    async fn should_expose_raw_response_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
            .mount(&server)
            .await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.embedding_model("text-embedding-3-large").unwrap();

        let result = model
            .do_embed(&default_options(test_values()))
            .await
            .expect("should succeed");

        let body = result
            .response
            .as_ref()
            .and_then(|r| r.body.as_ref())
            .expect("response body should be present");
        assert_eq!(body["usage"]["prompt_tokens"], 12);
        assert_eq!(body["data"].as_array().unwrap().len(), 2);
    }

    /// partial: `should pass the model and the values`; full port in src/openai/tests/embedding/embedding_model.rs.
    #[tokio::test]
    async fn should_pass_model_and_values() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
            .mount(&server)
            .await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.embedding_model("text-embedding-3-large").unwrap();

        let _ = model
            .do_embed(&default_options(test_values()))
            .await
            .expect("should succeed");

        let requests = server.received_requests().await.expect("requests recorded");
        assert_eq!(requests.len(), 1);
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();

        assert_eq!(body["model"], "text-embedding-3-large");
        assert_eq!(
            body["input"],
            json!(["sunny day at the beach", "rainy day in the city"])
        );
        assert_eq!(body["encoding_format"], "float");
        // dimensions and user should be absent when not provided
        assert!(body.get("dimensions").is_none());
        assert!(body.get("user").is_none());
    }

    /// `providerOptions.openai.dimensions` is sent as the number it was given, so
    /// an integer stays an integer, as the AI SDK passes it through
    /// (`embedding/openai-embedding-model.ts:94`).
    #[tokio::test]
    async fn should_pass_integer_dimensions_as_an_integer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
            .mount(&server)
            .await;

        let provider = create_openai(OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        })
        .unwrap();
        let model = provider.embedding_model("text-embedding-3-large").unwrap();
        let mut options = default_options(test_values());
        options.provider_options = Some(
            aimux_core::shared::provider_namespace("openai", json!({ "dimensions": 64 })).unwrap(),
        );
        model.do_embed(&options).await.expect("should succeed");

        let requests = server.received_requests().await.expect("requests recorded");
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["dimensions"], json!(64));
    }

    /// partial: `should pass headers`; full port in src/openai/tests/embedding/embedding_model.rs.
    /// Verifies that Authorization, OpenAI-Organization, OpenAI-Project, config-level headers, and request-level headers are all sent.
    #[tokio::test]
    async fn should_pass_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response_body()))
            .mount(&server)
            .await;

        let mut config_headers = HashMap::new();
        config_headers.insert(
            "Custom-Provider-Header".to_string(),
            "provider-header-value".to_string(),
        );

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            organization: Some("test-organization".into()),
            project: Some("test-project".into()),
            headers: Some(
                config_headers
                    .into_iter()
                    .map(|(k, v)| (k, Some(v)))
                    .collect(),
            ),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.embedding_model("text-embedding-3-large").unwrap();

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
            h.get("authorization").and_then(|v| v.to_str().ok()),
            Some("Bearer test-api-key")
        );
        assert_eq!(
            h.get("openai-organization").and_then(|v| v.to_str().ok()),
            Some("test-organization")
        );
        assert_eq!(
            h.get("openai-project").and_then(|v| v.to_str().ok()),
            Some("test-project")
        );
        assert_eq!(
            h.get("custom-provider-header")
                .and_then(|v| v.to_str().ok()),
            Some("provider-header-value")
        );
        assert_eq!(
            h.get("custom-request-header").and_then(|v| v.to_str().ok()),
            Some("request-header-value")
        );
    }
}

/// aimux's own: an explicit `null` or a wrong type fails as
/// `parseProviderOptions` fails, with the schema error as the cause.
#[tokio::test]
async fn invalid_options_fail_with_the_parse_provider_options_message() {
    for openai in [
        json!({ "dimensions": null }),
        json!({ "user": null }),
        json!({ "user": 1 }),
    ] {
        let mock = MockFetch::new(vec![json_fixture_response(&[])]);
        let error = provider(&mock)
            .embedding("text-embedding-3-large")
            .do_embed(&EmbeddingCallOptions {
                provider_options: Some(provider_options(json!({ "openai": openai.clone() }))),
                ..options()
            })
            .await
            .unwrap_err();
        assert!(
            matches!(&error, aimux_core::AiMuxError::InvalidArgument { message, cause: Some(_) }
                if message == "Invalid argument for parameter providerOptions: invalid openai provider options"),
            "{openai}: {error:?}"
        );
        assert!(mock.seen().is_empty(), "{openai}");
    }
}
