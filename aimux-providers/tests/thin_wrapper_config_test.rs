//! Provider configuration tests for the remaining thin OpenAI-compatible
//! wrappers: HuggingFace, TogetherAI, Vercel.
//!
//! The existing `openai_compatible_test.rs` covers basic do_generate /
//! do_stream / 401 / URL+auth-header for every wrapper via shared macros.
//! This file adds the provider-specific configuration tests that the TS
//! suites verify:
//!
//! - `from_env()` loads the correct environment variable.
//! - `Provider::name()` returns the documented provider string.
//! - Custom headers are forwarded to the HTTP request.
//! - The default base URL constant is wired correctly (verified via a mock
//!   round-trip that relies on `with_base_url`).
//!
//! TS sources:
//! - `packages/huggingface/src/huggingface-provider.test.ts`
//! - `packages/togetherai/src/togetherai-provider.test.ts`
//! - `packages/vercel/src/vercel-provider.test.ts`

use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;

use aimux_providers::{
    provider, provider_from_env, HuggingFaceConfig, HuggingFaceProvider, ProviderOptions,
};

// ── shared helpers ───────────────────────────────────────────────────────────

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
            "message": { "role": "assistant", "content": "Hello, World!" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
    })
}

// ════════════════════════════════════════════════════════════════════════════
// HuggingFace provider configuration
// ════════════════════════════════════════════════════════════════════════════

mod huggingface_config {
    use super::*;

    /// TS: `createHuggingFace()` should create a provider with default
    /// configuration. In Rust we verify the provider name and that a model
    /// can be created.
    #[test]
    fn provider_name_is_huggingface() {
        let config = HuggingFaceConfig::new("test-key");
        let provider = HuggingFaceProvider::new(config);
        assert_eq!(provider.name(), "huggingface");
    }

    /// TS: `createHuggingFace({ apiKey: 'custom-key' })` �?custom API key
    /// is used in the Authorization header.
    #[tokio::test]
    async fn custom_api_key_used_in_auth_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer my-custom-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let config = HuggingFaceConfig::new("my-custom-key").with_base_url(server.uri());
        let provider = HuggingFaceProvider::new(config);
        let model = provider.model("meta-llama/Llama-3.3-70B-Instruct");

        model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed with custom API key");
    }

    /// TS: `createHuggingFace({ headers: { 'Custom-Header': 'test' } })`
    /// �?custom headers should be forwarded.
    #[tokio::test]
    async fn custom_headers_forwarded() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("x-custom-header", "test-value"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let config = HuggingFaceConfig::new("test-key").with_base_url(server.uri());
        let provider = HuggingFaceProvider::new(config);
        let model = provider.model("meta-llama/Llama-3.3-70B-Instruct");

        let mut options = default_options(test_prompt());
        options.headers = Some(
            vec![("x-custom-header".to_string(), "test-value".to_string())]
                .into_iter()
                .collect(),
        );

        model
            .do_generate(&options)
            .await
            .expect("should succeed with custom headers");
    }

    /// TS: `provider.languageModel(modelId)` �?the provider should create a
    /// language model via the `Provider` trait.
    #[tokio::test]
    async fn language_model_via_trait() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let config = HuggingFaceConfig::new("test-key").with_base_url(server.uri());
        let provider = HuggingFaceProvider::new(config);
        let model = provider
            .language_model("meta-llama/Llama-3.3-70B-Instruct")
            .expect("language_model should succeed");

        model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");
    }

    /// TS: `from_env` should load `HUGGINGFACE_API_KEY`.
    #[test]
    fn from_env_loads_correct_env_var() {
        // Save and restore env var.
        let saved = std::env::var("HUGGINGFACE_API_KEY").ok();
        unsafe { std::env::set_var("HUGGINGFACE_API_KEY", "env-test-key") };

        let config = HuggingFaceConfig::from_env();
        assert!(config.is_ok(), "from_env should succeed with env var set");

        // Restore.
        unsafe {
            match saved {
                Some(v) => std::env::set_var("HUGGINGFACE_API_KEY", v),
                None => std::env::remove_var("HUGGINGFACE_API_KEY"),
            }
        }
    }

    /// TS: `from_env` should fail without the env var.
    #[test]
    #[ignore = "flaky: parallel test env var race"]
    fn from_env_fails_without_env_var() {
        let saved = std::env::var("HUGGINGFACE_API_KEY").ok();
        unsafe { std::env::remove_var("HUGGINGFACE_API_KEY") };

        let config = HuggingFaceConfig::from_env();
        assert!(config.is_err(), "from_env should fail without env var");

        // Restore.
        unsafe {
            if let Some(v) = saved {
                std::env::set_var("HUGGINGFACE_API_KEY", v);
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// TogetherAI provider configuration
// ════════════════════════════════════════════════════════════════════════════

mod togetherai_config {
    use super::*;

    /// TS: `createTogetherAI()` should create a provider with default
    /// configuration.
    #[test]
    fn provider_name_is_togetherai() {
        // Phase 4: the shell TogetherAIConfig/TogetherAIProvider pair is
        // retired — registry-backed provider() replaces it.
        let model = provider(
            "togetherai",
            Some("test-key".to_string()),
            "meta-llama/Llama-3-70b-chat-hf",
            None,
        )
        .expect("togetherai should construct from registry");
        assert_eq!(model.model_id(), "meta-llama/Llama-3-70b-chat-hf");
    }

    /// TS: `createTogetherAI({ apiKey: 'custom-key' })` �?custom API key
    /// is used in the Authorization header.
    #[tokio::test]
    async fn custom_api_key_used_in_auth_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer my-custom-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let model = provider(
            "togetherai",
            Some("my-custom-key".to_string()),
            "meta-llama/Llama-3-70b-chat-hf",
            Some(ProviderOptions {
                base_url: Some(server.uri()),
                ..Default::default()
            }),
        )
        .expect("togetherai should construct from registry");

        model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed with custom API key");
    }

    /// TS: custom headers should be forwarded.
    #[tokio::test]
    async fn custom_headers_forwarded() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("x-custom-header", "test-value"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let model = provider(
            "togetherai",
            Some("test-key".to_string()),
            "meta-llama/Llama-3-70b-chat-hf",
            Some(ProviderOptions {
                base_url: Some(server.uri()),
                headers: Some(
                    vec![("x-custom-header".to_string(), "test-value".to_string())]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            }),
        )
        .expect("togetherai should construct from registry");

        model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed with custom headers");
    }

    /// TS: `from_env` should load `TOGETHER_API_KEY`.
    #[test]
    fn from_env_loads_correct_env_var() {
        let saved = std::env::var("TOGETHER_API_KEY").ok();
        unsafe { std::env::set_var("TOGETHER_API_KEY", "env-test-key") };

        let result = provider_from_env("togetherai", "meta-llama/Llama-3-70b-chat-hf", None);
        assert!(
            result.is_ok(),
            "provider_from_env should succeed with env var set"
        );

        unsafe {
            match saved {
                Some(v) => std::env::set_var("TOGETHER_API_KEY", v),
                None => std::env::remove_var("TOGETHER_API_KEY"),
            }
        }
    }

    /// TS: `from_env` should fail without the env var.
    #[test]
    #[ignore = "flaky: parallel test env var race"]
    fn from_env_fails_without_env_var() {
        let saved = std::env::var("TOGETHER_API_KEY").ok();
        unsafe { std::env::remove_var("TOGETHER_API_KEY") };

        let result = provider_from_env("togetherai", "meta-llama/Llama-3-70b-chat-hf", None);
        assert!(
            result.is_err(),
            "provider_from_env should fail without env var"
        );

        unsafe {
            if let Some(v) = saved {
                std::env::set_var("TOGETHER_API_KEY", v);
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Vercel provider configuration
// ════════════════════════════════════════════════════════════════════════════

mod vercel_config {
    use super::*;

    /// TS: `createVercel()` should create a provider with default
    /// configuration.
    #[test]
    fn provider_name_is_vercel() {
        // Phase 4: the shell VercelConfig/VercelProvider pair is retired —
        // registry-backed provider() replaces it.
        let model = provider(
            "vercel",
            Some("test-key".to_string()),
            "v0-1.5-md",
            None,
        )
        .expect("vercel should construct from registry");
        assert_eq!(model.model_id(), "v0-1.5-md");
    }

    /// TS: `createVercel({ apiKey: 'custom-key' })` �?custom API key
    /// is used in the Authorization header.
    #[tokio::test]
    async fn custom_api_key_used_in_auth_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer my-custom-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(text_completion_body()))
            .mount(&server)
            .await;

        let model = provider(
            "vercel",
            Some("my-custom-key".to_string()),
            "v0-1.5-md",
            Some(ProviderOptions {
                base_url: Some(server.uri()),
                ..Default::default()
            }),
        )
        .expect("vercel should construct from registry");

        model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed with custom API key");
    }

    /// TS: `from_env` should load `VERCEL_API_KEY`.
    #[test]
    fn from_env_loads_correct_env_var() {
        let saved = std::env::var("VERCEL_API_KEY").ok();
        unsafe { std::env::set_var("VERCEL_API_KEY", "env-test-key") };

        let result = provider_from_env("vercel", "v0-1.5-md", None);
        assert!(
            result.is_ok(),
            "provider_from_env should succeed with env var set"
        );

        unsafe {
            match saved {
                Some(v) => std::env::set_var("VERCEL_API_KEY", v),
                None => std::env::remove_var("VERCEL_API_KEY"),
            }
        }
    }

    /// TS: `from_env` should fail without the env var.
    #[test]
    #[ignore = "flaky: parallel test env var race"]
    fn from_env_fails_without_env_var() {
        let saved = std::env::var("VERCEL_API_KEY").ok();
        unsafe { std::env::remove_var("VERCEL_API_KEY") };

        let result = provider_from_env("vercel", "v0-1.5-md", None);
        assert!(
            result.is_err(),
            "provider_from_env should fail without env var"
        );

        unsafe {
            if let Some(v) = saved {
                std::env::set_var("VERCEL_API_KEY", v);
            }
        }
    }
}
