//! Upstream `xai-provider.test.ts`.
//!
//! The upstream file mocks the model classes and checks their constructor
//! arguments. Here each fact is checked through the model the provider builds:
//! its `provider()` and `model_id()`, and the URL and headers of the request
//! it sends, recorded by a [`MockFetch`].
//!
//! The upstream `vi.mock` of `loadApiKey` returns `mock-api-key`; here the key
//! comes from `XAI_API_KEY`, which the tests set with [`EnvVar`]. The upstream
//! `VERSION` mock (`0.0.0-test`) has no counterpart: the user agent carries the
//! pinned `ai-sdk-xai/<version>` token (see [`assert_headers`]).
//!
//! The `imageModel` cases are not ported: aimux has no xAI image model.

use std::collections::HashMap;
use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_core::speech_model::{SpeechCallOptions, SpeechModel};
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_core::video_model::{VideoCallOptions, VideoModel};
use serde_json::{Value, json};

use crate::shared::aisdk_pins;
use crate::test_support::mock_fetch::{Canned, EnvVar, MockFetch, Seen};
use crate::xai::{API_KEY_ENV_VAR, XAIProvider};
use crate::{XAIProviderSettings, create_xai};

/// The key the upstream `loadApiKey` mock returns.
const MOCK_API_KEY: &str = "mock-api-key";

/// The upstream `loadApiKey` mock: the key every request uses, set through
/// `XAI_API_KEY`. Tests that hold it are `#[serial]`.
fn mock_api_key() -> EnvVar {
    EnvVar::set(API_KEY_ENV_VAR, Some(MOCK_API_KEY))
}

/// The `ai-sdk-xai/<version>` token of the pinned package: what the provider's
/// own headers carry.
fn user_agent() -> String {
    format!("ai-sdk-xai/{}", aisdk_pins::XAI.version)
}

/// A transport that answers every request with a 400, which fails the call
/// without a retry. The tests look at the request, not at the answer.
fn rejecting_fetch() -> Arc<MockFetch> {
    MockFetch::new(vec![Canned {
        status: 400,
        headers: vec![("content-type".into(), "application/json".into())],
        body: br#"{"error":{"message":"rejected by the test transport"}}"#.to_vec(),
    }])
}

/// The one request `fetch` recorded.
fn only_request(fetch: &MockFetch) -> Seen {
    let mut seen = fetch.seen();
    assert_eq!(seen.len(), 1, "the call sends exactly one request");
    seen.remove(0)
}

/// `toMatchObject` on the recorded headers: each expected header is present
/// with its value.
///
/// The `user-agent` is matched by token. Upstream checks the provider's own
/// headers, which carry `ai-sdk-xai/<version>` alone; the request also carries
/// the transport's `ai-sdk-provider-utils/<version>` token, which that check
/// never sees.
fn assert_headers(seen: &Seen, expected: Value) {
    for (name, value) in expected.as_object().expect("an object of headers") {
        let actual = seen.headers.get(name).map(String::as_str);
        let expected = value.as_str().expect("a string header value");
        if name == "user-agent" {
            assert!(
                actual.is_some_and(|agent| agent.split(' ').any(|token| token == expected)),
                "header {name}: {actual:?} does not carry {expected}"
            );
        } else {
            assert_eq!(actual, Some(expected), "header {name}");
        }
    }
}

/// The error of a call that the rejecting transport answered.
fn rejected<T>(result: Result<T, AiMuxError>) -> AiMuxError {
    match result {
        Ok(_) => panic!("the 400 answer of the test transport is an error"),
        Err(error) => error,
    }
}

/// `provider.videoModel(id)`: the trait factory.
fn video_model_of(provider: &XAIProvider, model_id: &str) -> Arc<dyn VideoModel> {
    provider
        .video_model(model_id)
        .expect("xAI has a video model")
        .expect("the video model builds")
}

/// `provider.speechModel()`: xAI's speech model takes no model id, so the
/// trait is given the empty id.
fn speech_model_of(provider: &XAIProvider) -> Arc<dyn SpeechModel> {
    provider
        .speech_model("")
        .expect("xAI has a speech model")
        .expect("the speech model builds")
}

/// `provider.transcriptionModel()`: no model id, as for speech.
fn transcription_model_of(provider: &XAIProvider) -> Arc<dyn TranscriptionModel> {
    provider
        .transcription_model("")
        .expect("xAI has a transcription model")
        .expect("the transcription model builds")
}

/// The call options of a transcription request. The audio is arbitrary: the
/// tests only look at the request the model sends.
fn transcription_call() -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(vec![1, 2, 3]), "audio/wav")
}

mod x_ai_provider {
    use super::*;

    mod create_xai {
        use super::*;

        /// upstream: @ai-sdk/xai src/xai-provider.test.ts › xAIProvider › createXAI › should create an XAIProvider instance with default options
        #[tokio::test]
        #[serial_test::serial]
        async fn should_create_an_xai_provider_instance_with_default_options() {
            // The upstream `loadApiKey` is called with no key, `XAI_API_KEY`
            // and "xAI API key": the key is unset here and comes from the
            // environment.
            let _key = EnvVar::set(API_KEY_ENV_VAR, None);
            let fetch = MockFetch::new(Vec::new());
            let provider = create_xai(XAIProviderSettings {
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .expect("creating an xAI provider does not fail");
            let model = provider.call("model-id");

            let error = model
                .do_generate(&CallOptions::default())
                .await
                .expect_err("no key is set");

            assert!(
                matches!(
                    &error,
                    AiMuxError::LoadApiKey { env_var, description }
                        if env_var == "XAI_API_KEY" && description == "xAI API key"
                ),
                "{error:?}"
            );
            assert!(
                fetch.seen().is_empty(),
                "the key is loaded before any request is sent"
            );
        }

        /// upstream: @ai-sdk/xai src/xai-provider.test.ts › xAIProvider › createXAI › should create an XAIProvider instance with custom options
        #[tokio::test]
        async fn should_create_an_xai_provider_instance_with_custom_options() {
            let fetch = rejecting_fetch();
            let provider = create_xai(XAIProviderSettings {
                api_key: Some("custom-key".to_string()),
                base_url: Some("https://custom.url".to_string()),
                headers: Some(HashMap::from([(
                    "Custom-Header".to_string(),
                    Some("value".to_string()),
                )])),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .expect("creating an xAI provider does not fail");
            let model = provider.call("model-id");

            rejected(model.do_generate(&CallOptions::default()).await);

            let seen = only_request(&fetch);
            assert_eq!(seen.url, "https://custom.url/responses");
            assert_headers(
                &seen,
                json!({
                    "authorization": "Bearer custom-key",
                    "custom-header": "value",
                }),
            );
        }

        /// upstream: @ai-sdk/xai src/xai-provider.test.ts › xAIProvider › createXAI › should return a responses model when called as a function
        #[test]
        fn should_return_a_responses_model_when_called_as_a_function() {
            let provider = create_xai(XAIProviderSettings::default())
                .expect("creating an xAI provider does not fail");
            let model = provider.call("foo-model-id");

            assert_eq!(model.provider(), "xai.responses");
            assert_eq!(model.model_id(), "foo-model-id");
        }
    }

    // describe('imageModel'): not ported. aimux has no xAI image model;
    // `Provider::image_model` answers `NoSuchModel`.

    mod video_model {
        use super::*;

        /// upstream: @ai-sdk/xai src/xai-provider.test.ts › xAIProvider › videoModel › should construct a video model with correct configuration
        #[tokio::test]
        #[serial_test::serial]
        async fn should_construct_a_video_model_with_correct_configuration() {
            let _key = mock_api_key();
            let fetch = rejecting_fetch();
            let provider = create_xai(XAIProviderSettings {
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .expect("creating an xAI provider does not fail");

            let model = video_model_of(&provider, "grok-imagine-video");

            assert_eq!(model.provider(), "xai.video");
            assert_eq!(model.model_id(), "grok-imagine-video");
            rejected(model.do_start(&VideoCallOptions::new("a prompt")).await);
            assert_eq!(
                only_request(&fetch).url,
                "https://api.x.ai/v1/videos/generations"
            );
        }

        /// upstream: @ai-sdk/xai src/xai-provider.test.ts › xAIProvider › videoModel › should use custom baseURL for video model
        #[tokio::test]
        #[serial_test::serial]
        async fn should_use_custom_base_url_for_video_model() {
            let _key = mock_api_key();
            let fetch = rejecting_fetch();
            let provider = create_xai(XAIProviderSettings {
                base_url: Some("https://custom.xai.api".to_string()),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .expect("creating an xAI provider does not fail");

            let model = video_model_of(&provider, "grok-imagine-video");

            rejected(model.do_start(&VideoCallOptions::new("a prompt")).await);
            assert_eq!(
                only_request(&fetch).url,
                "https://custom.xai.api/videos/generations"
            );
        }

        /// upstream: @ai-sdk/xai src/xai-provider.test.ts › xAIProvider › videoModel › should pass custom headers to video model
        #[tokio::test]
        #[serial_test::serial]
        async fn should_pass_custom_headers_to_video_model() {
            let _key = mock_api_key();
            let fetch = rejecting_fetch();
            let provider = create_xai(XAIProviderSettings {
                headers: Some(HashMap::from([(
                    "Custom-Header".to_string(),
                    Some("test-value".to_string()),
                )])),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .expect("creating an xAI provider does not fail");

            let model = video_model_of(&provider, "grok-imagine-video");

            rejected(model.do_start(&VideoCallOptions::new("a prompt")).await);
            assert_headers(
                &only_request(&fetch),
                json!({
                    "authorization": "Bearer mock-api-key",
                    "custom-header": "test-value",
                    "user-agent": user_agent(),
                }),
            );
        }

        /// upstream: @ai-sdk/xai src/xai-provider.test.ts › xAIProvider › videoModel › should create a video model via .video() alias
        #[test]
        fn should_create_a_video_model_via_video_alias() {
            let provider = create_xai(XAIProviderSettings::default())
                .expect("creating an xAI provider does not fail");

            let model = provider.video("grok-imagine-video");

            assert_eq!(model.provider(), "xai.video");
            assert_eq!(model.model_id(), "grok-imagine-video");
        }
    }

    mod speech_model {
        use super::*;

        /// upstream: @ai-sdk/xai src/xai-provider.test.ts › xAIProvider › speechModel › should construct a speech model with correct configuration
        #[tokio::test]
        #[serial_test::serial]
        async fn should_construct_a_speech_model_with_correct_configuration() {
            let _key = mock_api_key();
            let fetch = rejecting_fetch();
            let provider = create_xai(XAIProviderSettings {
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .expect("creating an xAI provider does not fail");

            let model = speech_model_of(&provider);

            assert_eq!(model.provider(), "xai.speech");
            assert_eq!(model.model_id(), "");
            rejected(model.do_generate(&SpeechCallOptions::new("hello")).await);
            assert_eq!(only_request(&fetch).url, "https://api.x.ai/v1/tts");
        }

        /// upstream: @ai-sdk/xai src/xai-provider.test.ts › xAIProvider › speechModel › should use custom baseURL and headers for speech models
        #[tokio::test]
        #[serial_test::serial]
        async fn should_use_custom_base_url_and_headers_for_speech_models() {
            let _key = mock_api_key();
            let fetch = rejecting_fetch();
            let provider = create_xai(XAIProviderSettings {
                base_url: Some("https://custom.xai.api".to_string()),
                headers: Some(HashMap::from([(
                    "Custom-Header".to_string(),
                    Some("test-value".to_string()),
                )])),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .expect("creating an xAI provider does not fail");

            let model = provider.speech();

            rejected(model.do_generate(&SpeechCallOptions::new("hello")).await);
            let seen = only_request(&fetch);
            assert_eq!(seen.url, "https://custom.xai.api/tts");
            assert_headers(
                &seen,
                json!({
                    "authorization": "Bearer mock-api-key",
                    "custom-header": "test-value",
                    "user-agent": user_agent(),
                }),
            );
        }

        /// upstream: @ai-sdk/xai src/xai-provider.test.ts › xAIProvider › speechModel › should create a speech model via .speech() alias
        #[test]
        fn should_create_a_speech_model_via_speech_alias() {
            let provider = create_xai(XAIProviderSettings::default())
                .expect("creating an xAI provider does not fail");

            let model = provider.speech();

            assert_eq!(model.provider(), "xai.speech");
        }
    }

    mod transcription_model {
        use super::*;

        /// upstream: @ai-sdk/xai src/xai-provider.test.ts › xAIProvider › transcriptionModel › should construct a transcription model with correct configuration
        #[tokio::test]
        #[serial_test::serial]
        async fn should_construct_a_transcription_model_with_correct_configuration() {
            let _key = mock_api_key();
            let fetch = rejecting_fetch();
            let provider = create_xai(XAIProviderSettings {
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .expect("creating an xAI provider does not fail");

            let model = transcription_model_of(&provider);

            assert_eq!(model.provider(), "xai.transcription");
            assert_eq!(model.model_id(), "");
            rejected(model.do_generate(&transcription_call()).await);
            assert_eq!(only_request(&fetch).url, "https://api.x.ai/v1/stt");
        }

        /// upstream: @ai-sdk/xai src/xai-provider.test.ts › xAIProvider › transcriptionModel › should use custom baseURL and headers for transcription models
        #[tokio::test]
        #[serial_test::serial]
        async fn should_use_custom_base_url_and_headers_for_transcription_models() {
            let _key = mock_api_key();
            let fetch = rejecting_fetch();
            let provider = create_xai(XAIProviderSettings {
                base_url: Some("https://custom.xai.api".to_string()),
                headers: Some(HashMap::from([(
                    "Custom-Header".to_string(),
                    Some("test-value".to_string()),
                )])),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .expect("creating an xAI provider does not fail");

            let model = provider.transcription();

            rejected(model.do_generate(&transcription_call()).await);
            let seen = only_request(&fetch);
            assert_eq!(seen.url, "https://custom.xai.api/stt");
            assert_headers(
                &seen,
                json!({
                    "authorization": "Bearer mock-api-key",
                    "custom-header": "test-value",
                    "user-agent": user_agent(),
                }),
            );
        }

        /// upstream: @ai-sdk/xai src/xai-provider.test.ts › xAIProvider › transcriptionModel › should create a transcription model via .transcription() alias
        #[test]
        fn should_create_a_transcription_model_via_transcription_alias() {
            let provider = create_xai(XAIProviderSettings::default())
                .expect("creating an xAI provider does not fail");

            let model = provider.transcription();

            assert_eq!(model.provider(), "xai.transcription");
        }
    }
}
