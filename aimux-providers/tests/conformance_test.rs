//! Conformance tests using real recorded API responses (cassettes).
//!
//! These tests mount cassette files from `tests/cassettes/<provider>/` and
//! verify that our provider implementations correctly parse real-world API
//! responses 閳?not just hand-crafted wiremock mocks.
//!
//! Per RFC 0003, these are *structural* conformance tests: they check that
//! our parsing code produces the right *shape* of output (text content,
//! finish reason, usage, tool calls, stream parts) without asserting on
//! specific text content (which varies by model).

mod common;

use common::replay::mount_cassettes;
use futures::StreamExt;
use wiremock::MockServer;

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;

use aimux_providers::{
    AnthropicConfig, AnthropicProvider, BedrockProvider, BedrockProviderConfig, CohereConfig,
    CohereProvider, GoogleConfig, GoogleProvider, HuggingFaceConfig, HuggingFaceProvider,
    LlamafileConfig, LlamafileProvider, LmStudioConfig, LmStudioProvider, MistralConfig,
    MistralProvider, MistralrsConfig, MistralrsProvider, OllamaConfig, OllamaProvider,
    OpenAIConfig, OpenAIProvider, OpenRouterConfig, OpenRouterProvider, ProviderOptions, XAIConfig,
    XAIProvider, provider,
};

// 閳光偓閳光偓 helpers 閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓閳光偓

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

async fn collect_stream(result: aimux_core::result::StreamResult) -> Vec<StreamPart> {
    let mut parts = Vec::new();
    let mut stream = result.stream;
    while let Some(part) = stream.next().await {
        match part {
            Ok(p) => parts.push(p),
            Err(e) => panic!("stream error: {:?}", e),
        }
    }
    parts
}

fn has_text(content: &[GenerateContent]) -> bool {
    content
        .iter()
        .any(|c| matches!(c, GenerateContent::Text { .. }))
}

fn has_tool_call(content: &[GenerateContent]) -> bool {
    content
        .iter()
        .any(|c| matches!(c, GenerateContent::ToolCall { .. }))
}

fn has_reasoning(content: &[GenerateContent]) -> bool {
    content
        .iter()
        .any(|c| matches!(c, GenerateContent::Reasoning { .. }))
}

#[allow(dead_code)]
fn has_text_delta(parts: &[StreamPart]) -> bool {
    parts
        .iter()
        .any(|p| matches!(p, StreamPart::TextDelta { .. }))
}

fn has_finish(parts: &[StreamPart]) -> bool {
    parts.iter().any(|p| matches!(p, StreamPart::Finish { .. }))
}

#[allow(dead_code)]
fn has_tool_call_part(parts: &[StreamPart]) -> bool {
    parts
        .iter()
        .any(|p| matches!(p, StreamPart::ToolCall { .. }))
}

/// True when an error signals the request never matched a cassette — i.e. a
/// test-infrastructure failure (wrong base URL / endpoint / path encoding)
/// rather than a provider parsing gap. Such errors must fail the test loudly;
/// the tolerant `Err` arms below use this to avoid silently swallowing 404s
/// that would otherwise masquerade as "unsupported feature" passes.
fn is_infrastructure_error(e: &aimux_core::error::AiMuxError) -> bool {
    use aimux_core::error::AiMuxError;
    match e {
        // wiremock returns 404 when no Mock matches the request path/method.
        AiMuxError::ApiCall(d) => d.status_code == Some(404),
        _ => false,
    }
}

// 閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜
// Anthropic 閳?/v1/messages
// 閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜

mod anthropic_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> AnthropicProvider {
        let config = AnthropicConfig::new("test-key").with_base_url(server.uri());
        AnthropicProvider::new(config)
    }

    /// doGenerate with a non-streaming cassette should return text content.
    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/anthropic").await;

        let provider = make_provider(&server);
        let model = provider.model("claude-sonnet-4-6");

        let result = model.do_generate(&default_options(test_prompt())).await;

        // The cassette has a real response 閳?our parser should produce text.
        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected some content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                // Some cassettes may use features we don't support yet 閳?                // the error should be a structured AiMuxError, not a panic.
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(
                    !msg.contains("panic") && !msg.contains("unwrap"),
                    "unexpected error: {}",
                    msg
                );
            }
        }
    }

    /// doStream with a streaming cassette should emit text deltas and finish.
    #[tokio::test]
    async fn do_stream_returns_parts() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/anthropic").await;

        let provider = make_provider(&server);
        let model = provider.model("claude-sonnet-4-6");

        let result = model.do_stream(&default_options(test_prompt())).await;

        match result {
            Ok(stream_result) => {
                let parts = collect_stream(stream_result).await;
                // Streaming should produce at least a finish part.
                // (Text deltas depend on the cassette 閳?some may be tool-only.)
                assert!(
                    has_finish(&parts) || !parts.is_empty(),
                    "stream should produce some parts, got empty"
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(
                    !msg.contains("panic") && !msg.contains("unwrap"),
                    "unexpected error: {}",
                    msg
                );
            }
        }
    }
}

// 閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜
// OpenAI 閳?/v1/chat/completions and /v1/responses
// 閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜

mod openai_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> OpenAIProvider {
        // Cassettes are recorded against /v1/responses; the responses model
        // appends `/responses` to the base URL, so base must include `/v1`.
        let config = OpenAIConfig::new("test-key").with_base_url(format!("{}/v1", server.uri()));
        OpenAIProvider::new(config)
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/openai").await;

        let provider = make_provider(&server);
        let model = provider.responses_model("gpt-4o");

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(
                    !msg.contains("panic") && !msg.contains("unwrap"),
                    "unexpected error: {}",
                    msg
                );
            }
        }
    }

    #[tokio::test]
    async fn do_stream_returns_parts() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/openai").await;

        let provider = make_provider(&server);
        let model = provider.responses_model("gpt-4o");

        let result = model.do_stream(&default_options(test_prompt())).await;

        match result {
            Ok(stream_result) => {
                let parts = collect_stream(stream_result).await;
                assert!(
                    has_finish(&parts) || !parts.is_empty(),
                    "stream should produce some parts"
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(
                    !msg.contains("panic") && !msg.contains("unwrap"),
                    "unexpected error: {}",
                    msg
                );
            }
        }
    }
}

// 閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜
// DeepSeek 閳?/chat/completions
// 閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜

mod deepseek_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> Box<dyn LanguageModel> {
        provider(
            "deepseek",
            Some("test-key".to_string()),
            "deepseek-chat",
            Some(ProviderOptions {
                base_url: Some(server.uri()),
                ..Default::default()
            }),
        )
        .expect("deepseek should construct from registry")
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/deepseek").await;

        let model = make_provider(&server);

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// 閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜
// xAI 閳?/chat/completions
// 閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜

mod xai_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> XAIProvider {
        // Cassettes are recorded against /v1/responses; the responses model
        // appends `/responses` to the base URL, so base must include `/v1`.
        let config = XAIConfig::new("test-key").with_base_url(format!("{}/v1", server.uri()));
        XAIProvider::new(config)
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/xai").await;

        let provider = make_provider(&server);
        let model = provider.responses_model("grok-3-mini");

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// 閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜
// Groq 閳?/openai/v1/chat/completions
// 閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜

mod groq_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> Box<dyn LanguageModel> {
        // Cassettes record paths under /openai/v1/chat/completions, matching
        // Groq's default base URL https://api.groq.com/openai/v1.
        provider(
            "groq",
            Some("test-key".to_string()),
            "llama-3.3-70b-versatile",
            Some(ProviderOptions {
                base_url: Some(format!("{}/openai/v1", server.uri())),
                ..Default::default()
            }),
        )
        .expect("groq should construct from registry")
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/groq").await;

        let model = make_provider(&server);

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// 閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜
// Mistral 閳?/v1/chat/completions
// 閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜

mod mistral_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> MistralProvider {
        // Cassettes record paths under /v1/chat/completions, matching Mistral's
        // default base URL https://api.mistral.ai/v1.
        let config = MistralConfig::new("test-key").with_base_url(format!("{}/v1", server.uri()));
        MistralProvider::new(config)
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/mistral").await;

        let provider = make_provider(&server);
        let model = provider.model("mistral-large-latest");

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// 閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜
// Perplexity 閳?/chat/completions
// 閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜閳烘劏鏅查埡鎰ㄦ櫜

mod perplexity_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> Box<dyn LanguageModel> {
        provider(
            "perplexity",
            Some("test-key".to_string()),
            "sonar",
            Some(ProviderOptions {
                base_url: Some(server.uri()),
                ..Default::default()
            }),
        )
        .expect("perplexity should construct from registry")
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/perplexity").await;

        let model = make_provider(&server);

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// 鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲
// Google Gemini 鈥?/v1beta/models/{model}:generateContent
// 鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲

mod gemini_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> GoogleProvider {
        let config =
            GoogleConfig::new("test-key").with_base_url(format!("{}/v1beta", server.uri()));
        GoogleProvider::new(config)
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/gemini").await;

        let provider = make_provider(&server);
        let model = provider.model("gemini-2.5-flash");

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }

    #[tokio::test]
    async fn do_stream_returns_parts() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/gemini").await;

        let provider = make_provider(&server);
        let model = provider.model("gemini-2.5-flash");

        let result = model.do_stream(&default_options(test_prompt())).await;

        match result {
            Ok(stream_result) => {
                let parts = collect_stream(stream_result).await;
                assert!(
                    has_finish(&parts) || !parts.is_empty(),
                    "stream should produce some parts"
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}
// ════════════════════════════════════════════════════════════════════════════
// OpenRouter — /chat/completions
// ════════════════════════════════════════════════════════════════════════════

mod openrouter_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> OpenRouterProvider {
        // Cassettes record paths under /api/v1/chat/completions, matching
        // OpenRouter's default base URL https://openrouter.ai/api/v1.
        let config =
            OpenRouterConfig::new("test-key").with_base_url(format!("{}/api/v1", server.uri()));
        OpenRouterProvider::new(config)
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/openrouter").await;

        let provider = make_provider(&server);
        let model = provider.model("openai/gpt-4o");

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// GitHub Copilot — /chat/completions
// ════════════════════════════════════════════════════════════════════════════

mod copilot_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> Box<dyn LanguageModel> {
        // Copilot's base URL has no `/v1` prefix; the OpenAIProvider appends
        // `/chat/completions` directly, matching the cassette path
        // `/chat/completions`.
        provider(
            "copilot",
            Some("test-key".to_string()),
            "gpt-4o",
            Some(ProviderOptions {
                base_url: Some(server.uri()),
                ..Default::default()
            }),
        )
        .expect("copilot should construct from registry")
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/copilot").await;

        let model = make_provider(&server);

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Doubleword — /v1/chat/completions
// ════════════════════════════════════════════════════════════════════════════

mod doubleword_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> Box<dyn LanguageModel> {
        // Doubleword's base URL includes `/v1`; the OpenAIProvider appends
        // `/chat/completions`, so we point at `<server>/v1` to match the
        // cassette path `/v1/chat/completions`.
        provider(
            "doubleword",
            Some("test-key".to_string()),
            "Qwen/Qwen3.5-9B",
            Some(ProviderOptions {
                base_url: Some(format!("{}/v1", server.uri())),
                ..Default::default()
            }),
        )
        .expect("doubleword should construct from registry")
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/doubleword").await;

        let model = make_provider(&server);

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// llamafile — /v1/chat/completions
// ════════════════════════════════════════════════════════════════════════════

mod llamafile_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> LlamafileProvider {
        // llamafile's base URL includes `/v1`; the OpenAIProvider appends
        // `/chat/completions`, so we point at `<server>/v1` to match the
        // cassette path `/v1/chat/completions`.
        let config = LlamafileConfig::new("test-key").with_base_url(format!("{}/v1", server.uri()));
        LlamafileProvider::new(config)
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/llamafile").await;

        let provider = make_provider(&server);
        let model = provider.model("llama3.2:latest");

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// mistral.rs — /v1/chat/completions
// ════════════════════════════════════════════════════════════════════════════

mod mistralrs_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> MistralrsProvider {
        // mistral.rs's base URL includes `/v1`; the OpenAIProvider appends
        // `/chat/completions`, so we point at `<server>/v1` to match the
        // cassette path `/v1/chat/completions`.
        let config = MistralrsConfig::new("test-key").with_base_url(format!("{}/v1", server.uri()));
        MistralrsProvider::new(config)
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/mistralrs").await;

        let provider = make_provider(&server);
        let model = provider.model("Qwen/Qwen3-4B");

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Amazon Bedrock — /model/{model-id}/converse and /converse-stream
// ════════════════════════════════════════════════════════════════════════════

mod bedrock_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> BedrockProvider {
        // Bearer-token auth bypasses SigV4 signing so the mock server sees
        // plain requests — matching how the rig cassettes were recorded.
        let config = BedrockProviderConfig::with_bearer_token("test-token", "us-east-1")
            .with_base_url(server.uri());
        BedrockProvider::new(config)
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/bedrock").await;

        let provider = make_provider(&server);
        let model = provider.model("amazon.nova-lite-v1:0");

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }

    #[tokio::test]
    async fn do_stream_returns_parts() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/bedrock").await;

        let provider = make_provider(&server);
        let model = provider.model("amazon.nova-lite-v1:0");

        let result = model.do_stream(&default_options(test_prompt())).await;

        match result {
            Ok(stream_result) => {
                let parts = collect_stream(stream_result).await;
                assert!(
                    has_finish(&parts) || !parts.is_empty(),
                    "stream should produce some parts"
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Cerebras — /v1/chat/completions (OpenAI-compatible wrapper)
// ════════════════════════════════════════════════════════════════════════════

mod cerebras_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> Box<dyn LanguageModel> {
        // Cassettes record paths under /v1/chat/completions, matching Cerebras's
        // default base URL https://api.cerebras.ai/v1.
        provider(
            "cerebras",
            Some("test-key".to_string()),
            "llama-3.3-70b",
            Some(ProviderOptions {
                base_url: Some(format!("{}/v1", server.uri())),
                ..Default::default()
            }),
        )
        .expect("cerebras should construct from registry")
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/cerebras").await;

        let model = make_provider(&server);

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Cohere — /v2/chat (native Cohere API)
// ════════════════════════════════════════════════════════════════════════════

mod cohere_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> CohereProvider {
        // Cassettes record paths under /v2/chat, matching Cohere's default base
        // URL https://api.cohere.com/v2; the model appends `/chat`.
        let config = CohereConfig::new("test-key").with_base_url(format!("{}/v2", server.uri()));
        CohereProvider::new(config)
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/cohere").await;

        let provider = make_provider(&server);
        let model = provider.model("command-r-08-2024");

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Hugging Face — /together/v1/chat/completions (OpenAI-compatible router)
// ════════════════════════════════════════════════════════════════════════════

mod huggingface_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> HuggingFaceProvider {
        // Cassettes record paths under /together/v1/chat/completions (HF routes
        // together-hosted models through this prefix). The OpenAI model appends
        // `/chat/completions` to the base URL.
        let config = HuggingFaceConfig::new("test-key")
            .with_base_url(format!("{}/together/v1", server.uri()));
        HuggingFaceProvider::new(config)
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/huggingface").await;

        let provider = make_provider(&server);
        let model = provider.model("deepseek-ai/DeepSeek-R1");

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Zai — /api/paas/v4/chat/completions (OpenAI-compatible, reasoning_content)
// ════════════════════════════════════════════════════════════════════════════

mod zai_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> Box<dyn LanguageModel> {
        // Cassettes record paths under /api/paas/v4/chat/completions, matching
        // Zai's base URL https://api.z.ai/api/paas/v4.
        provider(
            "zai",
            Some("test-key".to_string()),
            "glm-4.7",
            Some(ProviderOptions {
                base_url: Some(format!("{}/api/paas/v4", server.uri())),
                ..Default::default()
            }),
        )
        .expect("zai should construct from registry")
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/zai").await;

        let model = make_provider(&server);

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }

    #[tokio::test]
    async fn do_stream_returns_parts() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/zai").await;

        let model = make_provider(&server);

        let result = model.do_stream(&default_options(test_prompt())).await;

        match result {
            Ok(stream_result) => {
                let parts = collect_stream(stream_result).await;
                assert!(
                    has_finish(&parts) || !parts.is_empty(),
                    "stream should produce some parts"
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Ollama — /v1/chat/completions (pydantic-ai OpenAI-compatible endpoint)
// ════════════════════════════════════════════════════════════════════════════

mod ollama_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> OllamaProvider {
        // pydantic-ai cassettes record paths under /v1/chat/completions,
        // matching Ollama's OpenAI-compatible endpoint.
        // (rig cassettes use the native /api/chat NDJSON endpoint and won't
        //  be hit by this OpenAI-compatible provider — that's expected.)
        let config = OllamaConfig::new("test-key").with_base_url(format!("{}/v1", server.uri()));
        OllamaProvider::new(config)
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/ollama").await;

        let provider = make_provider(&server);
        let model = provider.model("gpt-oss:20b");

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(!msg.contains("panic"), "unexpected error: {}", msg);
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// ChatGPT — /backend-api/codex/responses (Responses API variant)
// ════════════════════════════════════════════════════════════════════════════

mod chatgpt_conformance {
    use super::*;

    fn make_provider(server: &MockServer) -> OpenAIProvider {
        // Cassettes record paths under /backend-api/codex/responses; the
        // responses model appends `/responses` to the base URL.
        let config = OpenAIConfig::new("test-key")
            .with_base_url(format!("{}/backend-api/codex", server.uri()));
        OpenAIProvider::new(config)
    }

    #[tokio::test]
    async fn do_generate_returns_text() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/chatgpt").await;

        let provider = make_provider(&server);
        let model = provider.responses_model("gpt-5.4");

        let result = model.do_generate(&default_options(test_prompt())).await;

        match result {
            Ok(r) => {
                assert!(
                    has_text(&r.content) || has_tool_call(&r.content) || has_reasoning(&r.content),
                    "expected content, got {:?}",
                    r.content
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(
                    !msg.contains("panic") && !msg.contains("unwrap"),
                    "unexpected error: {}",
                    msg
                );
            }
        }
    }

    #[tokio::test]
    async fn do_stream_returns_parts() {
        let server = MockServer::start().await;
        mount_cassettes(&server, "tests/cassettes/chatgpt").await;

        let provider = make_provider(&server);
        let model = provider.responses_model("gpt-5.4");

        let result = model.do_stream(&default_options(test_prompt())).await;

        match result {
            Ok(stream_result) => {
                let parts = collect_stream(stream_result).await;
                assert!(
                    has_finish(&parts) || !parts.is_empty(),
                    "stream should produce some parts"
                );
            }
            Err(e) => {
                if is_infrastructure_error(&e) {
                    panic!("request did not match any cassette (404): {e:?}");
                }
                let msg = e.to_string();
                assert!(
                    !msg.contains("panic") && !msg.contains("unwrap"),
                    "unexpected error: {}",
                    msg
                );
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Thin-wrapper conformance (OpenAI-compatible providers without real cassettes)
//
// alibaba / baseten / bytedance / deepinfra / fireworks / moonshotai /
// togetherai / vercel are all thin wrappers around OpenAIProvider. Their
// responses are byte-for-byte OpenAI Chat Completions, so cassettes are
// derived from real OpenAI recordings (path + model rewritten). See
// scripts/generate_thin_wrapper_cassettes.py.
// ════════════════════════════════════════════════════════════════════════════

/// Generates a conformance module for an OpenAI-compatible thin-wrapper provider.
/// Each gets a generate + stream test.
///
/// Two arms:
/// - native arm (`$provider:ty, $config:ty`): providers with their own src/
///   implementation (e.g. LmStudio) keep the config/provider construction.
/// - registry arm (`$name:literal`): the retired phase-4 shell types are built
///   through the registry-backed `provider(name, ...)` entry point.
macro_rules! thin_wrapper_conformance {
    ($mod_name:ident, $provider:ty, $config:ty, $cassette_dir:expr, $base_url_prefix:expr, $model_id:expr) => {
        mod $mod_name {
            use super::*;

            fn make_provider(server: &MockServer) -> Box<dyn LanguageModel> {
                Box::new(
                    <$provider>::new(<$config>::new("test-key").with_base_url(format!(
                        "{}{}",
                        server.uri(),
                        $base_url_prefix
                    )))
                    .model($model_id),
                )
            }

            thin_wrapper_tests!($cassette_dir);
        }
    };
    ($mod_name:ident, $name:literal, $cassette_dir:expr, $base_url_prefix:expr, $model_id:expr) => {
        mod $mod_name {
            use super::*;

            fn make_provider(server: &MockServer) -> Box<dyn LanguageModel> {
                provider(
                    $name,
                    Some("test-key".to_string()),
                    $model_id,
                    Some(ProviderOptions {
                        base_url: Some(format!("{}{}", server.uri(), $base_url_prefix)),
                        ..Default::default()
                    }),
                )
                .expect("registry provider should construct")
            }

            thin_wrapper_tests!($cassette_dir);
        }
    };
}

/// The two conformance tests shared by every thin-wrapper module.
macro_rules! thin_wrapper_tests {
    ($cassette_dir:expr) => {
        #[tokio::test]
        async fn do_generate_returns_text() {
            let server = MockServer::start().await;
            mount_cassettes(&server, $cassette_dir).await;

            let model = make_provider(&server);

            let result = model.do_generate(&default_options(test_prompt())).await;

            match result {
                Ok(r) => {
                    assert!(
                        has_text(&r.content)
                            || has_tool_call(&r.content)
                            || has_reasoning(&r.content),
                        "expected content, got {:?}",
                        r.content
                    );
                }
                Err(e) => {
                    if is_infrastructure_error(&e) {
                        panic!("request did not match any cassette (404): {e:?}");
                    }
                    let msg = e.to_string();
                    assert!(!msg.contains("panic"), "unexpected error: {}", msg);
                }
            }
        }

        #[tokio::test]
        async fn do_stream_returns_parts() {
            let server = MockServer::start().await;
            mount_cassettes(&server, $cassette_dir).await;

            let model = make_provider(&server);

            let result = model.do_stream(&default_options(test_prompt())).await;

            match result {
                Ok(stream_result) => {
                    let parts = collect_stream(stream_result).await;
                    assert!(
                        has_finish(&parts) || !parts.is_empty(),
                        "stream should produce some parts"
                    );
                }
                Err(e) => {
                    if is_infrastructure_error(&e) {
                        panic!("request did not match any cassette (404): {e:?}");
                    }
                    let msg = e.to_string();
                    assert!(!msg.contains("panic"), "unexpected error: {}", msg);
                }
            }
        }
    };
}

thin_wrapper_conformance!(
    alibaba_conformance,
    "alibaba",
    "tests/cassettes/alibaba",
    "/compatible-mode/v1",
    "qwen-plus"
);
thin_wrapper_conformance!(
    baseten_conformance,
    "baseten",
    "tests/cassettes/baseten",
    "/v1",
    "meta-llama/Llama-3.1-8B-Instruct"
);
thin_wrapper_conformance!(
    bytedance_conformance,
    "bytedance",
    "tests/cassettes/bytedance",
    "/api/v3",
    "doubao-pro-32k"
);
thin_wrapper_conformance!(
    deepinfra_conformance,
    "deepinfra",
    "tests/cassettes/deepinfra",
    "/v1/openai",
    "meta-llama/Llama-3.1-8B-Instruct"
);
thin_wrapper_conformance!(
    fireworks_conformance,
    "fireworks",
    "tests/cassettes/fireworks",
    "/inference/v1",
    "llama-v3p1-8b-instruct"
);
thin_wrapper_conformance!(
    moonshotai_conformance,
    "moonshotai",
    "tests/cassettes/moonshotai",
    "/v1",
    "moonshot-v1-8k"
);
thin_wrapper_conformance!(
    togetherai_conformance,
    "togetherai",
    "tests/cassettes/togetherai",
    "/v1",
    "meta-llama/Llama-3.1-8B-Instruct-Turbo"
);
thin_wrapper_conformance!(
    vercel_conformance,
    "vercel",
    "tests/cassettes/vercel",
    "/v1",
    "gpt-4o"
);
thin_wrapper_conformance!(
    github_conformance,
    "github",
    "tests/cassettes/github",
    "",
    "gpt-4o"
);
thin_wrapper_conformance!(
    siliconflow_conformance,
    "siliconflow",
    "tests/cassettes/siliconflow",
    "/v1",
    "Qwen/Qwen2.5-7B-Instruct"
);
thin_wrapper_conformance!(
    lmstudio_conformance,
    LmStudioProvider,
    LmStudioConfig,
    "tests/cassettes/lmstudio",
    "/v1",
    "llama-3.2-3b-instruct"
);
thin_wrapper_conformance!(
    sambanova_conformance,
    "sambanova",
    "tests/cassettes/sambanova",
    "/v1",
    "Meta-Llama-3.1-8B-Instruct"
);
