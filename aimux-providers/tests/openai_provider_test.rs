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
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;

use aimux_providers::{OpenAIConfig, OpenAIProvider};

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
    fn provider_name_is_openai() {
        let config = OpenAIConfig::new("test-key");
        let provider = OpenAIProvider::new(config);
        assert_eq!(provider.name(), "openai");
    }

    /// TS: default base URL is `https://api.openai.com/v1`.
    #[test]
    fn default_base_url() {
        let config = OpenAIConfig::new("test-key");
        assert_eq!(config.base_url, "https://api.openai.com/v1");
    }

    /// TS: custom base URL overrides the default.
    #[test]
    fn custom_base_url_override() {
        let config =
            OpenAIConfig::new("test-key").with_base_url("https://proxy.openai.example/v1/");
        // `with_base_url` strips trailing slash.
        assert_eq!(config.base_url, "https://proxy.openai.example/v1");
    }

    /// TS: `from_env` loads `OPENAI_API_KEY`.
    #[serial]
    #[test]
    fn from_env_loads_openai_api_key() {
        let saved = std::env::var("OPENAI_API_KEY").ok();
        unsafe { std::env::set_var("OPENAI_API_KEY", "env-test-key") };

        let config = OpenAIConfig::from_env();
        assert!(config.is_ok(), "from_env should succeed with env var set");

        unsafe {
            match saved {
                Some(v) => std::env::set_var("OPENAI_API_KEY", v),
                None => std::env::remove_var("OPENAI_API_KEY"),
            }
        }
    }

    /// TS: `from_env` fails without the env var.
    #[serial]
    #[test]
    fn from_env_fails_without_env_var() {
        let saved = std::env::var("OPENAI_API_KEY").ok();
        unsafe { std::env::remove_var("OPENAI_API_KEY") };

        let config = OpenAIConfig::from_env();
        assert!(config.is_err(), "from_env should fail without env var");

        unsafe {
            if let Some(v) = saved {
                std::env::set_var("OPENAI_API_KEY", v);
            }
        }
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

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-4o-mini");

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

        let config = OpenAIConfig::new("my-custom-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-4o");

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

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
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

        let config = OpenAIConfig::new("test-key")
            .with_base_url(server.uri())
            .with_org_id("org-123");
        let provider = OpenAIProvider::new(config);
        let model = provider.model("gpt-4o");

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

        let config = OpenAIConfig::new("test-key").with_base_url(server.uri());
        let provider = OpenAIProvider::new(config);
        let model = provider.model("o1");

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
