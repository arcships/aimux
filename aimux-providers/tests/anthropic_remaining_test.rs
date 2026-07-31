//! Remaining Anthropic provider tests — ported from the TS SDK suite.
//!
//! Covers the TS tests not already exercised by the existing six Anthropic
//! test files (`anthropic_model_test.rs`, `anthropic_convert_test.rs`,
//! `anthropic_convert_full_test.rs`, `anthropic_prepare_tools_test.rs`,
//! `anthropic_provider_tools_test.rs`, `anthropic_cache_control_test.rs`,
//! `anthropic_aws_model_test.rs`):
//!
//! - `anthropic-provider.test.ts` — provider configuration (baseURL, auth,
//!   custom provider name, `supportedUrls`).
//! - `anthropic-unknown-model-max-output-tokens.test.ts` — unknown-model
//!   `max_tokens` defaulting + compatibility warning.
//! - `sanitize-json-schema.test.ts` — JSON Schema sanitization (the
//!   `sanitize_json_schema.rs` implementation had no dedicated tests).
//! - `anthropic-language-model.test.ts` → `mid-conversation tool changes`
//!   describe block.
//!
//! HTTP is mocked with `wiremock` (a real loopback HTTP server). Env-var
//! tests are `#[serial]` and bracketed with `unsafe` `set_var`/`remove_var`
//! per the workspace convention.
//!
//! Tests for behaviour the Rust implementation does not yet expose
//! (`supportedUrls`, `toolChanges`, empty-`baseURL` rejection) are written
//! against the intended contract and marked `#[ignore]` with a
//! `// TODO: implementation gap` note, so they document the gap without
//! breaking the suite.

use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::types::Warning;

use aimux_providers::anthropic::AnthropicConfig;
use aimux_providers::anthropic::model::AnthropicModel;
use aimux_providers::anthropic::sanitize_json_schema::sanitize_json_schema;

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers
// ─────────────────────────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

/// Build `CallOptions` with everything unset except `prompt`.
fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// Build an `AnthropicModel` backed by `config` (model id `claude-3-haiku-20240307`).
fn make_model_with_config(config: AnthropicConfig) -> AnthropicModel {
    AnthropicModel::new("claude-3-haiku-20240307".to_string(), config)
}

/// A minimal Anthropic text response body (mirrors the TS `anthropic-text` fixture).
fn text_response(text: &str) -> Value {
    json!({
        "id": "msg_123",
        "type": "message",
        "role": "assistant",
        "content": [{ "type": "text", "text": text }],
        "model": "claude-3-haiku-20240307",
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": { "input_tokens": 1, "output_tokens": 1 },
    })
}

/// Mount a JSON 200 response on `POST /v1/messages`.
async fn mock_messages_ok(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Remove the Anthropic env vars consulted by `AnthropicConfig::from_env`
/// (test isolation for `#[serial]` env tests).
fn clear_anthropic_env() {
    unsafe {
        std::env::remove_var("ANTHROPIC_BASE_URL");
        std::env::remove_var("ANTHROPIC_API_KEY");
    }
}

/// Set the API key (always required by `from_env`) plus an optional base URL.
fn set_anthropic_env(api_key: &str, base_url: Option<&str>) {
    unsafe {
        std::env::set_var("ANTHROPIC_API_KEY", api_key);
        match base_url {
            Some(url) => std::env::set_var("ANTHROPIC_BASE_URL", url),
            None => std::env::remove_var("ANTHROPIC_BASE_URL"),
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// baseURL configuration  (TS: anthropic-provider.test.ts → 'baseURL configuration')
// ═════════════════════════════════════════════════════════════════════════════

/// TS: "uses the default Anthropic base URL when not provided".
///
/// The TS test mocks `fetch` and asserts the request URL is
/// `https://api.anthropic.com/v1/messages`. The Rust provider builds the
/// endpoint as `{base_url}/v1/messages`, so the equivalent observable
/// behaviour is: the default config's `base_url` is the bare Anthropic URL,
/// and a request against a stubbed server lands on `/v1/messages`.
#[tokio::test]
async fn default_base_url_when_not_provided() {
    // Config-level: default base URL is the bare Anthropic API URL.
    let config = AnthropicConfig::new("test-api-key");
    assert_eq!(config.base_url, "https://api.anthropic.com");

    // HTTP-level: the endpoint path is `/v1/messages`.
    let server = MockServer::start().await;
    mock_messages_ok(&server, text_response("Hi")).await;
    let model =
        make_model_with_config(AnthropicConfig::new("test-api-key").with_base_url(server.uri()));
    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/v1/messages");
}

/// TS: "uses ANTHROPIC_BASE_URL when set".
///
/// The TS test mocks `fetch` and asserts the request URL is
/// `https://proxy.anthropic.example/v1/messages`. The Rust provider builds the
/// endpoint as `{base_url}/v1/messages`, so the observable behaviour is that
/// `from_env` picks up `ANTHROPIC_BASE_URL` and normalizes it into `base_url`.
/// (The HTTP `/v1/messages` path round-trip is covered by
/// `default_base_url_when_not_provided`.)
#[tokio::test]
#[serial]
async fn uses_anthropic_base_url_env_when_set() {
    clear_anthropic_env();
    set_anthropic_env("test-api-key", Some("https://proxy.anthropic.example/v1/"));

    let config = AnthropicConfig::from_env().expect("from_env");
    // Trailing slash + `/v1` segment normalized away; the endpoint becomes
    // `https://proxy.anthropic.example/v1/messages`.
    assert_eq!(config.base_url, "https://proxy.anthropic.example");

    clear_anthropic_env();
}

/// TS: "normalizes a bare Anthropic API URL from ANTHROPIC_BASE_URL".
#[tokio::test]
#[serial]
async fn normalizes_bare_anthropic_url_from_env() {
    clear_anthropic_env();
    set_anthropic_env("test-api-key", Some("https://api.anthropic.com/"));
    let config = AnthropicConfig::from_env().expect("from_env");
    // Trailing slash stripped; endpoint becomes `https://api.anthropic.com/v1/messages`.
    assert_eq!(config.base_url, "https://api.anthropic.com");
    clear_anthropic_env();
}

/// TS: "normalizes a bare Anthropic API URL from the baseURL option".
#[tokio::test]
async fn normalizes_bare_anthropic_url_from_base_url_option() {
    let config = AnthropicConfig::new("test-api-key").with_base_url("https://api.anthropic.com/");
    assert_eq!(config.base_url, "https://api.anthropic.com");
}

/// TS: "prefers the baseURL option over ANTHROPIC_BASE_URL".
#[tokio::test]
#[serial]
async fn prefers_base_url_option_over_env() {
    clear_anthropic_env();
    set_anthropic_env("test-api-key", Some("https://env.anthropic.example/v1"));
    let config = AnthropicConfig::from_env()
        .expect("from_env")
        .with_base_url("https://option.anthropic.example/v1/");
    // The explicit option wins; its trailing slash and `/v1` are normalized away.
    assert_eq!(config.base_url, "https://option.anthropic.example");
    clear_anthropic_env();
}

/// TS: "rejects an empty baseURL option during provider creation".
// TODO: implementation gap — the Rust `normalize_base_url` returns the default
// Anthropic URL for an empty input instead of rejecting it with
// `InvalidArgumentError` (`argument: "baseURL"`, message: "baseURL must be a
// non-empty string."). Re-enable once `with_base_url` / the builder validate
// emptiness.
#[tokio::test]
#[ignore]
async fn rejects_empty_base_url_option() {
    let result = AnthropicConfig::builder()
        .api_key("test-api-key")
        .base_url("")
        .build();
    match result {
        Err(AiMuxError::InvalidArgument(msg)) => {
            assert_eq!(msg, "baseURL must be a non-empty string.");
        }
        other => panic!(
            "expected InvalidArgument for empty baseURL, got {:?}",
            other
        ),
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Authentication  (TS: anthropic-provider.test.ts → 'authentication')
// ═════════════════════════════════════════════════════════════════════════════

/// TS: "sends Authorization Bearer header when authToken is provided".
#[tokio::test]
async fn sends_authorization_bearer_when_auth_token_provided() {
    let server = MockServer::start().await;
    mock_messages_ok(&server, text_response("Hi")).await;

    let config = AnthropicConfig::builder()
        .auth_token("test-auth-token")
        .base_url(server.uri())
        .build()
        .expect("builder build");
    let model = make_model_with_config(config);

    let _ = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok()),
        Some("Bearer test-auth-token"),
        "Authorization Bearer header missing or wrong"
    );
    // When an auth token is set, the `x-api-key` header must NOT be sent.
    assert!(
        requests[0].headers.get("x-api-key").is_none(),
        "x-api-key should not be sent when authToken is provided"
    );
}

/// TS: "throws error when both apiKey and authToken options are provided".
#[test]
fn throws_when_both_api_key_and_auth_token_provided() {
    let result = AnthropicConfig::builder()
        .api_key("test-api-key")
        .auth_token("test-auth-token")
        .build();
    match result {
        Err(AiMuxError::InvalidArgument(msg)) => {
            assert_eq!(
                msg,
                "Both apiKey and authToken were provided. \
                 Please use only one authentication method."
            );
        }
        other => panic!(
            "expected InvalidArgument for conflicting apiKey + authToken, got {:?}",
            other
        ),
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Custom provider name  (TS: anthropic-provider.test.ts → 'custom provider name')
// ═════════════════════════════════════════════════════════════════════════════

/// TS: "should use custom provider name when specified".
#[test]
fn uses_custom_provider_name_when_specified() {
    let config = AnthropicConfig::new("test-api-key").with_name("my-claude-proxy");
    let model = make_model_with_config(config);
    assert_eq!(model.provider(), "my-claude-proxy");
}

/// TS: "should default to anthropic.messages when name not specified".
#[test]
fn defaults_to_anthropic_messages_when_not_specified() {
    let config = AnthropicConfig::new("test-api-key");
    let model = make_model_with_config(config);
    assert_eq!(model.provider(), "anthropic.messages");
}

// ═════════════════════════════════════════════════════════════════════════════
// supportedUrls  (TS: anthropic-provider.test.ts → 'supportedUrls')
// ═════════════════════════════════════════════════════════════════════════════

/// TS: "should support image/* URLs".
// TODO: implementation gap — the Rust `LanguageModel` trait has no
// `supported_urls` surface, so there is nothing to assert against. Re-enable
// once `supported_urls` is added to the trait and the Anthropic model exposes
// its image/PDF URL allow-list.
#[tokio::test]
#[ignore]
async fn supports_image_urls() {
    let config = AnthropicConfig::new("test-api-key");
    let model = make_model_with_config(config);
    // Intended: `model.supported_urls()["image/*"]` matches
    // `https://example.com/image.png`.
    let _ = model.provider();
}

/// TS: "should support application/pdf URLs".
// TODO: implementation gap — see `supports_image_urls`.
#[tokio::test]
#[ignore]
async fn supports_pdf_urls() {
    let config = AnthropicConfig::new("test-api-key");
    let model = make_model_with_config(config);
    let _ = model.provider();
}

// ═════════════════════════════════════════════════════════════════════════════
// Unknown-model max output tokens  (TS: anthropic-unknown-model-max-output-tokens.test.ts)
// ═════════════════════════════════════════════════════════════════════════════

/// Build a model with the given model id pointing at the wiremock server.
fn make_model_with_id(server: &MockServer, model_id: &str) -> AnthropicModel {
    AnthropicModel::new(
        model_id.to_string(),
        AnthropicConfig::new("test-api-key").with_base_url(server.uri()),
    )
}

/// TS: "should warn when using the default max output token limit".
#[tokio::test]
async fn warns_when_using_default_max_output_token_limit() {
    let server = MockServer::start().await;
    mock_messages_ok(&server, text_response("Hello!")).await;

    let model = make_model_with_id(&server, "future-model");
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
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
        other => panic!("expected Compatibility warning, got {:?}", other),
    }
}

/// TS: "should not warn when max output tokens are provided".
#[tokio::test]
async fn does_not_warn_when_max_output_tokens_provided() {
    let server = MockServer::start().await;
    mock_messages_ok(&server, text_response("Hello!")).await;

    let model = make_model_with_id(&server, "future-model");
    let mut opts = default_options(test_prompt());
    opts.max_output_tokens = Some(123456);
    let result = model
        .do_generate(&opts)
        .await
        .expect("do_generate should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "future-model");
    assert_eq!(body["max_tokens"], 123456);
    assert!(
        result.warnings.is_empty(),
        "warnings: {:?}",
        result.warnings
    );
}

/// TS: "should use the current-generation default and warn for an unknown Claude model".
#[tokio::test]
async fn uses_current_gen_default_and_warns_for_unknown_claude_model() {
    let server = MockServer::start().await;
    mock_messages_ok(&server, text_response("Hello!")).await;

    let model = make_model_with_id(&server, "claude-future-9");
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
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
        other => panic!("expected Compatibility warning, got {:?}", other),
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

// ═════════════════════════════════════════════════════════════════════════════
// Mid-conversation tool changes
// (TS: anthropic-language-model.test.ts → 'mid-conversation tool changes')
// ═════════════════════════════════════════════════════════════════════════════

/// TS: "should send tool change blocks and the beta header".
///
/// The TS test sends a system message carrying
/// `providerOptions.anthropic.toolChanges = [{ type: 'tool_removal', toolName:
/// 'get_weather' }]` and asserts:
/// - `messages` contains a `system` message whose content is
///   `[{ type: 'tool_removal', tool: { type: 'tool_reference', name:
///   'get_weather' } }]`.
/// - the `anthropic-beta` request header contains
///   `mid-conversation-tool-changes-2026-07-01`.
// TODO: implementation gap — the Rust convert does not yet read
// `toolChanges` from provider options, emit `tool_removal`/`tool_addition`
// system blocks, or set the `mid-conversation-tool-changes-2026-07-01` beta
// header. Re-enable once that pipeline is implemented.
#[tokio::test]
#[ignore]
async fn sends_tool_change_blocks_and_beta_header() {
    let server = MockServer::start().await;
    mock_messages_ok(&server, text_response("OK")).await;

    let config = AnthropicConfig::new("test-api-key").with_base_url(server.uri());
    let model = AnthropicModel::new("claude-opus-4-8".to_string(), config);

    // NOTE: when implemented, `toolChanges` should be carried on the system
    // message's provider options. The current `LanguageModelPromptMessage`
    // / `CallOptions` shape may not yet expose the required provider-options
    // slot; that is part of the implementation gap.
    let prompt: LanguageModelPrompt = vec![
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("Say OK.")],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::System,
            content: vec![],
            ..Default::default()
        },
    ];
    let opts = default_options(prompt);

    let _ = model
        .do_generate(&opts)
        .await
        .expect("do_generate should succeed");

    let requests = server.received_requests().await.expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();

    // The system message should carry the tool_removal block.
    let system_msgs: Vec<&Value> = body["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .filter(|m| m["role"] == "system")
        .collect();
    assert!(
        system_msgs.iter().any(|m| {
            m["content"]
                .as_array()
                .map(|c| {
                    c.iter().any(|block| {
                        block["type"] == "tool_removal"
                            && block["tool"]["type"] == "tool_reference"
                            && block["tool"]["name"] == "get_weather"
                    })
                })
                .unwrap_or(false)
        }),
        "expected a system tool_removal block, got body: {}",
        body
    );

    // The beta header should advertise mid-conversation tool changes.
    let beta = requests[0]
        .headers
        .get("anthropic-beta")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        beta.contains("mid-conversation-tool-changes-2026-07-01"),
        "missing beta header, got: {}",
        beta
    );
}
