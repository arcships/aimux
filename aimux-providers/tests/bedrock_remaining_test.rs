//! Remaining (not-yet-covered) Amazon Bedrock TS test cases, translated to Rust.
//!
//! Translation sources (under `reference/ai/packages/amazon-bedrock/src/`):
//! - `normalize-tool-call-id.test.ts` — `isMistralModel` / `normalizeToolCallId`
//!   (helpers not ported to Rust → `#[ignore]`).
//! - `convert-amazon-bedrock-usage.test.ts` — the two `raw`-echo cases (the
//!   Rust `Usage` type has no `raw` field → `#[ignore]`).
//! - `amazon-bedrock-provider.test.ts` — `from_env` auth variants + provider
//!   plumbing (green).
//! - `convert-to-amazon-bedrock-chat-messages.test.ts` — Mistral tool-call-id
//!   normalization cases (the Rust converter has no `isMistral` parameter →
//!   `#[ignore]`).
//! - `amazon-bedrock-chat-language-model.test.ts` — ARN URL encoding,
//!   `additionalModelResponseFieldPaths`, temperature clamping, guardrails,
//!   `trace` / `stop_sequence` extraction into `providerMetadata`,
//!   `supportedUrls`, tool calls with empty input, tool-content filtering.
//!
//! Tests that fail because the Rust implementation does not yet support the
//! feature are marked `#[ignore = "TODO: implementation gap"]`. The test body
//! documents the expected behaviour so it can be flipped on once the gap lands.

use std::collections::HashMap;

use futures::StreamExt;
use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, Tool, ToolChoice};
use aimux_core::provider::Provider;
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::FinishReasonUnified;

use aimux_providers::bedrock::{
    BedrockAuth, BedrockConfig, BedrockModel, BedrockProvider, BedrockProviderConfig,
};

// ── Shared helpers ───────────────────────────────────────────────────────────

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

fn make_model(server: &MockServer) -> BedrockModel {
    BedrockModel::new(
        "anthropic.claude-3-5-sonnet-20240620-v1:0".to_string(),
        BedrockConfig {
            base_url: server.uri(),
            auth: BedrockAuth::BearerToken("test-token".to_string()),
            retry_config: aimux_provider_utils::RetryConfig::default(),
            api_key_source: None,
        },
    )
}

async fn mock_converse_json(server: &MockServer, status: u16, body: Value) {
    let model_path = "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse";
    Mock::given(method("POST"))
        .and(path(model_path))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

fn ok_converse_body() -> Value {
    json!({
        "output": {
            "message": {
                "role": "assistant",
                "content": [{ "text": "ok" }]
            }
        },
        "stopReason": "end_turn",
        "usage": { "inputTokens": 4, "outputTokens": 7, "totalTokens": 11 }
    })
}

fn as_text(item: &GenerateContent) -> &str {
    match item {
        GenerateContent::Text { text, .. } => text,
        _ => panic!("expected Text content, got {item:?}"),
    }
}

fn as_tool_call(item: &GenerateContent) -> (&str, &str, &str) {
    match item {
        GenerateContent::ToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        } => (tool_call_id, tool_name, input),
        _ => panic!("expected ToolCall content, got {item:?}"),
    }
}

async fn collect_stream(result: aimux_core::result::StreamResult) -> Vec<StreamPart> {
    let mut parts = Vec::new();
    let mut stream = result.stream;
    while let Some(part) = stream.next().await {
        match part {
            Ok(p) => parts.push(p),
            Err(e) => panic!("stream error: {e:?}"),
        }
    }
    parts
}

/// Remove every env var that `BedrockProviderConfig::from_env` consults, so each
/// `#[serial]` test starts from a clean slate.
fn clear_bedrock_env() {
    unsafe {
        std::env::remove_var("AWS_BEARER_TOKEN_BEDROCK");
        std::env::remove_var("AWS_ACCESS_KEY_ID");
        std::env::remove_var("AWS_SECRET_ACCESS_KEY");
        std::env::remove_var("AWS_REGION");
        std::env::remove_var("AWS_SESSION_TOKEN");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// normalize-tool-call-id.test.ts
// ════════════════════════════════════════════════════════════════════════════
//
// The TS SDK exposes `isMistralModel(modelId)` and
// `normalizeToolCallId(toolCallId, isMistral)`. Neither helper is ported to the
// Rust bedrock module — `convert_prompt_to_bedrock` has no `isMistral`
// parameter and never normalizes tool-call IDs. The tests below document the
// expected behaviour; they are `#[ignore]` until the helpers land.

/// TS: "should return true for mistral models"
#[test]
#[ignore = "TODO: implementation gap — is_mistral_model not implemented in bedrock module"]
fn is_mistral_model_true_for_mistral() {
    // Expected: is_mistral_model returns true for:
    //   "mistral.mistral-7b-instruct-v0:2"
    //   "mistral.mixtral-8x7b-instruct-v0:1"
    //   "mistral.mistral-large-2402-v1:0"
    //   "mistral.mistral-small-2402-v1:0"
    //   "mistral.mistral-large-2407-v1:0"
    //   "mistral.ministral-3-14b-instruct"
    //   "mistral.ministral-3-8b-instruct"
    todo!("is_mistral_model is not implemented");
}

/// TS: "should return true for region-prefixed mistral models"
#[test]
#[ignore = "TODO: implementation gap — is_mistral_model not implemented in bedrock module"]
fn is_mistral_model_region_prefixed() {
    // Expected: is_mistral_model("us.mistral.pixtral-large-2502-v1:0") == true
    // Expected: is_mistral_model("eu.mistral.mistral-large-2407-v1:0") == true
    todo!("is_mistral_model is not implemented");
}

/// TS: "should return false for non-mistral models"
#[test]
#[ignore = "TODO: implementation gap — is_mistral_model not implemented in bedrock module"]
fn is_mistral_model_false_for_non_mistral() {
    // Expected: is_mistral_model returns false for:
    //   "anthropic.claude-3-5-sonnet-20241022-v2:0"
    //   "amazon.nova-pro-v1:0"
    //   "openai.gpt-4o"
    //   "meta.llama3-70b-instruct-v1:0"
    todo!("is_mistral_model is not implemented");
}

/// TS: "should return the original ID when not a Mistral model"
#[test]
#[ignore = "TODO: implementation gap — normalize_tool_call_id not implemented in bedrock module"]
fn normalize_tool_call_id_passthrough_when_not_mistral() {
    // Expected: normalize_tool_call_id("tooluse_bpe71yCfRu2b5i-nKGDr5g", false)
    //           == "tooluse_bpe71yCfRu2b5i-nKGDr5g"
    todo!("normalize_tool_call_id is not implemented");
}

/// TS: "should extract first 9 alphanumeric characters for Mistral models"
#[test]
#[ignore = "TODO: implementation gap — normalize_tool_call_id not implemented in bedrock module"]
fn normalize_tool_call_id_first_9_alphanumeric() {
    // Expected: normalize_tool_call_id("tooluse_bpe71yCfRu2b5i-nKGDr5g", true)
    //           == "toolusebp"
    todo!("normalize_tool_call_id is not implemented");
}

/// TS: "should handle IDs with various special characters"
#[test]
#[ignore = "TODO: implementation gap — normalize_tool_call_id not implemented in bedrock module"]
fn normalize_tool_call_id_special_chars() {
    // Expected: normalize_tool_call_id("tool-use_123ABC456", true) == "tooluse12"
    // Expected: normalize_tool_call_id("___abc123DEF___", true)   == "abc123DEF"
    todo!("normalize_tool_call_id is not implemented");
}

/// TS: "should handle IDs that are already alphanumeric"
#[test]
#[ignore = "TODO: implementation gap — normalize_tool_call_id not implemented in bedrock module"]
fn normalize_tool_call_id_already_alphanumeric() {
    // Expected: normalize_tool_call_id("abcdefghi", true) == "abcdefghi"
    // Expected: normalize_tool_call_id("abc123XYZ", true) == "abc123XYZ"
    todo!("normalize_tool_call_id is not implemented");
}

/// TS: "should handle short IDs"
#[test]
#[ignore = "TODO: implementation gap — normalize_tool_call_id not implemented in bedrock module"]
fn normalize_tool_call_id_short_ids() {
    // Expected: normalize_tool_call_id("abc", true)    == "abc"
    // Expected: normalize_tool_call_id("12345", true)  == "12345"
    todo!("normalize_tool_call_id is not implemented");
}

/// TS: "should handle IDs with only special characters"
#[test]
#[ignore = "TODO: implementation gap — normalize_tool_call_id not implemented in bedrock module"]
fn normalize_tool_call_id_only_special_chars() {
    // Expected: normalize_tool_call_id("___---___", true) == ""
    todo!("normalize_tool_call_id is not implemented");
}

/// TS: "should produce valid Mistral tool call IDs (9 alphanumeric chars)"
#[test]
#[ignore = "TODO: implementation gap — normalize_tool_call_id not implemented in bedrock module"]
fn normalize_tool_call_id_valid_mistral_format() {
    // Expected: normalize_tool_call_id("tooluse_bpe71yCfRu2b5i-nKGDr5g", true)
    //           matches ^[a-zA-Z0-9]{1,9}$
    todo!("normalize_tool_call_id is not implemented");
}

// ════════════════════════════════════════════════════════════════════════════
// convert-amazon-bedrock-usage.test.ts — `raw` echo cases
// ════════════════════════════════════════════════════════════════════════════
//
// The non-raw cases are already covered in bedrock_convert_test.rs. The two
// `raw`-only cases below require a `raw` echo field on `Usage`, which the Rust
// type does not model.

use aimux_providers::bedrock::convert::{BedrockUsage, convert_usage};

fn bedrock_usage_with_total(
    input: u32,
    output: u32,
    total: Option<u32>,
    cache_read: Option<u32>,
    cache_write: Option<u32>,
) -> BedrockUsage {
    BedrockUsage {
        input_tokens: Some(input),
        output_tokens: Some(output),
        total_tokens: total,
        cache_read_input_tokens: cache_read,
        cache_write_input_tokens: cache_write,
    }
}

/// TS: "should include totalTokens in raw when provided"
#[test]
#[ignore = "TODO: implementation gap — Usage type has no raw echo field"]
fn usage_raw_includes_total_tokens() {
    let u = convert_usage(Some(&bedrock_usage_with_total(
        100,
        50,
        Some(150),
        None,
        None,
    )));
    // TS asserts result.raw == { inputTokens: 100, outputTokens: 50, totalTokens: 150 }
    // The Rust Usage has no raw field; only the token breakdown is available.
    assert_eq!(u.input_tokens.total, Some(100));
    assert_eq!(u.output_tokens.total, Some(50));
    todo!("Usage.raw echo is not modelled");
}

/// TS: "should preserve raw usage data"
#[test]
#[ignore = "TODO: implementation gap — Usage type has no raw echo field"]
fn usage_raw_preserved() {
    let raw = bedrock_usage_with_total(100, 50, Some(150), Some(80), Some(60));
    let u = convert_usage(Some(&raw));
    // TS asserts result.raw == the full input object.
    assert_eq!(u.input_tokens.total, Some(240));
    todo!("Usage.raw echo is not modelled");
}

// ════════════════════════════════════════════════════════════════════════════
// amazon-bedrock-provider.test.ts — provider configuration / auth
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should create a provider instance with default options" — default
/// region is `us-east-1` and the base URL points at bedrock-runtime.
#[test]
#[serial]
fn provider_default_region_and_base_url() {
    clear_bedrock_env();
    let config = BedrockProviderConfig::new("ak", "sk", "us-east-1");
    assert_eq!(
        config.base_url,
        "https://bedrock-runtime.us-east-1.amazonaws.com"
    );
    assert!(matches!(config.auth, BedrockAuth::SigV4(_)));
}

/// TS: "should create a provider instance with custom options" — custom region
/// flows into the base URL.
#[test]
fn provider_custom_region_base_url() {
    let config = BedrockProviderConfig::new("ak", "sk", "eu-west-1");
    assert_eq!(
        config.base_url,
        "https://bedrock-runtime.eu-west-1.amazonaws.com"
    );
}

/// TS: "should accept a credentialProvider in options" — the Rust port models
/// only static SigV4 creds + bearer; the `with_base_url` override mirrors the
/// custom baseURL option.
#[test]
fn provider_with_base_url_override() {
    let config = BedrockProviderConfig::with_bearer_token("tok", "us-east-1")
        .with_base_url("https://custom.url/");
    assert_eq!(config.base_url, "https://custom.url");
}

/// TS: "should use API key when provided in options" — bearer-token auth path.
#[test]
fn provider_bearer_token_auth() {
    let config = BedrockProviderConfig::with_bearer_token("test-api-key", "us-east-1");
    match config.auth {
        BedrockAuth::BearerToken(t) => assert_eq!(t, "test-api-key"),
        _ => panic!("expected BearerToken auth"),
    }
}

/// TS: "should use API key from environment variable" — `AWS_BEARER_TOKEN_BEDROCK`
/// takes precedence over SigV4 env vars in `from_env`.
#[test]
#[serial]
fn provider_from_env_bearer_token_precedence() {
    clear_bedrock_env();
    unsafe {
        std::env::set_var("AWS_BEARER_TOKEN_BEDROCK", "env-bearer");
        std::env::set_var("AWS_ACCESS_KEY_ID", "should-not-be-used");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "should-not-be-used");
        std::env::set_var("AWS_REGION", "us-west-2");
    }

    let config = BedrockProviderConfig::from_env().expect("from_env should succeed");
    match config.auth {
        BedrockAuth::BearerToken(t) => assert_eq!(t, "env-bearer"),
        _ => panic!("expected BearerToken auth (bearer takes precedence)"),
    }
    assert!(config.base_url.contains("us-west-2"));

    clear_bedrock_env();
}

/// TS: "should fall back to SigV4 when no API key provided" — `from_env` uses
/// SigV4 when `AWS_BEARER_TOKEN_BEDROCK` is absent.
#[test]
#[serial]
fn provider_from_env_sigv4_fallback() {
    clear_bedrock_env();
    unsafe {
        std::env::set_var("AWS_ACCESS_KEY_ID", "test-ak");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "test-sk");
        std::env::set_var("AWS_REGION", "eu-west-1");
    }

    let config = BedrockProviderConfig::from_env().expect("from_env should succeed");
    match config.auth {
        BedrockAuth::SigV4(creds) => {
            assert_eq!(creds.access_key_id, "test-ak");
            assert_eq!(creds.secret_access_key, "test-sk");
            assert_eq!(creds.region, "eu-west-1");
            assert!(creds.session_token.is_none());
        }
        _ => panic!("expected SigV4 auth"),
    }

    clear_bedrock_env();
}

/// TS: "should maintain backward compatibility with existing SigV4
/// authentication" — `AWS_SESSION_TOKEN` from env is picked up by `from_env`.
#[test]
#[serial]
fn provider_from_env_session_token() {
    clear_bedrock_env();
    unsafe {
        std::env::set_var("AWS_ACCESS_KEY_ID", "ak");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "sk");
        std::env::set_var("AWS_REGION", "us-east-1");
        std::env::set_var("AWS_SESSION_TOKEN", "sts-token");
    }

    let config = BedrockProviderConfig::from_env().expect("from_env should succeed");
    match config.auth {
        BedrockAuth::SigV4(creds) => {
            assert_eq!(creds.session_token.as_deref(), Some("sts-token"));
        }
        _ => panic!("expected SigV4 auth"),
    }

    clear_bedrock_env();
}

/// TS: default region falls back to `us-east-1` when `AWS_REGION` is unset.
#[test]
#[serial]
fn provider_from_env_default_region() {
    clear_bedrock_env();
    unsafe {
        std::env::set_var("AWS_ACCESS_KEY_ID", "ak");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "sk");
    }

    let config = BedrockProviderConfig::from_env().expect("from_env should succeed");
    assert!(config.base_url.contains("us-east-1"));

    clear_bedrock_env();
}

/// TS: `from_env` errors when neither bearer token nor access key is present.
#[test]
#[serial]
fn provider_from_env_missing_credentials_errors() {
    clear_bedrock_env();
    let result = BedrockProviderConfig::from_env();
    assert!(
        result.is_err(),
        "from_env should error without any credentials"
    );
    clear_bedrock_env();
}

/// The `BedrockProvider` exposes its name and can vend language models.
#[test]
fn provider_name_and_language_model() {
    let provider =
        BedrockProvider::new(BedrockProviderConfig::with_bearer_token("tok", "us-east-1"));
    assert_eq!(provider.name(), "amazon-bedrock");

    let model = provider.model("anthropic.claude-3-5-sonnet-20240620-v1:0");
    assert_eq!(
        model.model_id(),
        "anthropic.claude-3-5-sonnet-20240620-v1:0"
    );
    assert_eq!(model.provider(), "amazon-bedrock");
}

/// TS: "should prioritize options.apiKey over environment variable" — explicit
/// bearer token construction does not consult env vars.
#[test]
#[serial]
fn provider_explicit_bearer_over_env() {
    clear_bedrock_env();
    unsafe {
        std::env::set_var("AWS_BEARER_TOKEN_BEDROCK", "env-bearer");
    }
    // Explicit construction with a different token should win over env.
    let config = BedrockProviderConfig::with_bearer_token("explicit-tok", "us-east-1");
    match config.auth {
        BedrockAuth::BearerToken(t) => assert_eq!(t, "explicit-tok"),
        _ => panic!("expected BearerToken auth"),
    }
    clear_bedrock_env();
}

// ════════════════════════════════════════════════════════════════════════════
// convert-to-amazon-bedrock-chat-messages.test.ts — Mistral normalization
// ════════════════════════════════════════════════════════════════════════════
//
// The non-Mistral cases are covered in bedrock_convert_test.rs. The two
// `isMistral: true` cases below require the Rust converter to accept an
// `is_mistral` flag (or auto-detect via model id) and normalize tool-call IDs
// to the first 9 alphanumeric characters. Neither is implemented.

use aimux_providers::bedrock::convert::convert_prompt_to_bedrock;
fn tool_msg(content: Vec<ContentPart>) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role: Role::Tool,
        content,
        provider_options: None,
    }
}

fn assistant_msg(content: Vec<ContentPart>) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role: Role::Assistant,
        content,
        provider_options: None,
    }
}

/// TS: "should normalize tool call IDs in tool results when isMistral is true"
#[test]
#[ignore = "TODO: implementation gap — convert_prompt_to_bedrock has no isMistral parameter"]
fn convert_mistral_normalize_tool_result_id() {
    let original_id = "tooluse_bpe71yCfRu2b5i-nKGDr5g";
    let prompt = vec![tool_msg(vec![ContentPart::tool_result(
        original_id.to_string(),
        json!({ "type": "text", "value": "The result is 42" }),
    )])];
    let (_, messages) = convert_prompt_to_bedrock(&prompt);
    // TS (isMistral: true) expects toolUseId == "toolusebp" (first 9 alphanum).
    // The Rust converter has no isMistral flag, so the ID is passed through
    // unchanged — this assertion fails until the gap is closed.
    assert_eq!(
        messages[0]["content"][0]["toolResult"]["toolUseId"],
        json!("toolusebp")
    );
}

/// TS: "should normalize tool call IDs in tool calls when isMistral is true"
#[test]
#[ignore = "TODO: implementation gap — convert_prompt_to_bedrock has no isMistral parameter"]
fn convert_mistral_normalize_tool_call_id() {
    let original_id = "tooluse_bpe71yCfRu2b5i-nKGDr5g";
    let prompt = vec![assistant_msg(vec![ContentPart::tool_call(
        original_id.to_string(),
        "getWeather".to_string(),
        json!({ "city": "SF" }),
    )])];
    let (_, messages) = convert_prompt_to_bedrock(&prompt);
    // TS (isMistral: true) expects toolUseId == "toolusebp".
    assert_eq!(
        messages[0]["content"][0]["toolUse"]["toolUseId"],
        json!("toolusebp")
    );
}

// ════════════════════════════════════════════════════════════════════════════
// amazon-bedrock-chat-language-model.test.ts — request URL / body / response
// ════════════════════════════════════════════════════════════════════════════

// ── supportedUrls ────────────────────────────────────────────────────────────

/// TS: "should support S3 URLs for image parts" — the TS model exposes a
/// `supportedUrls` map (`{ 'image/*': [/^s3:\/\//] }`). The Rust `BedrockModel`
/// has no `supported_urls` concept, so this test is `#[ignore]`.
#[test]
#[ignore = "TODO: implementation gap — BedrockModel has no supported_urls field"]
fn model_supported_urls_s3() {
    // TS: model.supportedUrls == { 'image/*': [/^s3:\/\//] }
    todo!("supportedUrls is not modelled on BedrockModel");
}

// ── ARN model IDs containing a slash ─────────────────────────────────────────

/// TS: "should generate text through the encoded Converse route".
///
/// ARN inference-profile IDs contain a `/` which must be percent-encoded in the
/// URL path. The TS SDK uses `encodeURIComponent(modelId)`. The Rust
/// `BedrockModel::endpoint` does **not** encode the model id, so the slash is
/// sent raw and the request misses the mocked (encoded) route.
#[tokio::test]
#[ignore = "TODO: implementation gap — endpoint() does not percent-encode the model id"]
async fn arn_model_id_encoded_generate_route() {
    let server = MockServer::start().await;
    let arn = "arn:aws:bedrock:eu-west-1:474668406012:inference-profile/eu.amazon.nova-lite-v1:0";
    let encoded =
        percent_encoding::utf8_percent_encode(arn, percent_encoding::NON_ALPHANUMERIC).to_string();
    let encoded_path = format!("/model/{encoded}/converse");

    Mock::given(method("POST"))
        .and(path(&encoded_path))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "output": { "message": { "role": "assistant", "content": [{ "text": "Hello!" }] } },
            "stopReason": "end_turn",
            "usage": { "inputTokens": 1, "outputTokens": 1 }
        })))
        .mount(&server)
        .await;

    let model = BedrockModel::new(
        arn.to_string(),
        BedrockConfig {
            base_url: server.uri(),
            auth: BedrockAuth::BearerToken("tok".to_string()),
            retry_config: aimux_provider_utils::RetryConfig::default(),
            api_key_source: None,
        },
    );

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed via the encoded route");
    assert_eq!(as_text(&result.content[0]), "Hello!");
}

/// TS: "should stream text through the encoded Converse route".
#[tokio::test]
#[ignore = "TODO: implementation gap — endpoint() does not percent-encode the model id"]
async fn arn_model_id_encoded_stream_route() {
    let server = MockServer::start().await;
    let arn = "arn:aws:bedrock:eu-west-1:474668406012:inference-profile/eu.amazon.nova-lite-v1:0";
    let encoded =
        percent_encoding::utf8_percent_encode(arn, percent_encoding::NON_ALPHANUMERIC).to_string();
    let encoded_path = format!("/model/{encoded}/converse-stream");

    let events: Vec<(&str, &str, &str)> = vec![
        ("event", "messageStart", r#"{"role":"assistant"}"#),
        (
            "event",
            "contentBlockDelta",
            r#"{"contentBlockIndex":0,"delta":{"text":"Hello!"}}"#,
        ),
        ("event", "contentBlockStop", r#"{"contentBlockIndex":0}"#),
        ("event", "messageStop", r#"{"stopReason":"end_turn"}"#),
        (
            "event",
            "metadata",
            r#"{"usage":{"inputTokens":1,"outputTokens":1}}"#,
        ),
    ];
    let body_bytes = aimux_providers::bedrock::event_stream::encode_messages(&events);

    Mock::given(method("POST"))
        .and(path(&encoded_path))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.amazon.eventstream")
                .set_body_raw(body_bytes, "application/vnd.amazon.eventstream"),
        )
        .mount(&server)
        .await;

    let model = BedrockModel::new(
        arn.to_string(),
        BedrockConfig {
            base_url: server.uri(),
            auth: BedrockAuth::BearerToken("tok".to_string()),
            retry_config: aimux_provider_utils::RetryConfig::default(),
            api_key_source: None,
        },
    );

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("should succeed via the encoded stream route");
    let parts = collect_stream(result).await;

    let text: String = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Hello!");
}

// ── additionalModelResponseFieldPaths ────────────────────────────────────────

/// TS: "should return the request body" — the default request body includes
/// `additionalModelResponseFieldPaths: ["/delta/stop_sequence"]` so that the
/// stop_sequence value is returned in `additionalModelResponseFields`. The Rust
/// `build_request_body` does not emit this field.
#[tokio::test]
#[ignore = "TODO: implementation gap — build_request_body does not emit additionalModelResponseFieldPaths"]
async fn request_body_includes_additional_model_response_field_paths() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let body = result.request_body.expect("request body");
    assert_eq!(
        body["additionalModelResponseFieldPaths"],
        json!(["/delta/stop_sequence"])
    );
}

// ── temperature clamping ─────────────────────────────────────────────────────

/// TS: "should clamp temperature above 1 to 1 and add warning".
#[tokio::test]
#[ignore = "TODO: implementation gap — temperature is not clamped to [0, 1]"]
async fn temperature_clamped_above_1() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let model = make_model(&server);
    let mut opts = default_options(test_prompt());
    opts.temperature = Some(1.5);

    let result = model.do_generate(&opts).await.expect("should succeed");

    let body = result.request_body.expect("request body");
    assert_eq!(
        body["inferenceConfig"]["temperature"].as_f64().unwrap(),
        1.0
    );
    assert!(
        result.warnings.iter().any(
            |w| matches!(w, aimux_core::types::Warning::Unsupported { feature, .. }
                if feature == "temperature")
        ),
        "expected an unsupported-temperature warning, got {:?}",
        result.warnings
    );
}

/// TS: "should clamp temperature below 0 to 0 and add warning".
#[tokio::test]
#[ignore = "TODO: implementation gap — temperature is not clamped to [0, 1]"]
async fn temperature_clamped_below_0() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let model = make_model(&server);
    let mut opts = default_options(test_prompt());
    opts.temperature = Some(-0.5);

    let result = model.do_generate(&opts).await.expect("should succeed");

    let body = result.request_body.expect("request body");
    assert_eq!(
        body["inferenceConfig"]["temperature"].as_f64().unwrap(),
        0.0
    );
    assert!(
        result.warnings.iter().any(
            |w| matches!(w, aimux_core::types::Warning::Unsupported { feature, .. }
                if feature == "temperature")
        ),
        "expected an unsupported-temperature warning"
    );
}

/// TS: "should not clamp valid temperature between 0 and 1".
#[tokio::test]
async fn temperature_not_clamped_in_range() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let model = make_model(&server);
    let mut opts = default_options(test_prompt());
    opts.temperature = Some(0.7);

    let result = model.do_generate(&opts).await.expect("should succeed");

    let body = result.request_body.expect("request body");
    let temp = body["inferenceConfig"]["temperature"].as_f64().unwrap();
    assert!(
        (temp - 0.7).abs() < 1e-6,
        "temperature should be 0.7, got {temp}"
    );
    assert!(result.warnings.is_empty(), "no warnings expected");
}

// ── guardrails ───────────────────────────────────────────────────────────────

/// TS: "should support guardrails" — `providerOptions.bedrock.guardrailConfig`
/// is forwarded as a top-level `guardrailConfig` in the request body.
#[tokio::test]
#[ignore = "TODO: implementation gap — build_request_body does not emit guardrailConfig"]
async fn guardrail_config_in_request_body() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let model = make_model(&server);
    let mut opts = default_options(test_prompt());
    let mut po = HashMap::new();
    po.insert(
        "bedrock".to_string(),
        json!({
            "guardrailConfig": {
                "guardrailIdentifier": "-1",
                "guardrailVersion": "1",
                "trace": "enabled"
            }
        }),
    );
    opts.provider_options = Some(po);

    let result = model.do_generate(&opts).await.expect("should succeed");
    let body = result.request_body.expect("request body");
    assert_eq!(
        body["guardrailConfig"],
        json!({
            "guardrailIdentifier": "-1",
            "guardrailVersion": "1",
            "trace": "enabled"
        })
    );
}

// ── trace in providerMetadata ────────────────────────────────────────────────

/// TS: "should include trace information in providerMetadata" — the response
/// `trace` field is surfaced as `providerMetadata.bedrock.trace`.
#[tokio::test]
#[ignore = "TODO: implementation gap — do_generate does not extract trace into providerMetadata"]
async fn trace_in_provider_metadata() {
    let server = MockServer::start().await;
    let trace = json!({
        "guardrail": {
            "inputAssessment": {
                "1abcd2ef34gh": {
                    "contentPolicy": {
                        "filters": [{
                            "action": "BLOCKED",
                            "confidence": "LOW",
                            "type": "INSULTS"
                        }]
                    }
                }
            }
        }
    });
    Mock::given(method("POST"))
        .and(path("/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "output": { "message": { "role": "assistant", "content": [{ "text": "Hello, World!" }] } },
            "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
            "stopReason": "stop_sequence",
            "trace": trace
        })))
        .mount(&server)
        .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let pm = result
        .provider_metadata
        .as_ref()
        .expect("provider_metadata should be Some");
    assert_eq!(pm["bedrock"]["trace"], trace);
}

// ── stop_sequence in providerMetadata ────────────────────────────────────────

/// TS: "should include stop_sequence in provider metadata" — when the response
/// carries `additionalModelResponseFields.delta.stop_sequence`, it is surfaced
/// as `providerMetadata.bedrock.stopSequence` (and `amazonBedrock.stopSequence`).
#[tokio::test]
#[ignore = "TODO: implementation gap — do_generate does not extract stop_sequence from additionalModelResponseFields"]
async fn stop_sequence_in_provider_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "output": { "message": { "role": "assistant", "content": [{ "text": "Hello, World!" }] } },
            "stopReason": "stop_sequence",
            "additionalModelResponseFields": { "delta": { "stop_sequence": "STOP" } },
            "usage": { "inputTokens": 4, "outputTokens": 30, "totalTokens": 34 }
        })))
        .mount(&server)
        .await;

    let model = make_model(&server);
    let mut opts = default_options(test_prompt());
    opts.stop_sequences = Some(vec!["STOP".to_string()]);

    let result = model.do_generate(&opts).await.expect("should succeed");

    let pm = result
        .provider_metadata
        .as_ref()
        .expect("provider_metadata should be Some");
    assert_eq!(pm["bedrock"]["stopSequence"], json!("STOP"));
    assert_eq!(pm["amazonBedrock"]["stopSequence"], json!("STOP"));
}

// ── tool calls with empty input ──────────────────────────────────────────────

/// TS: "should support tool calls with empty input (no arguments)" (stream).
///
/// When a `toolUse` block carries no input deltas, the stream event loop
/// accumulates an empty string and defaults the parsed input to `{}` — matching
/// the TS behaviour. (The non-streaming path does **not** handle this; see
/// `generate_tool_call_empty_input` below.)
#[tokio::test]
async fn stream_tool_call_empty_input() {
    let server = MockServer::start().await;

    let events: Vec<(&str, &str, &str)> = vec![
        ("event", "messageStart", r#"{"role":"assistant"}"#),
        (
            "event",
            "contentBlockStart",
            r#"{"contentBlockIndex":0,"start":{"toolUse":{"name":"updateIssueList","toolUseId":"tool_1"}}}"#,
        ),
        // No input deltas — the tool call has no arguments.
        ("event", "contentBlockStop", r#"{"contentBlockIndex":0}"#),
        ("event", "messageStop", r#"{"stopReason":"tool_use"}"#),
    ];
    let body_bytes = aimux_providers::bedrock::event_stream::encode_messages(&events);

    Mock::given(method("POST"))
        .and(path(
            "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse-stream",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.amazon.eventstream")
                .set_body_raw(body_bytes, "application/vnd.amazon.eventstream"),
        )
        .mount(&server)
        .await;

    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    let tool_calls: Vec<_> = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].0, "tool_1");
    assert_eq!(tool_calls[0].1, "updateIssueList");
    assert_eq!(tool_calls[0].2, Value::String("{}".into()));
}

// ── omit toolConfig ──────────────────────────────────────────────────────────

/// TS: "should omit toolConfig when conversation has tool calls but toolChoice
/// is none" — with `toolChoice: none`, no `toolConfig` is sent even when tools
/// are provided.
#[tokio::test]
async fn omit_tool_config_when_tool_choice_none() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let model = make_model(&server);
    let opts = CallOptions {
        tools: Some(vec![Tool::Function(FunctionTool::new(
            "weather".to_string(),
            json!({ "type": "object", "properties": { "city": { "type": "string" } } }),
        ))]),
        tool_choice: ToolChoice::None,
        ..default_options(test_prompt())
    };

    let result = model.do_generate(&opts).await.expect("should succeed");
    let body = result.request_body.expect("request body");
    assert!(
        body.get("toolConfig").is_none(),
        "toolConfig should be absent when toolChoice is none, got {}",
        body["toolConfig"]
    );
}

/// TS: "should omit toolConfig and filter tool content when conversation has
/// tool calls but no active tools".
///
/// The `toolConfig` omission (tools = `[]`) works in Rust. The *content
/// filtering* — dropping the assistant's `toolUse` block from the messages when
/// there are no active tools — is **not** implemented, so only the toolConfig
/// assertion is live; the filtering assertion is `#[ignore]` below.
#[tokio::test]
async fn omit_tool_config_when_no_active_tools() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let prompt: LanguageModelPrompt = vec![
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("What is the weather in Toronto?")],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::tool_call(
                "tool-call-1".to_string(),
                "weather".to_string(),
                json!({ "city": "Toronto" }),
            )],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Tool,
            content: vec![ContentPart::tool_result(
                "tool-call-1".to_string(),
                json!({ "type": "text", "value": "The weather in Toronto is 20°C." }),
            )],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("Now give me a summary.")],
            ..Default::default()
        },
    ];

    let model = make_model(&server);
    let opts = CallOptions {
        tools: Some(vec![]),
        ..default_options(prompt)
    };

    let result = model.do_generate(&opts).await.expect("should succeed");
    let body = result.request_body.expect("request body");
    assert!(
        body.get("toolConfig").is_none(),
        "toolConfig should be absent when tools list is empty"
    );
}

/// TS: (same test) the assistant's `toolUse` block should be filtered out of
/// the messages when there are no active tools. The Rust converter always
/// emits `toolUse` blocks regardless of active tools.
#[tokio::test]
#[ignore = "TODO: implementation gap — convert_prompt_to_bedrock does not filter toolUse blocks when no active tools"]
async fn filter_tool_content_when_no_active_tools() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let prompt: LanguageModelPrompt = vec![
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("What is the weather in Toronto?")],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::tool_call(
                "tool-call-1".to_string(),
                "weather".to_string(),
                json!({ "city": "Toronto" }),
            )],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Tool,
            content: vec![ContentPart::tool_result(
                "tool-call-1".to_string(),
                json!({ "type": "text", "value": "The weather in Toronto is 20°C." }),
            )],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("Now give me a summary.")],
            ..Default::default()
        },
    ];

    let model = make_model(&server);
    let opts = CallOptions {
        tools: Some(vec![]),
        ..default_options(prompt)
    };

    let result = model.do_generate(&opts).await.expect("should succeed");
    let body = result.request_body.expect("request body");
    // The assistant message should NOT contain a toolUse block.
    let assistant_msg = &body["messages"][1];
    assert_eq!(assistant_msg["role"], "assistant");
    let has_tool_use = assistant_msg["content"]
        .as_array()
        .map(|arr| arr.iter().any(|b| b.get("toolUse").is_some()))
        .unwrap_or(false);
    assert!(
        !has_tool_use,
        "toolUse should be filtered out when no active tools"
    );
}

// ── doGenerate: tool call with empty input (non-streaming) ───────────────────

/// TS variant: a non-streaming `toolUse` with no `input` field should yield
/// `input: {}`. The Rust `BedrockToolUse.input` serde-defaults to `Null`.
#[tokio::test]
#[ignore = "TODO: implementation gap — empty toolUse input deserializes as Null, not {}"]
async fn generate_tool_call_empty_input() {
    let server = MockServer::start().await;
    mock_converse_json(
        &server,
        200,
        json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [
                        {
                            "toolUse": {
                                "toolUseId": "tool_1",
                                "name": "updateIssueList"
                                // no "input" field
                            }
                        }
                    ]
                }
            },
            "stopReason": "tool_use",
            "usage": { "inputTokens": 5, "outputTokens": 5 }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 1);
    let (id, name, input) = as_tool_call(&result.content[0]);
    assert_eq!(id, "tool_1");
    assert_eq!(name, "updateIssueList");
    // TS expects input == {} (empty object), not null.
    assert_eq!(input, &json!("{}"));
}

// ── doGenerate: basic text + finish reason (sanity, already covered but
//    exercised here against the remaining-test helper set) ────────────────────

/// TS: "should pass the model and the messages" — the request body carries the
/// model id implicitly via the URL path and the messages array.
#[tokio::test]
async fn request_body_messages_shape() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let prompt: LanguageModelPrompt = vec![
        LanguageModelPromptMessage {
            role: Role::System,
            content: vec![ContentPart::text("System Prompt")],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("Hello")],
            ..Default::default()
        },
    ];

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(prompt))
        .await
        .expect("ok");
    let body = result.request_body.expect("request body");
    assert_eq!(body["system"], json!([{ "text": "System Prompt" }]));
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"][0]["text"], "Hello");
}

/// TS: "should extract finish reason" — `guardrail_intervened` → ContentFilter.
#[tokio::test]
async fn finish_reason_guardrail_intervened() {
    let server = MockServer::start().await;
    mock_converse_json(
        &server,
        200,
        json!({
            "output": { "message": { "role": "assistant", "content": [{ "text": "" }] } },
            "stopReason": "guardrail_intervened",
            "usage": { "inputTokens": 4, "outputTokens": 1 }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(
        result.finish_reason.unified,
        FinishReasonUnified::ContentFilter
    );
    assert_eq!(
        result.finish_reason.raw.as_deref(),
        Some("guardrail_intervened")
    );
}

/// TS: "should send all tools when toolChoice is auto" — all provided tools
/// appear in the toolConfig, not just a filtered subset.
#[tokio::test]
async fn tool_choice_auto_sends_all_tools() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let model = make_model(&server);
    let opts = CallOptions {
        tools: Some(vec![
            Tool::Function(
                FunctionTool::new("get-weather".to_string(), json!({ "type": "object" }))
                    .with_description("Get weather".to_string()),
            ),
            Tool::Function(
                FunctionTool::new("get-time".to_string(), json!({ "type": "object" }))
                    .with_description("Get time".to_string()),
            ),
        ]),
        tool_choice: ToolChoice::Auto,
        ..default_options(test_prompt())
    };

    let result = model.do_generate(&opts).await.expect("should succeed");
    let body = result.request_body.expect("request body");
    assert_eq!(body["toolConfig"]["toolChoice"], json!({ "auto": {} }));
    let tools = body["toolConfig"]["tools"].as_array().unwrap();
    assert_eq!(
        tools.len(),
        2,
        "both tools should be sent with toolChoice: auto"
    );
    assert_eq!(tools[0]["toolSpec"]["name"], "get-weather");
    assert_eq!(tools[1]["toolSpec"]["name"], "get-time");
}

/// TS: "should omit empty tool descriptions to avoid Bedrock validation errors".
///
/// A function tool with an empty-string description must not emit a
/// `description` field in the `toolSpec`.
#[tokio::test]
async fn tool_empty_description_omitted() {
    let server = MockServer::start().await;
    mock_converse_json(&server, 200, ok_converse_body()).await;

    let model = make_model(&server);
    let opts = CallOptions {
        tools: Some(vec![Tool::Function(
            FunctionTool::new("get-weather".to_string(), json!({ "type": "object" }))
                .with_description("".to_string()),
        )]),
        tool_choice: ToolChoice::Auto,
        ..default_options(test_prompt())
    };

    let result = model.do_generate(&opts).await.expect("should succeed");
    let body = result.request_body.expect("request body");
    let spec = &body["toolConfig"]["tools"][0]["toolSpec"];
    assert!(
        spec.get("description").is_none(),
        "empty description should be omitted"
    );
}

// ── doStream: error handling ─────────────────────────────────────────────────

/// TS: "should handle throttlingException error" (stream) — a 429 stream
/// response maps to `AiMuxError::ApiCall` (429 in `status_code`).
#[tokio::test]
async fn stream_throttling_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path(
            "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse-stream",
        ))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "message": "throttlingException",
            "type": "TooManyRequestsException"
        })))
        .mount(&server)
        .await;

    let model = make_model(&server);
    let result = model.do_stream(&default_options(test_prompt())).await;
    assert!(
        matches!(result, Err(ref e) if e.status_code() == Some(429)),
        "expected RateLimited, got {result:?}"
    );
}

/// TS: "should handle validationException error" (stream) — a 400 stream
/// response surfaces as an error (not a panic).
#[tokio::test]
async fn stream_validation_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path(
            "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse-stream",
        ))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "message": "The model ID is invalid",
            "type": "ValidationException"
        })))
        .mount(&server)
        .await;

    let model = make_model(&server);
    let result = model.do_stream(&default_options(test_prompt())).await;
    assert!(result.is_err(), "expected error for 400 stream response");
}

// ── doStream: request body parity ────────────────────────────────────────────

/// TS: "should return the request body" (stream) — the stream result carries
/// the request body for debugging.
#[tokio::test]
async fn stream_request_body_available() {
    let server = MockServer::start().await;

    let events: Vec<(&str, &str, &str)> = vec![
        ("event", "messageStart", r#"{"role":"assistant"}"#),
        (
            "event",
            "contentBlockDelta",
            r#"{"contentBlockIndex":0,"delta":{"text":"hi"}}"#,
        ),
        ("event", "contentBlockStop", r#"{"contentBlockIndex":0}"#),
        ("event", "messageStop", r#"{"stopReason":"end_turn"}"#),
    ];
    let body_bytes = aimux_providers::bedrock::event_stream::encode_messages(&events);

    Mock::given(method("POST"))
        .and(path(
            "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/converse-stream",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.amazon.eventstream")
                .set_body_raw(body_bytes, "application/vnd.amazon.eventstream"),
        )
        .mount(&server)
        .await;

    let model = make_model(&server);
    let mut opts = default_options(test_prompt());
    opts.max_output_tokens = Some(256);

    let result = model.do_stream(&opts).await.expect("should succeed");
    let body = result.request_body.expect("stream request body");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["inferenceConfig"]["maxTokens"], 256);
}
