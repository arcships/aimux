use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};
use crate::test_support::mock_fetch::{Canned, MockFetch};
use aimux_core::shared::SharedProviderOptions;
use aimux_core::speech_model::{SpeechCallOptions, SpeechModel};

const SPEECH_URL: &str = "https://api.openai.com/v1/audio/speech";
const BASE_URL: &str = "https://api.openai.com/v1";
const PINNED_USER_AGENT: &str = "ai-sdk-openai/4.0.83";

/// The test server's `binary` response: 100 zero bytes as `audio/{format}`.
fn audio_response(format: &str) -> Canned {
    Canned {
        status: 200,
        headers: vec![("content-type".into(), format!("audio/{format}"))],
        body: vec![0; 100],
    }
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

fn provider_options(value: Value) -> SharedProviderOptions {
    serde_json::from_value(value).unwrap()
}

/// `toMatchObject`: every key of `expected` has the same value in `actual`.
fn assert_matches_object(actual: &Value, expected: &Value) {
    for (key, value) in expected.as_object().unwrap() {
        assert_eq!(actual.get(key), Some(value), "key {key} in {actual}");
    }
}

mod do_generate {
    use super::*;

    /// upstream: @ai-sdk/openai src/speech/openai-speech-model.test.ts › doGenerate › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let mock = MockFetch::new(vec![audio_response("mp3")]);
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
            .speech("tts-1")
            .do_generate(&SpeechCallOptions {
                headers: Some(HashMap::from([(
                    "Custom-Request-Header".to_string(),
                    "request-header-value".to_string(),
                )])),
                ..SpeechCallOptions::new("Hello from the AI SDK!")
            })
            .await
            .unwrap();

        let seen = &mock.seen()[0];
        assert_eq!(seen.url, SPEECH_URL);
        assert_matches_object(
            &serde_json::to_value(&seen.headers).unwrap(),
            &json!({
                "authorization": "Bearer test-api-key",
                "content-type": "application/json",
                "custom-provider-header": "provider-header-value",
                "custom-request-header": "request-header-value",
                "openai-organization": "test-organization",
                "openai-project": "test-project",
            }),
        );
        let user_agent = seen.headers.get("user-agent").cloned().unwrap_or_default();
        assert!(user_agent.contains(PINNED_USER_AGENT), "{user_agent}");
    }

    /// upstream: @ai-sdk/openai src/speech/openai-speech-model.test.ts › doGenerate › should pass provider options
    #[tokio::test]
    async fn should_pass_provider_options() {
        let mock = MockFetch::new(vec![audio_response("mp3")]);
        let model = provider_with(&mock, OpenAIProviderSettings::default()).speech("tts-1");

        model
            .do_generate(&SpeechCallOptions {
                speed: Some(1.5),
                provider_options: Some(provider_options(json!({
                    "openai": { "speed": 0.75, "instructions": "Speak slowly." }
                }))),
                ..SpeechCallOptions::new("Hello from the AI SDK!")
            })
            .await
            .unwrap();

        assert_matches_object(
            &mock.seen()[0].json_body(),
            &json!({ "speed": 0.75, "instructions": "Speak slowly." }),
        );
    }

    /// upstream: @ai-sdk/openai src/speech/openai-speech-model.test.ts › doGenerate › should preserve top-level options that provider options do not override
    #[tokio::test]
    async fn should_preserve_top_level_options_that_provider_options_do_not_override() {
        let mock = MockFetch::new(vec![audio_response("mp3")]);
        let model = provider_with(&mock, OpenAIProviderSettings::default()).speech("tts-1");

        model
            .do_generate(&SpeechCallOptions {
                speed: Some(1.5),
                provider_options: Some(provider_options(json!({
                    "openai": { "instructions": "Speak slowly." }
                }))),
                ..SpeechCallOptions::new("Hello from the AI SDK!")
            })
            .await
            .unwrap();

        assert_matches_object(
            &mock.seen()[0].json_body(),
            &json!({ "speed": 1.5, "instructions": "Speak slowly." }),
        );
    }
}
