//! Remaining Anthropic provider tests — ported from the TS SDK suite.
//!
//! Sources:
//!
//! - `anthropic-provider.test.ts` — provider configuration (baseURL, auth,
//!   custom provider name, `supportedUrls`).
//! - `anthropic-unknown-model-max-output-tokens.test.ts` — unknown-model
//!   `max_tokens` defaulting + compatibility warning.
//! - `sanitize-json-schema.test.ts` — JSON Schema sanitization.
//! - `anthropic-language-model.test.ts` → `mid-conversation tool changes`
//!   describe block.
//!
#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::shared::provider_namespace;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, ProviderTool, Tool};
use aimux_core::types::Warning;
use aimux_providers::anthropic::sanitize_json_schema::sanitize_json_schema;
use aimux_providers::anthropic::{
    AnthropicMessagesModel, AnthropicProviderSettings, create_anthropic,
};
use futures::StreamExt;
use mock_fetch::{Canned, EnvVar, MockFetch};
use serde_json::{Value, json};
use serial_test::serial;
use std::sync::Arc;

/// The TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

fn make_model_with_config(settings: AnthropicProviderSettings) -> AnthropicMessagesModel {
    create_anthropic(settings)
        .unwrap()
        .messages("claude-3-haiku-20240307")
}

fn text_response(text: &str) -> Value {
    json!({
        "id": "msg_123", "type": "message", "role": "assistant",
        "content": [{ "type": "text", "text": text }],
        "model": "claude-3-haiku-20240307", "stop_reason": "end_turn",
        "stop_sequence": null, "usage": { "input_tokens": 1, "output_tokens": 1 }
    })
}

fn mock() -> Arc<MockFetch> {
    MockFetch::new(vec![Canned::json(&text_response("Hi"))])
}

fn settings(fetch: &Arc<MockFetch>) -> AnthropicProviderSettings {
    AnthropicProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    }
}

async fn request_url(settings: AnthropicProviderSettings, fetch: &Arc<MockFetch>) -> String {
    make_model_with_config(settings)
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();
    let requests = fetch.seen();
    assert_eq!(requests.len(), 1);
    requests[0].url.clone()
}

mod create_anthropic {
    use super::*;

    mod base_url_configuration {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › createAnthropic › baseURL configuration › uses the default Anthropic base URL when not provided
        #[tokio::test]
        #[serial]
        async fn default_base_url_when_not_provided() {
            let _env = EnvVar::set("ANTHROPIC_BASE_URL", None);
            let fetch = mock();
            assert_eq!(
                request_url(settings(&fetch), &fetch).await,
                "https://api.anthropic.com/v1/messages"
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › createAnthropic › baseURL configuration › uses ANTHROPIC_BASE_URL when set
        #[tokio::test]
        #[serial]
        async fn uses_anthropic_base_url_env_when_set() {
            let _env = EnvVar::set(
                "ANTHROPIC_BASE_URL",
                Some("https://proxy.anthropic.example/v1/"),
            );
            let fetch = mock();
            assert_eq!(
                request_url(settings(&fetch), &fetch).await,
                "https://proxy.anthropic.example/v1/messages"
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › createAnthropic › baseURL configuration › normalizes a bare Anthropic API URL from ANTHROPIC_BASE_URL
        #[tokio::test]
        #[serial]
        async fn normalizes_bare_anthropic_url_from_env() {
            let _env = EnvVar::set("ANTHROPIC_BASE_URL", Some("https://api.anthropic.com/"));
            let fetch = mock();
            assert_eq!(
                request_url(settings(&fetch), &fetch).await,
                "https://api.anthropic.com/v1/messages"
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › createAnthropic › baseURL configuration › normalizes a bare Anthropic API URL from the baseURL option
        #[tokio::test]
        async fn normalizes_bare_anthropic_url_from_base_url_option() {
            let fetch = mock();
            let settings = AnthropicProviderSettings {
                base_url: Some("https://api.anthropic.com/".into()),
                ..settings(&fetch)
            };
            assert_eq!(
                request_url(settings, &fetch).await,
                "https://api.anthropic.com/v1/messages"
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › createAnthropic › baseURL configuration › prefers the baseURL option over ANTHROPIC_BASE_URL
        #[tokio::test]
        #[serial]
        async fn prefers_base_url_option_over_env() {
            let _env = EnvVar::set(
                "ANTHROPIC_BASE_URL",
                Some("https://env.anthropic.example/v1"),
            );
            let fetch = mock();
            let settings = AnthropicProviderSettings {
                base_url: Some("https://option.anthropic.example/v1/".into()),
                ..settings(&fetch)
            };
            assert_eq!(
                request_url(settings, &fetch).await,
                "https://option.anthropic.example/v1/messages"
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › createAnthropic › baseURL configuration › rejects an empty baseURL option during provider creation
        #[test]
        fn rejects_empty_base_url_option() {
            let result = create_anthropic(AnthropicProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(String::new()),
                ..Default::default()
            });
            assert!(
                matches!(result, Err(AiMuxError::InvalidArgument { message: msg, .. }) if msg == "baseURL must be a non-empty string.")
            );
        }
    }
}

mod anthropic_provider_authentication {
    use super::*;

    mod auth_token_option {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › anthropic provider - authentication › authToken option › sends Authorization Bearer header when authToken is provided
        #[tokio::test]
        async fn sends_authorization_bearer_when_auth_token_provided() {
            let fetch = mock();
            let settings = AnthropicProviderSettings {
                auth_token: Some("test-auth-token".into()),
                api_key: None,
                ..settings(&fetch)
            };
            request_url(settings, &fetch).await;
            let requests = fetch.seen();
            assert_eq!(
                requests[0].headers.get("authorization").map(String::as_str),
                Some("Bearer test-auth-token")
            );
            assert!(!requests[0].headers.contains_key("x-api-key"));
        }
    }

    mod api_key_and_auth_token_conflict {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › anthropic provider - authentication › apiKey and authToken conflict › throws error when both apiKey and authToken options are provided
        #[test]
        fn throws_when_both_api_key_and_auth_token_provided() {
            let result = create_anthropic(AnthropicProviderSettings {
                api_key: Some("test-api-key".into()),
                auth_token: Some("test-auth-token".into()),
                ..Default::default()
            });
            assert!(
                matches!(result, Err(AiMuxError::InvalidArgument { message: msg, .. }) if msg == "Both apiKey and authToken were provided. Please use only one authentication method.")
            );
        }
    }
}

mod anthropic_provider_custom_provider_name {
    use super::*;

    /// partial: `should use custom provider name when specified`; full port in src/anthropic/tests/provider.rs.
    #[test]
    fn uses_custom_provider_name_when_specified() {
        let model = make_model_with_config(AnthropicProviderSettings {
            name: Some("my-proxy".into()),
            api_key: Some("test-api-key".into()),
            ..Default::default()
        });
        assert_eq!(model.provider(), "my-proxy");
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › anthropic provider - custom provider name › should default to anthropic.messages when name not specified
    #[test]
    fn defaults_to_anthropic_messages_when_not_specified() {
        let model = make_model_with_config(AnthropicProviderSettings {
            api_key: Some("test-api-key".into()),
            ..Default::default()
        });
        assert_eq!(model.provider(), "anthropic.messages");
    }
}

mod anthropic_provider_supported_urls {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › anthropic provider - supportedUrls › should support image/* URLs
    #[tokio::test]
    async fn supports_image_urls() {
        let model = make_model_with_config(AnthropicProviderSettings::default());
        assert!(
            model.supported_urls().await.0["image/*"][0].is_match("https://example.com/image.png")
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › anthropic provider - supportedUrls › should support application/pdf URLs
    #[tokio::test]
    async fn supports_pdf_urls() {
        let model = make_model_with_config(AnthropicProviderSettings::default());
        assert!(
            model.supported_urls().await.0["application/pdf"][0]
                .is_match("https://arxiv.org/pdf/2401.00001")
        );
    }
}

mod unknown_model_max_output_tokens {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/anthropic-unknown-model-max-output-tokens.test.ts › unknown model max output tokens › should warn when using the default max output token limit
    #[tokio::test]
    async fn warns_when_using_default_max_output_token_limit() {
        let server = mock();

        let model = create_anthropic(settings(&server))
            .unwrap()
            .messages("future-model");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let requests = server.seen();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "future-model");
        assert_eq!(body["max_tokens"], 4096);

        assert_eq!(result.warnings.len(), 1);
        match &result.warnings[0] {
            Warning::Compatibility { feature, details } => {
                assert_eq!(feature, "maxOutputTokens");
                assert_eq!(
                    details.as_deref(),
                    Some(
                        "The model \"future-model\" is unknown. The max output tokens have been \
                         limited to 4096. Set maxOutputTokens explicitly to override this limit."
                    )
                );
            }
            other => panic!("expected Compatibility warning, got {other:?}"),
        }
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-unknown-model-max-output-tokens.test.ts › unknown model max output tokens › should not warn when max output tokens are provided
    #[tokio::test]
    async fn does_not_warn_when_max_output_tokens_provided() {
        let server = mock();

        let model = create_anthropic(settings(&server))
            .unwrap()
            .messages("future-model");
        let mut opts = default_options(test_prompt());
        opts.max_output_tokens = Some(123456);
        let result = model
            .do_generate(&opts)
            .await
            .expect("do_generate should succeed");

        let requests = server.seen();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "future-model");
        assert_eq!(body["max_tokens"], 123456);
        assert!(
            result.warnings.is_empty(),
            "warnings: {:?}",
            result.warnings
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-unknown-model-max-output-tokens.test.ts › unknown model max output tokens › should use the current-generation default and warn for an unknown Claude model
    #[tokio::test]
    async fn uses_current_gen_default_and_warns_for_unknown_model() {
        let server = mock();

        let model = create_anthropic(settings(&server))
            .unwrap()
            .messages("claude-future-9");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("do_generate should succeed");

        let requests = server.seen();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "claude-future-9");
        assert_eq!(body["max_tokens"], 128000);

        assert_eq!(result.warnings.len(), 1);
        match &result.warnings[0] {
            Warning::Compatibility { feature, details } => {
                assert_eq!(feature, "maxOutputTokens");
                assert_eq!(
                    details.as_deref(),
                    Some(
                        "The model \"claude-future-9\" is unknown. The max output tokens have been \
                         limited to 128000. Set maxOutputTokens explicitly to override this limit."
                    )
                );
            }
            other => panic!("expected Compatibility warning, got {other:?}"),
        }
    }
}

mod sanitize_json_schema {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/sanitize-json-schema.test.ts › sanitizeJsonSchema › strips unsupported number constraints and adds readable descriptions
    #[test]
    fn sanitize_strips_unsupported_number_constraints() {
        let schema = json!({
            "type": "object",
            "properties": {
                "recurringIntervalMinutes": {
                    "type": "number",
                    "exclusiveMinimum": 0,
                    "minimum": 1,
                    "maximum": 60,
                    "exclusiveMaximum": 120
                }
            },
            "required": ["recurringIntervalMinutes"],
            "additionalProperties": false
        });

        let expected = json!({
            "additionalProperties": false,
            "properties": {
                "recurringIntervalMinutes": {
                    "description":
                        "minimum: 1; maximum: 60; exclusive minimum: 0; exclusive maximum: 120.",
                    "type": "number"
                }
            },
            "required": ["recurringIntervalMinutes"],
            "type": "object"
        });

        assert_eq!(sanitize_json_schema(&schema), expected);
    }

    /// upstream: @ai-sdk/anthropic src/sanitize-json-schema.test.ts › sanitizeJsonSchema › strips unsupported string constraints and unsupported formats
    #[test]
    fn sanitize_strips_unsupported_string_constraints_and_formats() {
        let schema = json!({
            "type": "object",
            "properties": {
                "slug": {
                    "type": "string",
                    "description": "A URL slug",
                    "minLength": 1,
                    "maxLength": 20,
                    "pattern": "^[a-z0-9-]+$",
                    "format": "regex"
                }
            }
        });

        let expected = json!({
            "additionalProperties": false,
            "properties": {
                "slug": {
                    "description":
                        "A URL slug\nmin length: 1; max length: 20; pattern: ^[a-z0-9-]+$; format: regex.",
                    "type": "string"
                }
            },
            "type": "object"
        });

        assert_eq!(sanitize_json_schema(&schema), expected);
    }

    /// upstream: @ai-sdk/anthropic src/sanitize-json-schema.test.ts › sanitizeJsonSchema › recursively sanitizes arrays, definitions, and composition schemas
    #[test]
    fn sanitize_recursively_handles_arrays_defs_and_composition() {
        let schema = json!({
            "type": "object",
            "$defs": {
                "PositiveInteger": { "type": "integer", "minimum": 1 }
            },
            "properties": {
                "count": { "$ref": "#/$defs/PositiveInteger" },
                "tags": {
                    "type": "array",
                    "minItems": 2,
                    "maxItems": 4,
                    "uniqueItems": true,
                    "items": {
                        "anyOf": [
                            { "type": "string", "minLength": 1 },
                            { "type": "number", "maximum": 10 }
                        ]
                    }
                }
            }
        });

        let expected = json!({
            "$defs": {
                "PositiveInteger": {
                    "description": "minimum: 1.",
                    "type": "integer"
                }
            },
            "additionalProperties": false,
            "properties": {
                "count": { "$ref": "#/$defs/PositiveInteger" },
                "tags": {
                    "description": "min items: 2; max items: 4; unique items: true.",
                    "items": {
                        "anyOf": [
                            { "description": "min length: 1.", "type": "string" },
                            { "description": "maximum: 10.", "type": "number" }
                        ]
                    },
                    "type": "array"
                }
            },
            "type": "object"
        });

        assert_eq!(sanitize_json_schema(&schema), expected);
    }

    /// upstream: @ai-sdk/anthropic src/sanitize-json-schema.test.ts › sanitizeJsonSchema › converts oneOf to anyOf
    #[test]
    fn sanitize_converts_one_of_to_any_of() {
        let schema = json!({
            "oneOf": [
                { "type": "string", "minLength": 1 },
                { "type": "number", "minimum": 0 }
            ]
        });

        let expected = json!({
            "anyOf": [
                { "description": "min length: 1.", "type": "string" },
                { "description": "minimum: 0.", "type": "number" }
            ]
        });

        assert_eq!(sanitize_json_schema(&schema), expected);
    }

    /// upstream: @ai-sdk/anthropic src/sanitize-json-schema.test.ts › sanitizeJsonSchema › does not mutate the input schema
    #[test]
    fn sanitize_does_not_mutate_input_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "value": { "type": "number", "exclusiveMinimum": 0 }
            }
        });

        let snapshot = schema.clone();
        let _ = sanitize_json_schema(&schema);
        assert_eq!(schema, snapshot);
    }
}

mod mid_conversation_tool_changes {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/anthropic-language-model.test.ts › mid-conversation tool changes › should send tool change blocks and the beta header
    #[tokio::test]
    async fn sends_tool_change_blocks_and_beta_header() {
        let fetch = mock();
        let model = create_anthropic(settings(&fetch))
            .unwrap()
            .messages("claude-opus-4-8");
        let mut opts = default_options(vec![
            LanguageModelMessage::user_text("Say OK."),
            LanguageModelMessage::System {
                content: String::new(),
                provider_options: Some(
                    provider_namespace(
                        "anthropic",
                        json!({
                            "toolChanges": [{"type": "tool_removal", "toolName": "get_weather"}]
                        }),
                    )
                    .unwrap(),
                ),
            },
        ]);
        opts.tools = Some(vec![Tool::Function(
            FunctionTool::new(
                "get_weather",
                json!({
                    "type": "object", "properties": {"city": {"type": "string"}}
                }),
            )
            .with_description("Get weather"),
        )]);
        model.do_generate(&opts).await.unwrap();
        let requests = fetch.seen();
        let body = requests[0].json_body();
        assert!(body["messages"].as_array().unwrap().contains(&json!({
            "role": "system", "content": [{
                "type": "tool_removal", "tool": {"type": "tool_reference", "name": "get_weather"}
            }]
        })));
        assert!(
            requests[0].headers["anthropic-beta"]
                .contains("mid-conversation-tool-changes-2026-07-01")
        );
    }
}

/// Upstream drops the caller of a historical tool call whose source code execution call was pruned, with a warning.
#[test]
fn omits_orphaned_historical_programmatic_caller_with_warning() {
    let prompt: LanguageModelPrompt = serde_json::from_value(json!([
        {
            "role": "assistant",
            "content": [{
                "type": "tool-call",
                "toolCallId": "lookup-call",
                "toolName": "lookup",
                "input": { "ticker": "AAPL" },
                "providerOptions": { "anthropic": {
                    "caller": { "type": "code_execution_20260120", "toolId": "pruned-source" }
                } },
            }],
        },
        {
            "role": "tool",
            "content": [{
                "type": "tool-result",
                "toolCallId": "lookup-call",
                "toolName": "lookup",
                "output": { "type": "text", "value": "185.42" },
            }],
        },
        { "role": "user", "content": [{ "type": "text", "text": "Now look up MSFT." }] },
    ]))
    .unwrap();
    let result = aimux_providers::anthropic::convert::convert_prompt_to_anthropic_full_fallible(
        &prompt, false,
    )
    .unwrap();
    assert_eq!(
        result.messages[0]["content"],
        json!([{ "type": "tool_use", "id": "lookup-call", "name": "lookup", "input": { "ticker": "AAPL" } }])
    );
    assert_eq!(
        result.warnings,
        vec![Warning::Other {
            message: "Omitted caller metadata for tool lookup-call because source code execution tool pruned-source is missing from the conversation history.".into()
        }]
    );
}

/// Upstream names the unsupported output type when an advisor result cannot be sent back.
#[test]
fn advisor_result_with_text_output_warns_with_output_type() {
    let prompt: LanguageModelPrompt = serde_json::from_value(json!([{
        "role": "assistant",
        "content": [
            {
                "type": "tool-call",
                "toolCallId": "srvtoolu_advisor",
                "toolName": "advisor",
                "input": {},
                "providerExecuted": true,
            },
            {
                "type": "tool-result",
                "toolCallId": "srvtoolu_advisor",
                "toolName": "advisor",
                "output": { "type": "text", "value": "should be json" },
            },
        ],
    }]))
    .unwrap();
    let result = aimux_providers::anthropic::convert::convert_prompt_to_anthropic_full_fallible(
        &prompt, false,
    )
    .unwrap();
    assert_eq!(result.messages[0]["content"].as_array().unwrap().len(), 1);
    assert_eq!(
        result.warnings,
        vec![Warning::Other {
            message:
                "provider executed tool result output type text for tool advisor is not supported"
                    .into()
        }]
    );
}

/// Upstream sends an MCP tool result back as `mcp_tool_result` and still warns that the provider-executed result is not supported.
#[test]
fn mcp_tool_result_round_trip_warns_like_upstream() {
    let prompt: LanguageModelPrompt = serde_json::from_value(json!([{
        "role": "assistant",
        "content": [
            {
                "type": "tool-call",
                "toolCallId": "mcptoolu_1",
                "toolName": "echo",
                "input": {},
                "providerExecuted": true,
                "providerOptions": { "anthropic": { "type": "mcp-tool-use", "serverName": "echo" } },
            },
            {
                "type": "tool-result",
                "toolCallId": "mcptoolu_1",
                "toolName": "echo",
                "output": { "type": "json", "value": [{ "type": "text", "text": "hello" }] },
            },
        ],
    }]))
    .unwrap();
    let result = aimux_providers::anthropic::convert::convert_prompt_to_anthropic_full_fallible(
        &prompt, false,
    )
    .unwrap();
    assert_eq!(
        result.messages[0]["content"][1],
        json!({
            "type": "mcp_tool_result",
            "tool_use_id": "mcptoolu_1",
            "is_error": false,
            "content": [{ "type": "text", "text": "hello" }],
        })
    );
    assert_eq!(
        result.warnings,
        vec![Warning::Other {
            message: "provider executed tool result for tool echo is not supported".into()
        }]
    );
}

/// An MCP tool named like a provider tool (`web_search` here) gets no
/// unsupported-result warning: upstream's MCP branch has no `break` and falls
/// into `if (providerToolName === 'web_search')`, which pushes a
/// `web_search_tool_result` after the `mcp_tool_result` and breaks before the
/// warning (convert-to-anthropic-prompt.ts:1026-1046, 1284-1336, 1435-1438).
#[test]
fn mcp_tool_named_like_a_provider_tool_does_not_warn() {
    let prompt: LanguageModelPrompt = serde_json::from_value(json!([{
        "role": "assistant",
        "content": [
            {
                "type": "tool-call",
                "toolCallId": "mcptoolu_2",
                "toolName": "web_search",
                "input": {},
                "providerExecuted": true,
                "providerOptions": { "anthropic": { "type": "mcp-tool-use", "serverName": "search" } },
            },
            {
                "type": "tool-result",
                "toolCallId": "mcptoolu_2",
                "toolName": "web_search",
                "output": { "type": "json", "value": [{
                    "url": "https://example.com", "title": "Example", "pageAge": null,
                    "encryptedContent": "abc", "type": "web_search_result",
                }] },
            },
        ],
    }]))
    .unwrap();
    let result = aimux_providers::anthropic::convert::convert_prompt_to_anthropic_full_fallible(
        &prompt, false,
    )
    .unwrap();
    assert_eq!(
        result.messages[0]["content"],
        json!([
            {
                "type": "mcp_tool_use",
                "id": "mcptoolu_2",
                "name": "web_search",
                "server_name": "search",
                "input": {},
            },
            {
                "type": "mcp_tool_result",
                "tool_use_id": "mcptoolu_2",
                "is_error": false,
                "content": [{
                    "url": "https://example.com", "title": "Example", "pageAge": null,
                    "encryptedContent": "abc", "type": "web_search_result",
                }],
            },
            {
                "type": "web_search_tool_result",
                "tool_use_id": "mcptoolu_2",
                "content": [{
                    "url": "https://example.com", "title": "Example", "page_age": null,
                    "encrypted_content": "abc", "type": "web_search_result",
                }],
            },
        ])
    );
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
}

/// An `error-json` result of an MCP tool named like a tool search tool is sent
/// as an `mcp_tool_result` and warned about: upstream's MCP branch falls into
/// the tool search check, which takes only `json`
/// (convert-to-anthropic-prompt.ts:1026-1046, 1338-1350). A `json` result adds
/// the `tool_search_tool_result` with no warning.
#[test]
fn mcp_tool_named_like_tool_search_warns_for_error_json() {
    let prompt = |output: serde_json::Value| -> LanguageModelPrompt {
        serde_json::from_value(json!([{
            "role": "assistant",
            "content": [
                {
                    "type": "tool-call",
                    "toolCallId": "mcptoolu_3",
                    "toolName": "tool_search_tool_regex",
                    "input": {},
                    "providerExecuted": true,
                    "providerOptions": { "anthropic": { "type": "mcp-tool-use", "serverName": "search" } },
                },
                {
                    "type": "tool-result",
                    "toolCallId": "mcptoolu_3",
                    "toolName": "tool_search_tool_regex",
                    "output": output,
                },
            ],
        }]))
        .unwrap()
    };

    let result = aimux_providers::anthropic::convert::convert_prompt_to_anthropic_full_fallible(
        &prompt(json!({ "type": "error-json", "value": { "errorCode": "unavailable" } })),
        false,
    )
    .unwrap();
    assert_eq!(
        result.messages[0]["content"].as_array().unwrap()[1..],
        [json!({
            "type": "mcp_tool_result",
            "tool_use_id": "mcptoolu_3",
            "is_error": true,
            "content": { "errorCode": "unavailable" },
        })]
    );
    assert_eq!(
        result.warnings,
        vec![Warning::Other {
            message: "provider executed tool result output type error-json for tool tool_search_tool_regex is not supported".into()
        }]
    );

    let result = aimux_providers::anthropic::convert::convert_prompt_to_anthropic_full_fallible(
        &prompt(json!({ "type": "json", "value": [{ "type": "tool-reference", "toolName": "get_weather" }] })),
        false,
    )
    .unwrap();
    assert_eq!(
        result.messages[0]["content"][2],
        json!({
            "type": "tool_search_tool_result",
            "tool_use_id": "mcptoolu_3",
            "content": {
                "type": "tool_search_tool_search_result",
                "tool_references": [{ "type": "tool_reference", "tool_name": "get_weather" }],
            },
        })
    );
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
}

/// Upstream surfaces a response `fallback` block as an `anthropic.fallback` custom part and sends it back as a `fallback` block between reasoning blocks.
#[tokio::test]
async fn fallback_block_round_trips_as_custom_part() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "id": "msg_fallback", "type": "message", "role": "assistant",
        "model": "claude-opus-4-8",
        "content": [
            { "type": "fallback", "from": { "model": "claude-fable-5" }, "to": { "model": "claude-opus-4-8" } },
            { "type": "text", "text": "Hi" },
        ],
        "stop_reason": "end_turn", "stop_sequence": null,
        "usage": { "input_tokens": 1, "output_tokens": 1 },
    }))]);
    let prompt: LanguageModelPrompt = serde_json::from_value(json!([
        {
            "role": "assistant",
            "content": [
                { "type": "reasoning", "text": "primary", "providerOptions": { "anthropic": { "signature": "sig-1" } } },
                {
                    "type": "custom",
                    "kind": "anthropic.fallback",
                    "providerOptions": { "anthropic": {
                        "type": "fallback",
                        "from": { "model": "claude-fable-5" },
                        "to": { "model": "claude-opus-4-8" },
                    } },
                },
                { "type": "reasoning", "text": "fallback", "providerOptions": { "anthropic": { "signature": "sig-2" } } },
                { "type": "text", "text": "Done." },
            ],
        },
        { "role": "user", "content": [{ "type": "text", "text": "Hello" }] },
    ]))
    .unwrap();
    let result = make_model_with_config(settings(&fetch))
        .do_generate(&default_options(prompt))
        .await
        .unwrap();
    let body = fetch.seen()[0].json_body();
    assert_eq!(
        body["messages"][0]["content"],
        json!([
            { "type": "thinking", "thinking": "primary", "signature": "sig-1" },
            { "type": "fallback", "from": { "model": "claude-fable-5" }, "to": { "model": "claude-opus-4-8" } },
            { "type": "thinking", "thinking": "fallback", "signature": "sig-2" },
            { "type": "text", "text": "Done." },
        ])
    );
    assert_eq!(
        serde_json::to_value(&result.content).unwrap()[0],
        json!({
            "type": "custom",
            "kind": "anthropic.fallback",
            "providerMetadata": { "anthropic": {
                "type": "fallback",
                "from": { "model": "claude-fable-5" },
                "to": { "model": "claude-opus-4-8" },
            } },
        })
    );
}

/// The AI SDK leaves `citations` out of a web_fetch result whose document has none.
#[tokio::test]
async fn web_fetch_result_without_citations_omits_the_key() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "id": "msg_123", "type": "message", "role": "assistant",
        "content": [
            {
                "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_fetch",
                "input": { "url": "https://example.com" }
            },
            {
                "type": "web_fetch_tool_result", "tool_use_id": "srvtoolu_1",
                "content": {
                    "type": "web_fetch_result", "url": "https://example.com",
                    "retrieved_at": "2025-09-29T07:28:57.560000+00:00",
                    "content": {
                        "type": "document", "title": null,
                        "source": { "type": "text", "media_type": "text/plain", "data": "Hi" }
                    }
                }
            }
        ],
        "model": "claude-3-haiku-20240307", "stop_reason": "end_turn",
        "stop_sequence": null, "usage": { "input_tokens": 1, "output_tokens": 1 }
    }))]);
    let mut opts = default_options(test_prompt());
    opts.tools = Some(vec![Tool::Provider(ProviderTool {
        id: "anthropic.web_fetch_20250910".parse().unwrap(),
        name: "web_fetch".to_string(),
        args: serde_json::Map::new(),
        supports_deferred_results: None,
    })]);
    let result = make_model_with_config(settings(&fetch))
        .do_generate(&opts)
        .await
        .unwrap();
    let document = result
        .content
        .iter()
        .find_map(|part| match part {
            GenerateContent::ToolResult(tool_result) => Some(tool_result.result["content"].clone()),
            _ => None,
        })
        .expect("a web_fetch tool result");
    assert_eq!(
        document,
        json!({
            "type": "document",
            "title": null,
            "source": { "type": "text", "mediaType": "text/plain", "data": "Hi" }
        })
    );
}

/// The AI SDK sends `providerOptions.anthropic.safeguards` as `safeguards`
/// with the `dangerous-tool-use-2026-09-03` beta.
#[tokio::test]
async fn sends_safeguards_and_their_beta() {
    let fetch = mock();
    let mut opts = default_options(test_prompt());
    opts.provider_options = Some(
        provider_namespace(
            "anthropic",
            json!({
                "safeguards": [
                    { "type": "dangerous_tool_use", "classifierContext": { "v": 1 } },
                    { "type": "dangerous_tool_use" }
                ]
            }),
        )
        .unwrap(),
    );
    make_model_with_config(settings(&fetch))
        .do_generate(&opts)
        .await
        .unwrap();
    let requests = fetch.seen();
    assert_eq!(
        requests[0].json_body()["safeguards"],
        json!([
            { "type": "dangerous_tool_use", "classifier_context": { "v": 1 } },
            { "type": "dangerous_tool_use" }
        ])
    );
    assert_eq!(
        requests[0]
            .headers
            .get("anthropic-beta")
            .map(String::as_str),
        Some("dangerous-tool-use-2026-09-03")
    );
}

/// The AI SDK knows Vertex ids such as `claude-sonnet-4@20250514`
/// (`/claude-sonnet-4(?:-|@)/`) and defaults their `max_tokens` to 64000.
#[tokio::test]
async fn knows_vertex_claude_4_model_ids() {
    for (model_id, max_tokens) in [
        ("claude-sonnet-4@20250514", 64000),
        ("claude-opus-4@20250514", 32000),
    ] {
        let fetch = mock();
        let result = create_anthropic(settings(&fetch))
            .unwrap()
            .messages(model_id)
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();
        assert_eq!(
            fetch.seen()[0].json_body()["max_tokens"],
            json!(max_tokens),
            "{model_id}"
        );
        assert!(
            result.warnings.is_empty(),
            "{model_id}: {:?}",
            result.warnings
        );
    }
}

/// The AI SDK joins `anthropic-beta` in insertion order: prompt betas, then
/// provider-option betas, tool betas, header betas, and `anthropicBeta`.
#[tokio::test]
async fn joins_beta_headers_in_insertion_order() {
    let fetch = mock();
    let model = create_anthropic(AnthropicProviderSettings {
        headers: Some(
            [("anthropic-beta".to_string(), Some("zz-config".to_string()))]
                .into_iter()
                .collect(),
        ),
        ..settings(&fetch)
    })
    .unwrap()
    .messages("claude-3-haiku-20240307");
    let mut opts = default_options(test_prompt());
    opts.headers = Some(
        [("anthropic-beta".to_string(), "bb-request".to_string())]
            .into_iter()
            .collect(),
    );
    opts.provider_options = Some(
        provider_namespace(
            "anthropic",
            json!({
                "fallbacks": "default",
                "speed": "fast",
                "container": { "skills": [{ "type": "anthropic", "skillId": "pptx" }] },
                "mcpServers": [{ "type": "url", "name": "echo", "url": "https://example.com/mcp" }],
                "anthropicBeta": ["aa-option"]
            }),
        )
        .unwrap(),
    );
    opts.tools = Some(vec![Tool::Provider(ProviderTool {
        id: "anthropic.web_fetch_20250910".parse().unwrap(),
        name: "web_fetch".to_string(),
        args: serde_json::Map::new(),
        supports_deferred_results: None,
    })]);
    model.do_generate(&opts).await.unwrap();
    assert_eq!(
        fetch.seen()[0].headers["anthropic-beta"],
        "mcp-client-2025-04-04,code-execution-2025-08-25,skills-2025-10-02,\
         files-api-2025-04-14,fast-mode-2026-02-01,server-side-fallback-2026-07-01,\
         web-fetch-2025-09-10,zz-config,bb-request,aa-option"
    );
}

/// A `text/event-stream` response of one `data:` event per chunk.
fn sse_response(chunks: &[Value]) -> Canned {
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: chunks
            .iter()
            .map(|chunk| format!("data: {chunk}\n\n"))
            .collect::<String>()
            .into_bytes(),
    }
}

fn stream_message_start(id: &str) -> Value {
    json!({
        "type": "message_start",
        "message": {
            "id": id, "type": "message", "role": "assistant", "content": [],
            "model": "claude-3-haiku-20240307", "stop_reason": null, "stop_sequence": null,
            "usage": { "input_tokens": 17, "output_tokens": 1 }
        }
    })
}

/// `doStream`, then every stream item.
async fn stream_items(
    fetch: &Arc<MockFetch>,
    options: &CallOptions,
) -> Vec<Result<StreamPart, AiMuxError>> {
    make_model_with_config(settings(fetch))
        .do_stream(options)
        .await
        .unwrap()
        .stream
        .collect()
        .await
}

/// The AI SDK keeps a streamed code-execution call's input as the streamed
/// text with its `type` injected; it does not re-serialize it.
#[tokio::test]
async fn streamed_code_execution_tool_call_keeps_the_streamed_input_text() {
    let fetch = MockFetch::new(vec![sse_response(&[
        stream_message_start("msg_1"),
        json!({
            "type": "content_block_start", "index": 0,
            "content_block": {
                "type": "server_tool_use", "id": "srvtoolu_1",
                "name": "bash_code_execution", "input": {}
            }
        }),
        json!({
            "type": "content_block_delta", "index": 0,
            "delta": { "type": "input_json_delta", "partial_json": "{\"command\": \"ls -la\"}" }
        }),
        json!({ "type": "content_block_stop", "index": 0 }),
        json!({
            "type": "message_delta",
            "delta": { "stop_reason": "end_turn", "stop_sequence": null },
            "usage": { "output_tokens": 5 }
        }),
        json!({ "type": "message_stop" }),
    ])]);
    let mut options = default_options(test_prompt());
    options.tools = Some(vec![Tool::Provider(ProviderTool {
        id: "anthropic.code_execution_20250825".parse().unwrap(),
        name: "code_execution".to_string(),
        args: serde_json::Map::new(),
        supports_deferred_results: None,
    })]);
    let items = stream_items(&fetch, &options).await;
    let inputs: Vec<_> = items
        .iter()
        .filter_map(|item| match item {
            Ok(StreamPart::ToolCall(call)) => Some(call.input.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        inputs,
        [r#"{"type": "bash_code_execution","command": "ls -la"}"#]
    );
}

/// The AI SDK keeps reading after `message_stop`: each later
/// `message_start … message_stop` round streams its parts and its own finish.
#[tokio::test]
async fn stream_continues_after_message_stop_with_the_next_message() {
    let text_block = |text: &str| {
        [
            json!({
                "type": "content_block_start", "index": 0,
                "content_block": { "type": "text", "text": "" }
            }),
            json!({
                "type": "content_block_delta", "index": 0,
                "delta": { "type": "text_delta", "text": text }
            }),
            json!({ "type": "content_block_stop", "index": 0 }),
        ]
    };
    let mut chunks = vec![stream_message_start("msg_1")];
    chunks.extend(text_block("first"));
    chunks.extend([
        json!({
            "type": "message_delta",
            "delta": { "stop_reason": "pause_turn", "stop_sequence": null },
            "usage": { "output_tokens": 5 }
        }),
        json!({ "type": "message_stop" }),
        stream_message_start("msg_2"),
    ]);
    chunks.extend(text_block("second"));
    chunks.extend([
        json!({
            "type": "message_delta",
            "delta": { "stop_reason": "end_turn", "stop_sequence": null },
            "usage": { "output_tokens": 7 }
        }),
        json!({ "type": "message_stop" }),
    ]);
    let fetch = MockFetch::new(vec![sse_response(&chunks)]);
    let items = stream_items(&fetch, &default_options(test_prompt())).await;
    let summary: Vec<String> = items
        .iter()
        .filter_map(|item| match item.as_ref().unwrap() {
            StreamPart::ResponseMetadata(metadata) => {
                Some(format!("response {}", metadata.id.as_deref().unwrap()))
            }
            StreamPart::TextDelta { delta, .. } => Some(format!("text {delta}")),
            StreamPart::Finish {
                finish_reason,
                usage,
                ..
            } => Some(format!(
                "finish {:?} {}",
                finish_reason.raw,
                usage.output_tokens.total.unwrap()
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        summary,
        [
            "response msg_1",
            "text first",
            r#"finish Some("pause_turn") 5"#,
            "response msg_2",
            "text second",
            r#"finish Some("end_turn") 7"#,
        ]
    );
}

/// The AI SDK streams the `tool_use` blocks a `message_start` already
/// carries (deferred programmatic tool calls) as complete tool calls.
#[tokio::test]
async fn stream_emits_the_tool_calls_carried_by_message_start() {
    let mut start = stream_message_start("msg_1");
    start["message"]["content"] = json!([{
        "type": "tool_use", "id": "toolu_1", "name": "rollDie",
        "input": { "player": "player1" },
        "caller": { "type": "code_execution_20250825", "tool_id": "srvtoolu_1" }
    }]);
    start["message"]["stop_reason"] = json!("tool_use");
    let fetch = MockFetch::new(vec![sse_response(&[
        start,
        json!({ "type": "message_stop" }),
    ])]);
    let items = stream_items(&fetch, &default_options(test_prompt())).await;
    let parts: Vec<Value> = items
        .iter()
        .map(|item| serde_json::to_value(item.as_ref().unwrap()).unwrap())
        .filter(|part| part["type"].as_str().unwrap().starts_with("tool-"))
        .collect();
    assert_eq!(
        parts,
        [
            json!({ "type": "tool-input-start", "id": "toolu_1", "toolName": "rollDie" }),
            json!({
                "type": "tool-input-delta", "id": "toolu_1",
                "delta": "{\"player\":\"player1\"}"
            }),
            json!({ "type": "tool-input-end", "id": "toolu_1" }),
            json!({
                "type": "tool-call", "toolCallId": "toolu_1", "toolName": "rollDie",
                "input": "{\"player\":\"player1\"}",
                "providerMetadata": {
                    "anthropic": {
                        "caller": { "type": "code_execution_20250825", "toolId": "srvtoolu_1" }
                    }
                }
            }),
        ]
    );
}

/// The AI SDK throws a first-chunk stream error with the error's data as the
/// response body, a 500 status when its type maps to none, and no retry.
#[tokio::test]
async fn first_chunk_stream_error_body_is_the_error_data() {
    let cases = [
        (
            json!({ "type": "overloaded_error", "message": "Overloaded", "details": null }),
            r#"{"type":"overloaded_error","message":"Overloaded"}"#,
            529,
            true,
        ),
        (
            json!({ "type": "unknown_error", "message": "boom", "data": { "a": 1 } }),
            r#"{"a":1}"#,
            500,
            false,
        ),
    ];
    for (error, body, status, retryable) in cases {
        let fetch = MockFetch::new(vec![sse_response(&[
            json!({ "type": "error", "error": error }),
        ])]);
        let Err(AiMuxError::ApiCall(details)) = make_model_with_config(settings(&fetch))
            .do_stream(&default_options(test_prompt()))
            .await
        else {
            panic!("expected an API call error");
        };
        assert_eq!(
            (
                details.response_body.as_deref(),
                details.status_code,
                details.is_retryable
            ),
            (Some(body), Some(status), retryable)
        );
    }
}

/// The AI SDK's error-chunk schema keeps `code`, `statusCode` and
/// `isRetryable` present as `null` (`nullish()`, anthropic-api.ts:1450-1459),
/// so the error data it throws with keeps them; a `null` status or retry flag
/// still falls back to the type's (anthropic-language-model.ts:75-93).
#[tokio::test]
async fn first_chunk_stream_error_keeps_explicit_null_fields() {
    let cases = [
        (
            json!({ "type": "overloaded_error", "message": "Overloaded", "code": null, "statusCode": null, "isRetryable": null }),
            r#"{"type":"overloaded_error","message":"Overloaded","code":null,"statusCode":null,"isRetryable":null}"#,
            529,
            true,
        ),
        (
            json!({ "type": "unknown_error", "message": "boom", "isRetryable": null }),
            r#"{"type":"unknown_error","message":"boom","isRetryable":null}"#,
            500,
            false,
        ),
    ];
    for (error, body, status, retryable) in cases {
        let fetch = MockFetch::new(vec![sse_response(&[
            json!({ "type": "error", "error": error }),
        ])]);
        let Err(AiMuxError::ApiCall(details)) = make_model_with_config(settings(&fetch))
            .do_stream(&default_options(test_prompt()))
            .await
        else {
            panic!("expected an API call error");
        };
        assert_eq!(
            (
                details.response_body.as_deref(),
                details.status_code,
                details.is_retryable
            ),
            (Some(body), Some(status), retryable)
        );
    }
}
