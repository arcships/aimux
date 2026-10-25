//! Port of `@ai-sdk/openai-compatible` 3.0.62
//! `src/openai-compatible-provider.test.ts`.
//!
//! Upstream mocks the model classes and reads the config each constructor
//! was called with. Here the config is the one the returned model holds
//! (`model.config`). `provider(modelId)` and `provider.languageModel(modelId)`
//! are `call` / `Provider::language_model`, which return an
//! `Arc<dyn LanguageModel>` whose config cannot be read; for them the setting
//! is observed in the request the model sends.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::Value;

use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::types::ProviderMetadata;

use crate::openai_compatible::completion::{CompletionConfig, OpenAICompatibleCompletionModel};
use crate::openai_compatible::config::CompatModelConfig;
use crate::openai_compatible::{
    MetadataExtractor, OpenAICompatibleProvider, OpenAICompatibleProviderSettings,
    StreamMetadataExtractor, create_openai_compatible,
};
use crate::test_support::mock_fetch::{Canned, MockFetch};

/// The user-agent part the provider headers carry: the pinned
/// `ai-sdk-openai-compatible/<version>` (upstream mocks `0.0.0-test`).
const USER_AGENT: &str = "ai-sdk-openai-compatible/3.0.62";

fn header_opt_map(entries: &[(&str, &str)]) -> aimux_provider_utils::HeaderMapOpt {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_string(), Some((*value).to_string())))
        .collect()
}

fn expected_headers(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect()
}

/// `config.headers()`, with the user agent checked against the pinned value
/// and left out.
async fn config_headers(config: &CompatModelConfig) -> BTreeMap<String, String> {
    let mut headers: BTreeMap<String, String> = config
        .headers
        .resolve()
        .await
        .unwrap()
        .into_iter()
        .filter_map(|(name, value)| value.map(|value| (name, value)))
        .collect();
    assert_eq!(
        headers.remove("user-agent").as_deref(),
        Some(USER_AGENT),
        "pinned user agent"
    );
    headers
}

/// `config.url({ path })`.
fn config_url(config: &CompatModelConfig, path: &str) -> String {
    config.url(path).unwrap().0
}

fn completion_config(model: &OpenAICompatibleCompletionModel) -> &CompatModelConfig {
    match &model.config {
        CompletionConfig::Compatible(config) => config,
        CompletionConfig::Native(_) => unreachable!("a compatible model"),
    }
}

/// A server-sent-events chat response; the request body is what the probes
/// read, so the stream itself is never consumed.
fn chat_stream_response() -> Canned {
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: b"data: [DONE]\n\n".to_vec(),
    }
}

/// The body a `doStream` call of `model` sends with `options`.
async fn sent_stream_body(
    model: &dyn LanguageModel,
    mock: &Arc<MockFetch>,
    options: &CallOptions,
) -> Value {
    model.do_stream(options).await.unwrap();
    mock.seen().last().expect("a request").json_body()
}

fn prompt_options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

/// The `includeUsage` a language model got, read from the
/// `stream_options` it sends (`undefined` and `false` send none).
async fn sends_include_usage(model: &dyn LanguageModel, mock: &Arc<MockFetch>) -> Option<bool> {
    let body = sent_stream_body(model, mock, &prompt_options()).await;
    body.get("stream_options")
        .map(|stream_options| stream_options["include_usage"].as_bool().unwrap())
}

mod open_ai_compatible_provider {
    use super::*;

    mod create_open_ai_compatible {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/openai-compatible-provider.test.ts › OpenAICompatibleProvider › createOpenAICompatible › should create provider with correct configuration
        #[tokio::test]
        async fn should_create_provider_with_correct_configuration() {
            let options = OpenAICompatibleProviderSettings {
                base_url: "https://api.example.com".into(),
                name: "test-provider".into(),
                api_key: Some("test-api-key".into()),
                headers: Some(header_opt_map(&[("custom-header", "value")])),
                query_params: Some(HashMap::from([(
                    "Custom-Param".to_string(),
                    "value".to_string(),
                )])),
                ..Default::default()
            };

            let provider = create_openai_compatible(options).unwrap();
            // `provider('model-id')` is `call`, which returns `chat('model-id')`.
            assert_eq!(provider.call("model-id").provider(), "test-provider.chat");
            let model = provider.chat("model-id");

            let config = &model.config;
            let headers = config_headers(config).await;

            assert_eq!(
                headers,
                expected_headers(&[
                    ("authorization", "Bearer test-api-key"),
                    ("custom-header", "value"),
                ])
            );
            assert_eq!(config.provider, "test-provider.chat");
            assert_eq!(
                config_url(config, "/v1/chat"),
                "https://api.example.com/v1/chat?Custom-Param=value"
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/openai-compatible-provider.test.ts › OpenAICompatibleProvider › createOpenAICompatible › should create headers without authorization when no apiKey provided
        #[tokio::test]
        async fn should_create_headers_without_authorization_when_no_api_key_provided() {
            let options = OpenAICompatibleProviderSettings {
                base_url: "https://api.example.com".into(),
                name: "test-provider".into(),
                headers: Some(header_opt_map(&[("custom-header", "value")])),
                ..Default::default()
            };

            let provider = create_openai_compatible(options).unwrap();
            // `provider('model-id')` is `call`, which returns `chat('model-id')`.
            assert_eq!(provider.call("model-id").provider(), "test-provider.chat");
            let model = provider.chat("model-id");

            let headers = config_headers(&model.config).await;

            assert_eq!(headers, expected_headers(&[("custom-header", "value")]));
        }
    }

    mod model_creation_methods {
        use super::*;

        fn default_options() -> OpenAICompatibleProviderSettings {
            OpenAICompatibleProviderSettings {
                base_url: "https://api.example.com".into(),
                name: "test-provider".into(),
                api_key: Some("test-api-key".into()),
                headers: Some(header_opt_map(&[("custom-header", "value")])),
                query_params: Some(HashMap::from([(
                    "Custom-Param".to_string(),
                    "value".to_string(),
                )])),
                ..Default::default()
            }
        }

        /// upstream: @ai-sdk/openai-compatible src/openai-compatible-provider.test.ts › OpenAICompatibleProvider › model creation methods › should create chat model with correct configuration
        #[tokio::test]
        async fn should_create_chat_model_with_correct_configuration() {
            let provider = create_openai_compatible(default_options()).unwrap();

            let model = provider.chat("chat-model");

            let config = &model.config;
            let headers = config_headers(config).await;

            assert_eq!(
                headers,
                expected_headers(&[
                    ("authorization", "Bearer test-api-key"),
                    ("custom-header", "value"),
                ])
            );
            assert_eq!(config.provider, "test-provider.chat");
            assert_eq!(
                config_url(config, "/v1/chat"),
                "https://api.example.com/v1/chat?Custom-Param=value"
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/openai-compatible-provider.test.ts › OpenAICompatibleProvider › model creation methods › should create completion model with correct configuration
        #[tokio::test]
        async fn should_create_completion_model_with_correct_configuration() {
            let provider = create_openai_compatible(default_options()).unwrap();

            let model = provider.completion("completion-model");

            let config = completion_config(&model);
            let headers = config_headers(config).await;

            assert_eq!(
                headers,
                expected_headers(&[
                    ("authorization", "Bearer test-api-key"),
                    ("custom-header", "value"),
                ])
            );
            assert_eq!(config.provider, "test-provider.completion");
            assert_eq!(
                config_url(config, "/v1/completions"),
                "https://api.example.com/v1/completions?Custom-Param=value"
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/openai-compatible-provider.test.ts › OpenAICompatibleProvider › model creation methods › should create embedding model with correct configuration
        #[tokio::test]
        async fn should_create_embedding_model_with_correct_configuration() {
            let provider = create_openai_compatible(default_options()).unwrap();

            let model = provider.embedding("embedding-model");

            let config = &model.config;
            let headers = config_headers(config).await;

            assert_eq!(
                headers,
                expected_headers(&[
                    ("authorization", "Bearer test-api-key"),
                    ("custom-header", "value"),
                ])
            );
            assert_eq!(config.provider, "test-provider.embedding");
            assert_eq!(
                config_url(config, "/v1/embeddings"),
                "https://api.example.com/v1/embeddings?Custom-Param=value"
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/openai-compatible-provider.test.ts › OpenAICompatibleProvider › model creation methods › should use languageModel as default when called as function
        #[test]
        fn should_use_language_model_as_default_when_called_as_function() {
            let provider = create_openai_compatible(default_options()).unwrap();

            let model = provider.call("model-id");

            assert_eq!(model.model_id(), "model-id");
            assert_eq!(model.provider(), "test-provider.chat");
        }

        /// upstream: @ai-sdk/openai-compatible src/openai-compatible-provider.test.ts › OpenAICompatibleProvider › model creation methods › should create URL without query parameters when queryParams is not specified
        #[test]
        fn should_create_url_without_query_parameters_when_query_params_is_not_specified() {
            let options = OpenAICompatibleProviderSettings {
                base_url: "https://api.example.com".into(),
                name: "test-provider".into(),
                api_key: Some("test-api-key".into()),
                ..Default::default()
            };

            let provider = create_openai_compatible(options).unwrap();
            // `provider('model-id')` is `call`, which returns `chat('model-id')`.
            assert_eq!(provider.call("model-id").provider(), "test-provider.chat");
            let model = provider.chat("model-id");

            assert_eq!(
                config_url(&model.config, "/v1/chat"),
                "https://api.example.com/v1/chat"
            );
        }
    }

    mod include_usage_setting {
        use super::*;

        fn options(
            mock: &Arc<MockFetch>,
            include_usage: Option<bool>,
        ) -> OpenAICompatibleProviderSettings {
            OpenAICompatibleProviderSettings {
                base_url: "https://api.example.com".into(),
                name: "test-provider".into(),
                include_usage,
                fetch: Some(mock.transport()),
                ..Default::default()
            }
        }

        /// upstream: @ai-sdk/openai-compatible src/openai-compatible-provider.test.ts › OpenAICompatibleProvider › includeUsage setting › should pass includeUsage: true to all model types when specified in provider settings
        #[tokio::test]
        async fn should_pass_include_usage_true_to_all_model_types_when_specified_in_provider_settings()
         {
            let mock = MockFetch::new(vec![chat_stream_response()]);
            let provider = create_openai_compatible(options(&mock, Some(true))).unwrap();

            let chat = provider.chat("chat-model");
            assert!(chat.config.chat.include_usage);

            let completion = provider.completion("completion-model");
            assert!(completion_config(&completion).chat.include_usage);

            let model = provider.call("model-id");
            assert_eq!(sends_include_usage(model.as_ref(), &mock).await, Some(true));
        }

        /// upstream: @ai-sdk/openai-compatible src/openai-compatible-provider.test.ts › OpenAICompatibleProvider › includeUsage setting › should pass includeUsage: false to all model types when specified in provider settings
        #[tokio::test]
        async fn should_pass_include_usage_false_to_all_model_types_when_specified_in_provider_settings()
         {
            let mock = MockFetch::new(vec![chat_stream_response()]);
            let provider = create_openai_compatible(options(&mock, Some(false))).unwrap();

            let chat = provider.chat("chat-model");
            assert!(!chat.config.chat.include_usage);

            let completion = provider.completion("completion-model");
            assert!(!completion_config(&completion).chat.include_usage);

            let model = provider.call("model-id");
            // `false` and `undefined` both send no `stream_options`.
            assert_eq!(sends_include_usage(model.as_ref(), &mock).await, None);
        }

        /// upstream: @ai-sdk/openai-compatible src/openai-compatible-provider.test.ts › OpenAICompatibleProvider › includeUsage setting › should pass includeUsage: undefined to all model types when not specified in provider settings
        #[tokio::test]
        async fn should_pass_include_usage_undefined_to_all_model_types_when_not_specified() {
            let mock = MockFetch::new(vec![chat_stream_response()]);
            let provider = create_openai_compatible(options(&mock, None)).unwrap();

            // The model config holds a `bool`: upstream's `undefined` is its
            // `false` default.
            let chat = provider.chat("chat-model");
            assert!(!chat.config.chat.include_usage);

            let completion = provider.completion("completion-model");
            assert!(!completion_config(&completion).chat.include_usage);

            let model = provider.call("model-id");
            assert_eq!(sends_include_usage(model.as_ref(), &mock).await, None);
        }
    }

    mod metadata_extractor_setting {
        use super::*;

        struct MockExtractor;

        struct MockStreamExtractor;

        impl MetadataExtractor for MockExtractor {
            fn extract_metadata(&self, _parsed_body: &Value) -> Option<ProviderMetadata> {
                None
            }

            fn create_stream_extractor(&self) -> Box<dyn StreamMetadataExtractor> {
                Box::new(MockStreamExtractor)
            }
        }

        impl StreamMetadataExtractor for MockStreamExtractor {
            fn process_chunk(&mut self, _parsed_chunk: &Value) {}

            fn build_metadata(&self) -> Option<ProviderMetadata> {
                None
            }
        }

        /// upstream: @ai-sdk/openai-compatible src/openai-compatible-provider.test.ts › OpenAICompatibleProvider › metadataExtractor setting › should pass metadataExtractor to chat model
        #[test]
        fn should_pass_metadata_extractor_to_chat_model() {
            let mock_extractor: Arc<dyn MetadataExtractor> = Arc::new(MockExtractor);

            let provider: OpenAICompatibleProvider =
                create_openai_compatible(OpenAICompatibleProviderSettings {
                    base_url: "https://api.example.com".into(),
                    name: "test-provider".into(),
                    metadata_extractor: Some(mock_extractor.clone()),
                    ..Default::default()
                })
                .unwrap();

            let chat = provider.chat("chat-model");

            assert!(Arc::ptr_eq(
                chat.config.chat.metadata_extractor.as_ref().unwrap(),
                &mock_extractor
            ));
        }
    }
}
