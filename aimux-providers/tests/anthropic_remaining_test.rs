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
use aimux_core::shared::provider_namespace;
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::Warning;
use aimux_providers::anthropic::sanitize_json_schema::sanitize_json_schema;
use aimux_providers::anthropic::{
    AnthropicMessagesModel, AnthropicProviderSettings, create_anthropic,
};
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

/// TS: "uses the default Anthropic base URL when not provided".
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

/// TS: "uses ANTHROPIC_BASE_URL when set".
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

/// TS: "normalizes a bare Anthropic API URL from ANTHROPIC_BASE_URL".
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

/// TS: "normalizes a bare Anthropic API URL from the baseURL option".
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

/// TS: "prefers the baseURL option over ANTHROPIC_BASE_URL".
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

/// TS: "rejects an empty baseURL option during provider creation".
#[test]
fn rejects_empty_base_url_option() {
    let result = create_anthropic(AnthropicProviderSettings {
        api_key: Some("test-api-key".into()),
        base_url: Some(String::new()),
        ..Default::default()
    });
    assert!(
        matches!(result, Err(AiMuxError::InvalidArgument(msg)) if msg == "baseURL must be a non-empty string.")
    );
}

/// TS: "sends Authorization Bearer header when authToken is provided".
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

/// TS: "throws error when both apiKey and authToken options are provided".
#[test]
fn throws_when_both_api_key_and_auth_token_provided() {
    let result = create_anthropic(AnthropicProviderSettings {
        api_key: Some("test-api-key".into()),
        auth_token: Some("test-auth-token".into()),
        ..Default::default()
    });
    assert!(
        matches!(result, Err(AiMuxError::InvalidArgument(msg)) if msg == "Both apiKey and authToken were provided. Please use only one authentication method.")
    );
}

/// TS: "should use custom provider name when specified".
#[test]
fn uses_custom_provider_name_when_specified() {
    let model = make_model_with_config(AnthropicProviderSettings {
        name: Some("my-proxy".into()),
        api_key: Some("test-api-key".into()),
        ..Default::default()
    });
    assert_eq!(model.provider(), "my-proxy");
}

/// TS: "should default to anthropic.messages when name not specified".
#[test]
fn defaults_to_anthropic_messages_when_not_specified() {
    let model = make_model_with_config(AnthropicProviderSettings {
        api_key: Some("test-api-key".into()),
        ..Default::default()
    });
    assert_eq!(model.provider(), "anthropic.messages");
}

/// TS: "should support image/* URLs".
#[test]
fn supports_image_urls() {
    let model = make_model_with_config(AnthropicProviderSettings::default());
    assert!(model.supported_urls().0["image/*"][0].is_match("https://example.com/image.png"));
}

/// TS: "should support application/pdf URLs".
#[test]
fn supports_pdf_urls() {
    let model = make_model_with_config(AnthropicProviderSettings::default());
    assert!(
        model.supported_urls().0["application/pdf"][0].is_match("https://arxiv.org/pdf/2401.00001")
    );
}

/// TS: "should warn when using the default max output token limit".
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

/// TS: "should not warn when max output tokens are provided".
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

/// TS: unknown model current-generation default and compatibility warning.
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

// ═════════════════════════════════════════════════════════════════════════════
// sanitize-json-schema  (TS: sanitize-json-schema.test.ts)
// ═════════════════════════════════════════════════════════════════════════════

/// TS: "strips unsupported number constraints and adds readable descriptions".
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

/// TS: "strips unsupported string constraints and unsupported formats".
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

/// TS: "recursively sanitizes arrays, definitions, and composition schemas".
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

/// TS: "converts oneOf to anyOf".
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

/// TS: "does not mutate the input schema".
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

/// TS: "should send tool change blocks and the beta header".
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
        requests[0].headers["anthropic-beta"].contains("mid-conversation-tool-changes-2026-07-01")
    );
}
