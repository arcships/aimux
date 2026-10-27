//! Ports of `@ai-sdk/amazon-bedrock` 5.0.100
//! `src/amazon-bedrock-provider.test.ts`.
//!
//! Upstream mocks the model classes, the fetch factories and the settings
//! loaders, then inspects the arguments the provider passes them. aimux has
//! none of these seams (the models take a private `EndpointConfig`, and the
//! transport is chosen per request), so each case makes one request through
//! the provider with the mock transport and asserts what upstream's mocks
//! would have received, as it shows on the wire: the `baseUrl()` is the
//! request URL's prefix, `headers()` are the request headers, an API-key
//! fetch is an unsigned request with `Authorization: Bearer <key>`, and a
//! SigV4 fetch is a signed one whose credential names the access key.
//!
//! Upstream's `loadSetting` mock returns the setting or `us-east-1`, and its
//! `loadOptionalSetting` mock returns the setting and never the environment.
//! The cases that depend on the environment emulate that with environment
//! variables (`AWS_REGION`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` set
//! to `us-east-1`, the optional variables unset; a case's own
//! `mockImplementation` sets more) and are `#[serial]`.
//!
//! Upstream mocks the version as `0.0.0-test`; the user-agent part checked
//! here is the pinned `ai-sdk-amazon-bedrock/5.0.100`.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::image_model::{ImageCallOptions, ImageModel};
use aimux_core::language_model_message::{LanguageModelMessage, TextPart, UserPart};
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_core::reranking_model::{RerankingCallOptions, RerankingDocuments, RerankingModel};
use aimux_provider_utils::{AwsCredentials, FetchFunction, HeaderMapOpt, Resolvable};

use crate::bedrock::{
    AmazonBedrockChatModelFamily, AmazonBedrockChatModelSettings,
    AmazonBedrockEmbeddingModelFamily, AmazonBedrockEmbeddingModelSettings, AmazonBedrockProvider,
    AmazonBedrockProviderSettings, create_amazon_bedrock,
};
use crate::test_support::mock_fetch::{Canned, EnvVar, MockFetch, Seen};

/// The pinned `ai-sdk-amazon-bedrock/<v>` user-agent part.
const USER_AGENT_PART: &str = "ai-sdk-amazon-bedrock/5.0.100";

/// The environment variables the provider reads.
const ENV_VARS: [&str; 8] = [
    "AWS_REGION",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_BEARER_TOKEN_BEDROCK",
    "AWS_ENDPOINT_URL",
    "AWS_ENDPOINT_URL_BEDROCK_RUNTIME",
    "AWS_ENDPOINT_URL_BEDROCK_AGENT_RUNTIME",
];

/// Sets the environment the upstream settings mocks describe, with
/// `overrides` (`None` unsets a variable), and restores it when dropped.
#[must_use]
fn mocked_env(overrides: &[(&'static str, Option<&str>)]) -> Vec<EnvVar> {
    ENV_VARS
        .iter()
        .map(|&name| {
            let base = match name {
                // `loadSetting` returns `settingValue || 'us-east-1'`.
                "AWS_REGION" | "AWS_ACCESS_KEY_ID" | "AWS_SECRET_ACCESS_KEY" => Some("us-east-1"),
                // `loadOptionalSetting` returns only `settingValue`.
                _ => None,
            };
            let value = overrides
                .iter()
                .find(|(overridden, _)| *overridden == name)
                .map_or(base, |(_, value)| *value);
            EnvVar::set(name, value)
        })
        .collect()
}

/// The `loadOptionalSetting(() => undefined)` / no-environment cases: only
/// the explicit settings count.
#[must_use]
fn empty_env() -> Vec<EnvVar> {
    ENV_VARS
        .iter()
        .map(|&name| EnvVar::set(name, None))
        .collect()
}

/// A Converse response.
fn converse_response() -> Canned {
    Canned::json(&json!({
        "output": { "message": { "role": "assistant", "content": [{ "text": "ok" }] } },
        "stopReason": "end_turn",
        "usage": { "inputTokens": 1, "outputTokens": 1, "totalTokens": 2 },
    }))
}

fn chat_options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::User {
        content: vec![UserPart::Text(TextPart {
            text: "Hello".into(),
            provider_options: None,
        })],
        provider_options: None,
    }])
}

/// `create_amazon_bedrock(settings)` with the mock transport.
fn provider(
    settings: AmazonBedrockProviderSettings,
    mock: &Arc<MockFetch>,
) -> AmazonBedrockProvider {
    let fetch: FetchFunction = mock.transport();
    create_amazon_bedrock(AmazonBedrockProviderSettings {
        fetch: Some(fetch),
        ..settings
    })
    .expect("valid Bedrock settings")
}

/// One `doGenerate` of `provider(model_id)`; the request it sends.
async fn generate(provider: &AmazonBedrockProvider, mock: &Arc<MockFetch>, model_id: &str) -> Seen {
    let model = provider.call(model_id);
    assert_eq!(model.model_id(), model_id);
    model
        .do_generate(&chat_options())
        .await
        .expect("generation succeeds");
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    seen.into_iter().next().expect("one request")
}

/// One `doEmbed` of `provider.embedding(model_id)`; the request it sends.
async fn embed(provider: &AmazonBedrockProvider, mock: &Arc<MockFetch>, model_id: &str) -> Seen {
    let model = provider.embedding(model_id);
    assert_eq!(model.model_id(), model_id);
    model
        .do_embed(&EmbeddingCallOptions::new("x"))
        .await
        .expect("embedding succeeds");
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    seen.into_iter().next().expect("one request")
}

fn titan_response() -> Canned {
    Canned::json(&json!({ "embedding": [0.1], "inputTextTokenCount": 1 }))
}

fn rerank_response() -> Canned {
    Canned::json(&json!({ "results": [{ "index": 0, "relevanceScore": 0.5 }] }))
}

/// One `doRerank` of `provider.reranking(model_id)`; the request it sends.
async fn rerank(provider: &AmazonBedrockProvider, mock: &Arc<MockFetch>, model_id: &str) -> Seen {
    let model = provider.reranking(model_id);
    assert_eq!(model.model_id(), model_id);
    model
        .do_rerank(&RerankingCallOptions::new(
            "query",
            RerankingDocuments::Text {
                values: vec!["document".to_string()],
            },
        ))
        .await
        .expect("reranking succeeds");
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    seen.into_iter().next().expect("one request")
}

/// The base URL a Converse request of `model_id` was sent to.
fn base_url_of(seen: &Seen, path: &str) -> String {
    seen.url
        .strip_suffix(path)
        .unwrap_or_else(|| panic!("{} does not end with {path}", seen.url))
        .to_string()
}

fn converse_base_url(seen: &Seen) -> String {
    base_url_of(seen, "/model/anthropic.claude-v2/converse")
}

/// `headers()['user-agent']` contains `part`.
fn assert_user_agent_contains(headers: &BTreeMap<String, String>, part: &str) {
    let user_agent = headers.get("user-agent").expect("user-agent is sent");
    assert!(
        user_agent.contains(part),
        "user-agent {user_agent} lacks {part}"
    );
}

/// The request went through the API-key fetch: `Bearer <key>`, not signed.
fn assert_api_key(seen: &Seen, api_key: &str) {
    assert_eq!(
        seen.headers.get("authorization").map(String::as_str),
        Some(format!("Bearer {api_key}").as_str())
    );
    assert!(
        !seen.headers.contains_key("x-amz-date"),
        "{:?}",
        seen.headers
    );
}

/// The request went through the SigV4 fetch, signed with `access_key_id`.
fn assert_sigv4(seen: &Seen, access_key_id: &str) {
    let authorization = seen.headers.get("authorization").expect("authorization");
    assert!(
        authorization.starts_with(&format!("AWS4-HMAC-SHA256 Credential={access_key_id}/")),
        "unexpected authorization: {authorization}"
    );
    assert!(
        seen.headers.contains_key("x-amz-date"),
        "{:?}",
        seen.headers
    );
}

fn custom_headers() -> Option<HeaderMapOpt> {
    Some(HeaderMapOpt::from([(
        "custom-header".to_string(),
        Some("value".to_string()),
    )]))
}

/// A credential provider that resolves dynamic credentials (no region, as
/// upstream's).
fn dynamic_credential_provider() -> Resolvable<AwsCredentials> {
    Resolvable::from_async_fn(|| async {
        Ok(AwsCredentials {
            access_key_id: "dynamic-access-key".to_string(),
            secret_access_key: "dynamic-secret-key".to_string(),
            session_token: Some("dynamic-session-token".to_string()),
            region: String::new(),
        })
    })
}

mod amazon_bedrock_provider {
    use super::*;

    mod create_amazon_bedrock {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › should create a provider instance with default options
        #[tokio::test]
        #[serial_test::serial]
        async fn should_create_a_provider_instance_with_default_options() {
            let _env = mocked_env(&[]);
            let mock = MockFetch::new(vec![converse_response()]);
            let provider = provider(AmazonBedrockProviderSettings::default(), &mock);

            let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

            assert_user_agent_contains(&seen.headers, USER_AGENT_PART);
            assert_eq!(
                converse_base_url(&seen),
                "https://bedrock-runtime.us-east-1.amazonaws.com"
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › should create a provider instance with custom options
        #[tokio::test]
        #[serial_test::serial]
        async fn should_create_a_provider_instance_with_custom_options() {
            let _env = mocked_env(&[]);
            let mock = MockFetch::new(vec![converse_response()]);
            let provider = provider(
                AmazonBedrockProviderSettings {
                    region: Some("eu-west-1".to_string()),
                    base_url: Some("https://custom.url".to_string()),
                    headers: custom_headers(),
                    ..Default::default()
                },
                &mock,
            );

            let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

            assert_eq!(seen.headers["custom-header"], "value");
            assert_user_agent_contains(&seen.headers, USER_AGENT_PART);
            assert_eq!(converse_base_url(&seen), "https://custom.url");
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › resolves the Bedrock Runtime endpoint for %s
        #[tokio::test]
        #[serial_test::serial]
        async fn resolves_the_bedrock_runtime_endpoint_for_region() {
            let _env = mocked_env(&[]);
            for (region, dns_suffix) in [
                ("cn-north-1", "amazonaws.com.cn"),
                ("us-gov-west-1", "amazonaws.com"),
                ("us-iso-east-1", "c2s.ic.gov"),
                ("us-isob-east-1", "sc2s.sgov.gov"),
                ("eu-isoe-west-1", "cloud.adc-e.uk"),
                ("us-isof-south-1", "csp.hci.ic.gov"),
                ("eusc-de-east-1", "amazonaws.eu"),
            ] {
                let mock = MockFetch::new(vec![converse_response()]);
                let provider = provider(
                    AmazonBedrockProviderSettings {
                        region: Some(region.to_string()),
                        ..Default::default()
                    },
                    &mock,
                );

                let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

                assert_eq!(
                    converse_base_url(&seen),
                    format!("https://bedrock-runtime.{region}.{dns_suffix}")
                );
            }
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › uses an explicit base URL without loading a region
        ///
        /// `loadSetting` not being called is the request succeeding with no
        /// region anywhere.
        #[tokio::test]
        #[serial_test::serial]
        async fn uses_an_explicit_base_url_without_loading_a_region() {
            let _env = mocked_env(&[
                (
                    "AWS_ENDPOINT_URL_BEDROCK_RUNTIME",
                    Some("https://runtime.example.com"),
                ),
                ("AWS_ENDPOINT_URL", Some("https://global.example.com")),
                ("AWS_REGION", None),
            ]);
            let mock = MockFetch::new(vec![converse_response()]);
            let provider = provider(
                AmazonBedrockProviderSettings {
                    api_key: Some("test-api-key".to_string().into()),
                    base_url: Some("https://explicit.example.com/".to_string()),
                    ..Default::default()
                },
                &mock,
            );

            let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

            assert_eq!(converse_base_url(&seen), "https://explicit.example.com");
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › prefers the service-specific Bedrock Runtime endpoint over the global endpoint
        #[tokio::test]
        #[serial_test::serial]
        async fn prefers_the_service_specific_bedrock_runtime_endpoint_over_the_global_endpoint() {
            let _env = mocked_env(&[
                (
                    "AWS_ENDPOINT_URL_BEDROCK_RUNTIME",
                    Some("https://runtime.example.com/"),
                ),
                ("AWS_ENDPOINT_URL", Some("https://global.example.com")),
                ("AWS_REGION", None),
            ]);
            let mock = MockFetch::new(vec![converse_response()]);
            let provider = provider(
                AmazonBedrockProviderSettings {
                    api_key: Some("test-api-key".to_string().into()),
                    ..Default::default()
                },
                &mock,
            );

            let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

            assert_eq!(converse_base_url(&seen), "https://runtime.example.com");
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › uses the global endpoint override when no service-specific endpoint is set
        #[tokio::test]
        #[serial_test::serial]
        async fn uses_the_global_endpoint_override_when_no_service_specific_endpoint_is_set() {
            let _env = mocked_env(&[
                ("AWS_ENDPOINT_URL", Some("https://global.example.com/")),
                ("AWS_REGION", None),
            ]);
            let mock = MockFetch::new(vec![converse_response()]);
            let provider = provider(
                AmazonBedrockProviderSettings {
                    api_key: Some("test-api-key".to_string().into()),
                    ..Default::default()
                },
                &mock,
            );

            let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

            assert_eq!(converse_base_url(&seen), "https://global.example.com");
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › uses a separate service-specific endpoint for Bedrock Agent Runtime
        #[tokio::test]
        #[serial_test::serial]
        async fn uses_a_separate_service_specific_endpoint_for_bedrock_agent_runtime() {
            let _env = mocked_env(&[
                (
                    "AWS_ENDPOINT_URL_BEDROCK_RUNTIME",
                    Some("https://runtime.example.com"),
                ),
                (
                    "AWS_ENDPOINT_URL_BEDROCK_AGENT_RUNTIME",
                    Some("https://agent-runtime.example.com"),
                ),
            ]);
            let mock = MockFetch::new(vec![rerank_response()]);
            let provider = provider(
                AmazonBedrockProviderSettings {
                    region: Some("us-east-1".to_string()),
                    ..Default::default()
                },
                &mock,
            );

            let seen = rerank(&provider, &mock, "amazon.rerank-v1:0").await;

            assert_eq!(
                base_url_of(&seen, "/rerank"),
                "https://agent-runtime.example.com"
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › resolves the Bedrock Agent Runtime endpoint for a non-standard AWS partition
        #[tokio::test]
        #[serial_test::serial]
        async fn resolves_the_bedrock_agent_runtime_endpoint_for_a_non_standard_aws_partition() {
            let _env = mocked_env(&[]);
            let mock = MockFetch::new(vec![rerank_response()]);
            let provider = provider(
                AmazonBedrockProviderSettings {
                    region: Some("eu-isoe-west-1".to_string()),
                    ..Default::default()
                },
                &mock,
            );

            let seen = rerank(&provider, &mock, "amazon.rerank-v1:0").await;

            assert_eq!(
                base_url_of(&seen, "/rerank"),
                "https://bedrock-agent-runtime.eu-isoe-west-1.cloud.adc-e.uk"
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › should accept a credentialProvider in options
        #[tokio::test]
        #[serial_test::serial]
        async fn should_accept_a_credential_provider_in_options() {
            let _env = mocked_env(&[]);
            let mock = MockFetch::new(vec![converse_response()]);
            let provider = provider(
                AmazonBedrockProviderSettings {
                    credential_provider: Some(dynamic_credential_provider()),
                    ..Default::default()
                },
                &mock,
            );

            let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

            assert_user_agent_contains(&seen.headers, USER_AGENT_PART);
            assert_eq!(
                converse_base_url(&seen),
                "https://bedrock-runtime.us-east-1.amazonaws.com"
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › should prioritize credentialProvider over static credentials
        #[test]
        fn should_prioritize_credential_provider_over_static_credentials() {
            let provider = create_amazon_bedrock(AmazonBedrockProviderSettings {
                access_key_id: Some("static-access-key".to_string()),
                secret_access_key: Some("static-secret-key".to_string()),
                session_token: Some("static-session-token".to_string()),
                credential_provider: Some(dynamic_credential_provider()),
                ..Default::default()
            })
            .expect("valid Bedrock settings");

            let model = provider.call("anthropic.claude-v2");

            assert_eq!(model.model_id(), "anthropic.claude-v2");
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › should pass headers to embedding model
        #[tokio::test]
        #[serial_test::serial]
        async fn should_pass_headers_to_embedding_model() {
            let _env = mocked_env(&[]);
            let mock = MockFetch::new(vec![titan_response()]);
            let provider = provider(
                AmazonBedrockProviderSettings {
                    headers: custom_headers(),
                    ..Default::default()
                },
                &mock,
            );

            let seen = embed(&provider, &mock, "amazon.titan-embed-text-v1").await;

            assert_eq!(seen.headers["custom-header"], "value");
            assert_user_agent_contains(&seen.headers, USER_AGENT_PART);
        }

        // Not ported (ts-only): "should throw error when called with new
        // keyword". A Rust provider is not a JS function and has no `new`.

        mod api_key_authentication {
            use super::*;

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › API Key Authentication › should use API key when provided in options
            #[tokio::test]
            #[serial_test::serial]
            async fn should_use_api_key_when_provided_in_options() {
                let _env = mocked_env(&[]);
                let mock = MockFetch::new(vec![converse_response()]);
                let provider = provider(
                    AmazonBedrockProviderSettings {
                        api_key: Some("test-api-key".to_string().into()),
                        region: Some("us-east-1".to_string()),
                        ..Default::default()
                    },
                    &mock,
                );

                let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

                assert_api_key(&seen, "test-api-key");
                assert_user_agent_contains(&seen.headers, "ai-sdk-amazon-bedrock/");
                assert_eq!(
                    converse_base_url(&seen),
                    "https://bedrock-runtime.us-east-1.amazonaws.com"
                );
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › API Key Authentication › should use API key from environment variable
            #[tokio::test]
            #[serial_test::serial]
            async fn should_use_api_key_from_the_environment_variable() {
                let _env = mocked_env(&[("AWS_BEARER_TOKEN_BEDROCK", Some("env-api-key"))]);
                let mock = MockFetch::new(vec![converse_response()]);
                let provider = provider(
                    AmazonBedrockProviderSettings {
                        region: Some("us-east-1".to_string()),
                        ..Default::default()
                    },
                    &mock,
                );

                let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

                assert_api_key(&seen, "env-api-key");
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › API Key Authentication › should prioritize options.apiKey over environment variable
            #[tokio::test]
            #[serial_test::serial]
            async fn should_prioritize_options_api_key_over_environment_variable() {
                let _env = mocked_env(&[("AWS_BEARER_TOKEN_BEDROCK", Some("env-api-key"))]);
                let mock = MockFetch::new(vec![converse_response()]);
                let provider = provider(
                    AmazonBedrockProviderSettings {
                        api_key: Some("options-api-key".to_string().into()),
                        region: Some("us-east-1".to_string()),
                        ..Default::default()
                    },
                    &mock,
                );

                let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

                assert_api_key(&seen, "options-api-key");
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › API Key Authentication › should fall back to SigV4 when no API key provided
            #[tokio::test]
            #[serial_test::serial]
            async fn should_fall_back_to_sig_v4_when_no_api_key_provided() {
                let _env = empty_env();
                let mock = MockFetch::new(vec![converse_response()]);
                let provider = provider(
                    AmazonBedrockProviderSettings {
                        region: Some("us-east-1".to_string()),
                        access_key_id: Some("test-access-key".to_string()),
                        secret_access_key: Some("test-secret-key".to_string()),
                        ..Default::default()
                    },
                    &mock,
                );

                let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

                assert_sigv4(&seen, "test-access-key");
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › API Key Authentication › should pass custom fetch function to API key authentication
            ///
            /// The custom fetch is the mock transport: the API-key request
            /// reaches it.
            #[tokio::test]
            #[serial_test::serial]
            async fn should_pass_custom_fetch_function_to_api_key_authentication() {
                let _env = mocked_env(&[]);
                let mock = MockFetch::new(vec![converse_response()]);
                let provider = provider(
                    AmazonBedrockProviderSettings {
                        api_key: Some("test-api-key".to_string().into()),
                        region: Some("us-east-1".to_string()),
                        ..Default::default()
                    },
                    &mock,
                );

                let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

                assert_api_key(&seen, "test-api-key");
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › API Key Authentication › should pass custom fetch function to SigV4 authentication
            ///
            /// The custom fetch is the mock transport: the signed request
            /// reaches it.
            #[tokio::test]
            #[serial_test::serial]
            async fn should_pass_custom_fetch_function_to_sig_v4_authentication() {
                let _env = empty_env();
                let mock = MockFetch::new(vec![converse_response()]);
                let provider = provider(
                    AmazonBedrockProviderSettings {
                        region: Some("us-east-1".to_string()),
                        access_key_id: Some("test-access-key".to_string()),
                        secret_access_key: Some("test-secret-key".to_string()),
                        ..Default::default()
                    },
                    &mock,
                );

                let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

                assert_sigv4(&seen, "test-access-key");
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › API Key Authentication › should work with embedding models when using API key
            #[tokio::test]
            #[serial_test::serial]
            async fn should_work_with_embedding_models_when_using_api_key() {
                let _env = mocked_env(&[]);
                let mock = MockFetch::new(vec![titan_response()]);
                let provider = provider(
                    AmazonBedrockProviderSettings {
                        api_key: Some("test-api-key".to_string().into()),
                        region: Some("us-east-1".to_string()),
                        headers: custom_headers(),
                        ..Default::default()
                    },
                    &mock,
                );

                let seen = embed(&provider, &mock, "amazon.titan-embed-text-v1").await;

                assert_eq!(seen.headers["custom-header"], "value");
                assert_user_agent_contains(&seen.headers, USER_AGENT_PART);
                assert_api_key(&seen, "test-api-key");
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › API Key Authentication › should work with image models when using API key
            #[tokio::test]
            #[serial_test::serial]
            async fn should_work_with_image_models_when_using_api_key() {
                let _env = mocked_env(&[]);
                let mock = MockFetch::new(vec![Canned::json(&json!({ "images": ["image"] }))]);
                let provider = provider(
                    AmazonBedrockProviderSettings {
                        api_key: Some("test-api-key".to_string().into()),
                        region: Some("us-east-1".to_string()),
                        headers: custom_headers(),
                        ..Default::default()
                    },
                    &mock,
                );

                let model = provider.image("amazon.titan-image-generator");
                assert_eq!(model.model_id(), "amazon.titan-image-generator");
                model
                    .do_generate(&ImageCallOptions::new("prompt"))
                    .await
                    .expect("generation succeeds");

                let seen = mock.seen();
                assert_eq!(seen.len(), 1);
                assert_eq!(seen[0].headers["custom-header"], "value");
                assert_user_agent_contains(&seen[0].headers, USER_AGENT_PART);
                assert_api_key(&seen[0], "test-api-key");
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › API Key Authentication › should maintain backward compatibility with existing SigV4 authentication
            #[tokio::test]
            #[serial_test::serial]
            async fn should_maintain_backward_compatibility_with_existing_sig_v4_authentication() {
                let _env = empty_env();
                let mock = MockFetch::new(vec![converse_response()]);
                let provider = provider(
                    AmazonBedrockProviderSettings {
                        region: Some("eu-west-1".to_string()),
                        access_key_id: Some("test-access-key".to_string()),
                        secret_access_key: Some("test-secret-key".to_string()),
                        session_token: Some("test-session-token".to_string()),
                        ..Default::default()
                    },
                    &mock,
                );

                let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

                assert_sigv4(&seen, "test-access-key");
                assert_eq!(
                    converse_base_url(&seen),
                    "https://bedrock-runtime.eu-west-1.amazonaws.com"
                );
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › API Key Authentication › does not use AWS_SESSION_TOKEN from env when both access keys are passed as options
            ///
            /// The resolved credentials show as the signature's access key and
            /// the (absent) `x-amz-security-token`; the secret key is not
            /// observable on the wire.
            #[tokio::test]
            #[serial_test::serial]
            async fn does_not_use_aws_session_token_from_the_env_when_both_access_keys_are_passed()
            {
                let _env = mocked_env(&[(
                    "AWS_SESSION_TOKEN",
                    Some("env-session-token-should-not-be-used"),
                )]);
                let mock = MockFetch::new(vec![converse_response()]);
                let provider = provider(
                    AmazonBedrockProviderSettings {
                        region: Some("us-east-1".to_string()),
                        access_key_id: Some("from-options-ak".to_string()),
                        secret_access_key: Some("from-options-sk".to_string()),
                        ..Default::default()
                    },
                    &mock,
                );

                let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

                assert_sigv4(&seen, "from-options-ak");
                assert!(
                    !seen.headers.contains_key("x-amz-security-token"),
                    "{:?}",
                    seen.headers
                );
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › API Key Authentication › uses options.sessionToken when both access keys are passed as options
            #[tokio::test]
            #[serial_test::serial]
            async fn uses_options_session_token_when_both_access_keys_are_passed_as_options() {
                let _env = empty_env();
                let mock = MockFetch::new(vec![converse_response()]);
                let provider = provider(
                    AmazonBedrockProviderSettings {
                        region: Some("us-east-1".to_string()),
                        access_key_id: Some("from-options-ak".to_string()),
                        secret_access_key: Some("from-options-sk".to_string()),
                        session_token: Some("explicit-session".to_string()),
                        ..Default::default()
                    },
                    &mock,
                );

                let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

                assert_eq!(seen.headers["x-amz-security-token"], "explicit-session");
            }

            /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › createAmazonBedrock › API Key Authentication › should work with credential provider when no API key is provided
            #[tokio::test]
            #[serial_test::serial]
            async fn should_work_with_credential_provider_when_no_api_key_is_provided() {
                let _env = empty_env();
                let mock = MockFetch::new(vec![converse_response()]);
                let provider = provider(
                    AmazonBedrockProviderSettings {
                        region: Some("us-east-1".to_string()),
                        credential_provider: Some(dynamic_credential_provider()),
                        ..Default::default()
                    },
                    &mock,
                );

                let seen = generate(&provider, &mock, "anthropic.claude-v2").await;

                assert_sigv4(&seen, "dynamic-access-key");
            }
        }
    }

    mod provider_methods {
        use super::*;

        const APPLICATION_PROFILE_ARN: &str =
            "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/qibm5eutlkcy";

        fn default_provider() -> AmazonBedrockProvider {
            create_amazon_bedrock(AmazonBedrockProviderSettings::default())
                .expect("default settings are valid")
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › provider methods › should create an embedding model
        ///
        /// `toBeInstanceOf` is the return type, `BedrockEmbeddingModel`.
        #[test]
        fn should_create_an_embedding_model() {
            let model_id = "amazon.titan-embed-text-v1";

            let model: crate::bedrock::BedrockEmbeddingModel =
                default_provider().embedding(model_id);

            assert_eq!(model.model_id(), model_id);
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › provider methods › should pass chat model settings to the model
        #[test]
        fn should_pass_chat_model_settings_to_the_model() {
            let model = default_provider().chat_with_settings(
                APPLICATION_PROFILE_ARN,
                &AmazonBedrockChatModelSettings {
                    model_family: Some(AmazonBedrockChatModelFamily::Anthropic),
                },
            );

            assert_eq!(model.model_family.as_deref(), Some("anthropic"));
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › provider methods › should pass embedding model settings to the model
        #[test]
        fn should_pass_embedding_model_settings_to_the_model() {
            let model = default_provider().embedding_with_settings(
                APPLICATION_PROFILE_ARN,
                &AmazonBedrockEmbeddingModelSettings {
                    model_family: Some(AmazonBedrockEmbeddingModelFamily::Cohere),
                },
            );

            assert_eq!(
                model.model_family,
                Some(AmazonBedrockEmbeddingModelFamily::Cohere)
            );
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › provider methods › should create an image model
        ///
        /// `toBeInstanceOf` is the return type, `BedrockImageModel`.
        #[test]
        fn should_create_an_image_model() {
            let model_id = "amazon.titan-image-generator";

            let model: crate::bedrock::BedrockImageModel = default_provider().image(model_id);

            assert_eq!(model.model_id(), model_id);
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-provider.test.ts › AmazonBedrockProvider › provider methods › should create an image model via imageModel method
        ///
        /// `imageModel` is `Provider::image_model`, which returns the model as
        /// a trait object; its instance type is its provider and model id.
        #[test]
        fn should_create_an_image_model_via_image_model_method() {
            let model_id = "amazon.titan-image-generator";

            let model = default_provider()
                .image_model(model_id)
                .expect("an image model");

            assert_eq!(model.model_id(), model_id);
            assert_eq!(model.provider(), "amazon-bedrock");
        }

        // Not ported (sub-feature): "should expose anthropicTools".
        // `AmazonBedrockProvider` has no `tools` (the Anthropic provider tools)
        // member.
    }
}
