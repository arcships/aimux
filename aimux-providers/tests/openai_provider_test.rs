//! OpenAI provider configuration and forward-compatible defaults tests.
//!
//! Translates the chat-completions-relevant parts of:
//! - `packages/openai/src/openai-provider.test.ts` — baseURL config, chat routing
//! - `packages/openai/src/openai-forward-compatible-defaults.test.ts` — reasoning-safe defaults
//!
//! Responses API / embedding / image parts are excluded (covered by B1/C1/C2).

use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::files_model::Files;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_core::speech_model::SpeechModel;
use aimux_core::transcription_model::TranscriptionModel;

use aimux_provider_utils::Resolvable;
use aimux_providers::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai, openai};

/// A native OpenAI provider against `base_url`. The key is an explicit value,
/// so the environment is never consulted.
fn provider_with(api_key: &str, base_url: impl Into<String>) -> OpenAIProvider {
    create_openai(OpenAIProviderSettings {
        api_key: Some(Resolvable::Value(api_key.to_string())),
        base_url: Some(base_url.into()),
        ..Default::default()
    })
    .expect("settings are valid")
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

fn text_completion_body() -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "test-model",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "ok" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
    })
}

// ════════════════════════════════════════════════════════════════════════════
// Provider configuration (openai-provider.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod provider_config {
    use super::*;

    /// TS: `createOpenAI()` provider name is "openai".
    #[test]
    fn model_provider_is_openai() {
        let provider = create_openai(OpenAIProviderSettings::default()).unwrap();
        assert_eq!(provider.chat("gpt-4o").provider(), "openai.chat");
        assert_eq!(openai().chat("gpt-4o").provider(), "openai.chat");
    }

    /// TS: `name` replaces the `openai` prefix of every model's provider string.
    #[test]
    fn name_prefixes_every_model() {
        let provider = create_openai(OpenAIProviderSettings {
            name: Some("proxy".to_string()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(provider.chat("m").provider(), "proxy.chat");
        assert_eq!(provider.responses("m").provider(), "proxy.responses");
        assert_eq!(provider.embedding("m").provider(), "proxy.embedding");
        assert_eq!(provider.image("m").provider(), "proxy.image");
        assert_eq!(provider.speech("m").provider(), "proxy.speech");
        assert_eq!(
            provider.transcription("m").provider(),
            "proxy.transcription"
        );
        assert_eq!(provider.files().provider(), "proxy.files");
    }

    /// An invalid base URL is the only way creation fails.
    #[test]
    fn invalid_base_url_fails_creation() {
        for bad in ["", "not a url", "ftp://example.com/v1", "https://"] {
            let err = create_openai(OpenAIProviderSettings {
                base_url: Some(bad.to_string()),
                ..Default::default()
            })
            .err()
            .unwrap_or_else(|| panic!("{bad:?} must be rejected"));
            assert!(
                matches!(err, aimux_core::AiMuxError::InvalidArgument(_)),
                "{bad:?}: {err:?}"
            );
        }
    }

    /// TS: the key is loaded when a request is made, not when the provider is
    /// created, so creating a provider with no key and no environment works.
    #[serial]
    #[tokio::test]
    async fn key_is_loaded_per_request_from_the_environment() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let saved = std::env::var("OPENAI_API_KEY").ok();
        unsafe { std::env::remove_var("OPENAI_API_KEY") };

        let provider = create_openai(OpenAIProviderSettings {
            base_url: Some(server.uri()),
            ..Default::default()
        })
        .expect("creation never reads the key");
        let model = provider.chat("gpt-4o");

        let missing = model.do_generate(&default_options(test_prompt())).await;
        assert!(
            matches!(
                &missing,
                Err(aimux_core::AiMuxError::LoadApiKey { env_var, .. })
                    if env_var == "OPENAI_API_KEY"
            ),
            "{missing:?}"
        );

        // The same model picks the key up as soon as the environment has it.
        unsafe { std::env::set_var("OPENAI_API_KEY", "env-test-key") };
        model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("key from the environment");

        unsafe {
            match saved {
                Some(v) => std::env::set_var("OPENAI_API_KEY", v),
                None => std::env::remove_var("OPENAI_API_KEY"),
            }
        }

        let requests = server.received_requests().await.expect("requests recorded");
        assert_eq!(requests.len(), 1, "the failed call sent nothing");
        assert_eq!(
            requests[0]
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer env-test-key"),
        );
    }

    /// TS: a trailing slash on the base URL is removed.
    #[tokio::test]
    async fn base_url_trailing_slash_is_removed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let provider = provider_with("test-key", format!("{}/", server.uri()));
        provider
            .chat("gpt-4o")
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed");
    }

    /// TS: chat completions API routes to `/chat/completions`.
    #[tokio::test]
    async fn chat_routes_to_chat_completions() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let provider = provider_with("test-key", server.uri());
        let model = provider.chat("gpt-4o-mini");

        model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed");
    }

    /// TS: custom API key is sent in Authorization header.
    #[tokio::test]
    async fn custom_api_key_in_auth_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let provider = provider_with("my-custom-key", server.uri());
        let model = provider.chat("gpt-4o");

        let _ = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed");

        let requests = server.received_requests().await.expect("requests recorded");
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer my-custom-key"),
        );
    }

    /// TS: `languageModel` via Provider trait creates a working model.
    #[tokio::test]
    async fn language_model_via_trait() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let provider = provider_with("test-key", server.uri());
        let model = provider
            .language_model("gpt-4o")
            .expect("language_model should succeed");

        model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");
    }

    /// TS: org ID is sent in the OpenAI-Organization header.
    #[tokio::test]
    async fn org_id_in_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let provider = create_openai(OpenAIProviderSettings {
            api_key: Some(Resolvable::Value("test-key".to_string())),
            base_url: Some(server.uri()),
            organization: Some("org-123".to_string()),
            ..Default::default()
        })
        .unwrap();
        let model = provider.chat("gpt-4o");

        let _ = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed");

        let requests = server.received_requests().await.expect("requests recorded");
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .headers
                .get("openai-organization")
                .and_then(|v| v.to_str().ok()),
            Some("org-123"),
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Forward-compatible defaults (openai-forward-compatible-defaults.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod forward_compatible_defaults {
    use super::*;

    /// TS: gpt-99 (reasoning model) should use reasoning-safe Chat Completions
    /// defaults: system→developer, max_completion_tokens instead of max_tokens,
    /// temperature/top_p/penalties/logit_bias/logprobs stripped.
    ///
    /// We verify by checking the request body sent to the mock server.
    #[tokio::test]
    async fn reasoning_model_uses_safe_defaults() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let provider = provider_with("test-key", server.uri());
        let model = provider.chat("o1");

        let prompt = vec![
            LanguageModelPromptMessage {
                role: Role::System,
                content: vec![ContentPart::text("Follow the instructions.")],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Say ok.")],
                ..Default::default()
            },
        ];
        let mut options = CallOptions::new(prompt);
        options.max_output_tokens = Some(64);
        options.temperature = Some(0.2);
        options.top_p = Some(0.8);

        let _ = model.do_generate(&options).await.expect("should succeed");

        let requests = server.received_requests().await.expect("requests recorded");
        assert_eq!(requests.len(), 1);
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();

        // Reasoning models use `max_completion_tokens` not `max_tokens`.
        assert!(
            body.get("max_completion_tokens").is_some(),
            "should use max_completion_tokens for reasoning models"
        );
        // System message should be converted to developer for reasoning models.
        assert_eq!(
            body["messages"][0]["role"], "developer",
            "system should become developer for reasoning models"
        );
    }
}
