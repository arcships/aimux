use std::sync::Arc;

use serde_json::json;

use crate::openai::{OpenAIProviderSettings, create_openai};
use crate::test_support::mock_fetch::{Canned, EnvVar, MockFetch, RouteFetch};
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;

const BASE_URL_ENV_VAR: &str = "OPENAI_BASE_URL";

/// `createSuccessfulResponse()`.
fn successful_response() -> Canned {
    Canned::json(&json!({
        "object": "list",
        "data": [{ "object": "embedding", "index": 0, "embedding": [0.1, 0.2] }],
        "model": "text-embedding-3-small",
        "usage": { "prompt_tokens": 1, "total_tokens": 1 },
    }))
}

/// A transport that answers every request with HTTP 500.
fn failing_fetch() -> Arc<RouteFetch> {
    RouteFetch::new(|_| Canned {
        status: 500,
        ..Canned::json(&json!({ "error": { "message": "test error" } }))
    })
}

fn hello_prompt() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

/// Embeds `['hello']` through a provider built from `settings` and returns the
/// requested URLs.
async fn embedding_urls(settings: OpenAIProviderSettings) -> Vec<String> {
    let mock = MockFetch::new(vec![successful_response()]);
    let provider = create_openai(OpenAIProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(mock.transport()),
        ..settings
    })
    .unwrap();

    provider
        .embedding("text-embedding-3-small")
        .do_embed(&EmbeddingCallOptions::new("hello"))
        .await
        .unwrap();

    mock.seen().into_iter().map(|seen| seen.url).collect()
}

mod create_openai {
    use super::*;

    mod base_url_configuration {
        use super::*;

        /// upstream: @ai-sdk/openai src/openai-provider.test.ts › createOpenAI › baseURL configuration › uses the default OpenAI base URL when not provided
        #[tokio::test]
        #[serial_test::serial]
        async fn uses_the_default_openai_base_url_when_not_provided() {
            let _env = EnvVar::set(BASE_URL_ENV_VAR, None);

            let urls = embedding_urls(OpenAIProviderSettings::default()).await;

            assert_eq!(urls, ["https://api.openai.com/v1/embeddings"]);
        }

        /// upstream: @ai-sdk/openai src/openai-provider.test.ts › createOpenAI › baseURL configuration › uses OPENAI_BASE_URL when set
        #[tokio::test]
        #[serial_test::serial]
        async fn uses_openai_base_url_when_set() {
            let _env = EnvVar::set(BASE_URL_ENV_VAR, Some("https://proxy.openai.example/v1/"));

            let urls = embedding_urls(OpenAIProviderSettings::default()).await;

            assert_eq!(urls, ["https://proxy.openai.example/v1/embeddings"]);
        }

        /// upstream: @ai-sdk/openai src/openai-provider.test.ts › createOpenAI › baseURL configuration › prefers the baseURL option over OPENAI_BASE_URL
        #[tokio::test]
        #[serial_test::serial]
        async fn prefers_the_base_url_option_over_openai_base_url() {
            let _env = EnvVar::set(BASE_URL_ENV_VAR, Some("https://env.openai.example/v1"));

            let urls = embedding_urls(OpenAIProviderSettings {
                base_url: Some("https://option.openai.example/v1/".into()),
                ..Default::default()
            })
            .await;

            assert_eq!(urls, ["https://option.openai.example/v1/embeddings"]);
        }

        /// upstream: @ai-sdk/openai src/openai-provider.test.ts › createOpenAI › baseURL configuration › rejects an empty baseURL option during provider creation
        #[test]
        fn rejects_an_empty_base_url_option_during_provider_creation() {
            let error = create_openai(OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(String::new()),
                ..Default::default()
            })
            .err()
            .expect("createOpenAI throws");

            // `AI_InvalidArgumentError` with `argument: 'baseURL'`: aimux's
            // invalid-argument error carries only the message.
            match error {
                AiMuxError::InvalidArgument { message, .. } => {
                    assert_eq!(message, "baseURL must be a non-empty string.");
                }
                other => panic!("expected an invalid argument error, got {other:?}"),
            }
        }

        /// upstream: @ai-sdk/openai src/openai-provider.test.ts › createOpenAI › baseURL configuration › rejects an empty OPENAI_BASE_URL during provider creation
        ///
        /// aimux reads `OPENAI_BASE_URL` at creation like the SDK, but by
        /// contract (`create_openai` docs, CHANGELOG "no longer panic on an
        /// invalid `OPENAI_BASE_URL`") reports an invalid value from every call
        /// instead of failing the factory, so the error is checked on the
        /// first call; it carries the SDK's message after the variable's name.
        #[tokio::test]
        #[serial_test::serial]
        async fn rejects_an_empty_openai_base_url_during_provider_creation() {
            let _env = EnvVar::set(BASE_URL_ENV_VAR, Some(""));
            let mock = MockFetch::new(vec![successful_response()]);

            let provider = create_openai(OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                fetch: Some(mock.transport()),
                ..Default::default()
            })
            .expect("an invalid OPENAI_BASE_URL is reported per call");
            let error = provider
                .embedding("text-embedding-3-small")
                .do_embed(&EmbeddingCallOptions::new("hello"))
                .await
                .expect_err("Expected the call to reject an empty base URL.");

            match error {
                AiMuxError::InvalidArgument { message, .. } => {
                    assert!(message.starts_with("OPENAI_BASE_URL: "), "{message}");
                    assert!(
                        message.ends_with("baseURL must be a non-empty string."),
                        "{message}"
                    );
                }
                other => panic!("expected an invalid argument error, got {other:?}"),
            }
            assert!(mock.seen().is_empty());
        }

        /// upstream: @ai-sdk/openai src/openai-provider.test.ts › createOpenAI › baseURL configuration › uses the Responses API for the default language model
        #[tokio::test]
        async fn uses_the_responses_api_for_the_default_language_model() {
            let fetch = failing_fetch();
            let provider = create_openai(OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some("https://proxy.openai.example/v1/".into()),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap();

            let _ = provider
                .call("gpt-4o-mini")
                .do_generate(&hello_prompt())
                .await;

            let urls: Vec<String> = fetch.seen().into_iter().map(|seen| seen.url).collect();
            assert_eq!(urls, ["https://proxy.openai.example/v1/responses"]);
        }

        /// upstream: @ai-sdk/openai src/openai-provider.test.ts › createOpenAI › baseURL configuration › uses the Chat Completions API for chat models
        #[tokio::test]
        async fn uses_the_chat_completions_api_for_chat_models() {
            let fetch = failing_fetch();
            let provider = create_openai(OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some("https://proxy.openai.example/v1/".into()),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap();

            let _ = provider
                .chat("gpt-4o-mini")
                .do_generate(&hello_prompt())
                .await;

            let urls: Vec<String> = fetch.seen().into_iter().map(|seen| seen.url).collect();
            assert_eq!(urls, ["https://proxy.openai.example/v1/chat/completions"]);
        }
    }
}
