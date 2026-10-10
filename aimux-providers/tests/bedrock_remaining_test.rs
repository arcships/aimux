//! Remaining Amazon Bedrock upstream tests, restored from the tool-types branch.
//!
//! Pinned translation sources (`amazon-bedrock/src/`):
//! - `normalize-tool-call-id.test.ts`
//! - `convert-amazon-bedrock-usage.test.ts` (raw echo)
//! - `amazon-bedrock-provider.test.ts` (factory/auth variants)
//! - `convert-to-amazon-bedrock-chat-messages.test.ts` (Mistral IDs)
//! - `amazon-bedrock-chat-language-model.test.ts` (request/response cases)

use aimux_core::tool::RawToolCall;

use futures::StreamExt;
use serde_json::{Value, json};
use serial_test::serial;

use aimux_core::language_model_message::{
    AssistantPart, TextPart, ToolCallPart, ToolPart, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::shared::provider_namespace;
use aimux_provider_utils::Resolvable;
#[path = "common/mock_fetch.rs"]
mod mock_fetch;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::FinishReasonUnified;
use mock_fetch::{Canned, MockFetch};

use aimux_providers::bedrock::convert::{build_request_body_checked, normalize_tool_call_id};
use aimux_providers::bedrock::{
    AmazonBedrockProviderSettings, BedrockModel, create_amazon_bedrock,
};

fn test_tool() -> aimux_core::tool::FunctionTool {
    aimux_core::tool::FunctionTool {
        name: "getWeather".into(),
        description: None,
        input_schema: json!({"type":"object"}),
        strict: None,
        provider_options: None,
        input_examples: None,
    }
}
fn model_normalizes_tool_id(id: &str) -> bool {
    let mut opts = default_options(vec![assistant_msg(vec![tool_call(
        "tooluse_bpe71yCfRu2b5i-nKGDr5g".into(),
        "getWeather".into(),
        json!({}),
    )])]);
    opts.tools = Some(vec![test_tool().into()]);
    let (body, _, _, _) = build_request_body_checked(id, &opts, None).unwrap();
    body["messages"][0]["content"][0]["toolUse"]["toolUseId"] == "8eHypBDcw"
}

fn text(value: &str) -> UserPart {
    UserPart::Text(TextPart {
        text: value.into(),
        provider_options: None,
    })
}
fn tool_call(id: String, name: String, input: Value) -> AssistantPart {
    AssistantPart::ToolCall(ToolCallPart {
        tool_call_id: id,
        tool_name: name,
        input,
        provider_executed: None,
        provider_options: None,
    })
}
fn tool_result(id: String, output: Value) -> ToolPart {
    ToolPart::ToolResult(ToolResultPart {
        tool_call_id: id,
        tool_name: "weather".into(),
        output: serde_json::from_value::<ToolResultOutput>(output).unwrap(),
        provider_options: None,
    })
}

// ── Shared helpers ───────────────────────────────────────────────────────────

fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::User {
        content: vec![text("Hello")],
        provider_options: None,
    }]
}

fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

fn make_model(server: &std::sync::Arc<MockFetch>) -> BedrockModel {
    create_amazon_bedrock(AmazonBedrockProviderSettings {
        fetch: Some(server.transport()),
        base_url: Some("https://bedrock.test".into()),
        api_key: Some(Resolvable::Value("test-token".into())),
        ..Default::default()
    })
    .unwrap()
    .chat("anthropic.claude-3-5-sonnet-20240620-v1:0")
}

fn mock_converse_json(status: u16, body: Value) -> std::sync::Arc<MockFetch> {
    let mut canned = Canned::json(&body);
    canned.status = status;
    MockFetch::new(vec![canned])
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
        GenerateContent::ToolCall(RawToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        }) => (tool_call_id, tool_name, input),
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

/// Clear the authentication environment between serial tests.
fn clear_bedrock_env() {
    unsafe {
        std::env::remove_var("AWS_BEARER_TOKEN_BEDROCK");
        std::env::remove_var("AWS_ACCESS_KEY_ID");
        std::env::remove_var("AWS_SECRET_ACCESS_KEY");
        std::env::remove_var("AWS_REGION");
        std::env::remove_var("AWS_SESSION_TOKEN");
    }
}

/// TS: "should return true for mistral models"
#[test]
fn is_mistral_model_true_for_mistral() {
    for id in [
        "mistral.mistral-7b-instruct-v0:2",
        "mistral.mixtral-8x7b-instruct-v0:1",
        "mistral.mistral-large-2402-v1:0",
        "mistral.mistral-small-2402-v1:0",
        "mistral.mistral-large-2407-v1:0",
        "mistral.ministral-3-14b-instruct",
        "mistral.ministral-3-8b-instruct",
    ] {
        assert!(model_normalizes_tool_id(id));
    }
}

/// TS: "should return true for region-prefixed mistral models"
#[test]
fn is_mistral_model_region_prefixed() {
    assert!(model_normalizes_tool_id(
        "us.mistral.pixtral-large-2502-v1:0"
    ));
    assert!(model_normalizes_tool_id(
        "eu.mistral.mistral-large-2407-v1:0"
    ));
}

/// TS: "should return false for non-mistral models"
#[test]
fn is_mistral_model_false_for_non_mistral() {
    for id in [
        "anthropic.claude-3-5-sonnet-20241022-v2:0",
        "amazon.nova-pro-v1:0",
        "openai.gpt-4o",
        "meta.llama3-70b-instruct-v1:0",
    ] {
        assert!(!model_normalizes_tool_id(id));
    }
}

/// TS: "should return the original ID when not a Mistral model"
#[test]
fn normalize_tool_call_id_passthrough_when_not_mistral() {
    let id = "tooluse_bpe71yCfRu2b5i-nKGDr5g";
    assert_eq!(normalize_tool_call_id(id, false), id);
}

/// TS: "should hash incompatible IDs deterministically for Mistral models"
#[test]
fn normalize_tool_call_id_hashes_deterministically() {
    let id = "tooluse_bpe71yCfRu2b5i-nKGDr5g";
    assert_eq!(normalize_tool_call_id(id, true), "8eHypBDcw");
    assert_eq!(normalize_tool_call_id(id, true), "8eHypBDcw");
}

/// TS: "should produce 9 alphanumeric characters for incompatible IDs"
#[test]
fn normalize_tool_call_id_special_chars() {
    for (id, expected) in [
        ("tool-use_123ABC456", "hvVDqPNyj"),
        ("___abc123DEF___", "TnzPqldGU"),
        ("abc", "GRuIyUwcV"),
        ("12345", "PceAgWDYe"),
        ("___---___", "5C589HVqG"),
    ] {
        let normalized = normalize_tool_call_id(id, true);
        assert_eq!(normalized, expected);
        assert_eq!(normalized.len(), 9);
        assert!(normalized.bytes().all(|c| c.is_ascii_alphanumeric()));
    }
}

/// TS: "should preserve IDs that are already valid Mistral tool call IDs"
#[test]
fn normalize_tool_call_id_already_alphanumeric() {
    assert_eq!(normalize_tool_call_id("abcdefghi", true), "abcdefghi");
    assert_eq!(normalize_tool_call_id("abc123XYZ", true), "abc123XYZ");
}

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
        ..Default::default()
    }
}

/// TS: "should include totalTokens in raw when provided"
#[test]
fn usage_raw_includes_total_tokens() {
    let raw = bedrock_usage_with_total(100, 50, Some(150), None, None);
    let u = convert_usage(Some(&raw));
    assert_eq!(
        u.raw,
        Some(
            json!({"inputTokens":100,"outputTokens":50,"totalTokens":150})
                .as_object()
                .unwrap()
                .clone()
        )
    );
}

/// TS: "should preserve raw usage data"
#[test]
fn usage_raw_preserved() {
    let raw = bedrock_usage_with_total(100, 50, Some(150), Some(80), Some(60));
    let u = convert_usage(Some(&raw));
    assert_eq!(u.raw, Some(json!({"inputTokens":100,"outputTokens":50,"totalTokens":150,"cacheReadInputTokens":80,"cacheWriteInputTokens":60}).as_object().unwrap().clone()));
}

// ════════════════════════════════════════════════════════════════════════════
// amazon-bedrock-provider.test.ts — provider configuration / auth
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should create a provider instance with default options" — the mocked
/// upstream region loader returns `us-east-1`; pass it explicitly here.
#[tokio::test]
#[serial]
async fn provider_default_region_and_base_url() {
    let fetch = MockFetch::new(vec![Canned::json(&ok_converse_body())]);
    let provider = create_amazon_bedrock(AmazonBedrockProviderSettings {
        api_key: Some(Resolvable::Value(String::new())),
        region: Some("us-east-1".into()),
        access_key_id: Some("ak".into()),
        secret_access_key: Some("sk".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap();
    provider
        .chat("amazon.nova-pro-v1:0")
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();
    let seen = &fetch.seen()[0];
    assert_eq!(
        seen.url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/amazon.nova-pro-v1%3A0/converse"
    );
    assert!(seen.headers["authorization"].starts_with("AWS4-HMAC-SHA256"));
}

/// TS: "should create a provider instance with custom options" — custom region
/// flows into the base URL.
#[tokio::test]
async fn provider_custom_region_base_url() {
    let fetch = MockFetch::new(vec![Canned::json(&ok_converse_body())]);
    let provider = create_amazon_bedrock(AmazonBedrockProviderSettings {
        api_key: Some(Resolvable::Value(String::new())),
        region: Some("eu-west-1".into()),
        access_key_id: Some("ak".into()),
        secret_access_key: Some("sk".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap();
    provider
        .chat("amazon.nova-pro-v1:0")
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();
    let seen = &fetch.seen()[0];
    assert!(
        seen.url
            .starts_with("https://bedrock-runtime.eu-west-1.amazonaws.com/")
    );
}

/// TS: "should create a provider instance with custom options" — baseURL override.
#[tokio::test]
async fn provider_with_base_url_override() {
    let fetch = MockFetch::new(vec![Canned::json(&ok_converse_body())]);
    let provider = create_amazon_bedrock(AmazonBedrockProviderSettings {
        region: Some("us-east-1".into()),
        api_key: Some(Resolvable::Value("tok".into())),
        base_url: Some("https://custom.url/".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap();
    provider
        .chat("amazon.nova-pro-v1:0")
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();
    let seen = &fetch.seen()[0];
    assert!(seen.url.starts_with("https://custom.url/model/"));
}

/// TS: "should use API key when provided in options" — bearer-token auth path.
#[tokio::test]
async fn provider_bearer_token_auth() {
    let fetch = MockFetch::new(vec![Canned::json(&ok_converse_body())]);
    let provider = create_amazon_bedrock(AmazonBedrockProviderSettings {
        region: Some("us-east-1".into()),
        api_key: Some(Resolvable::Value("test-api-key".into())),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap();
    provider
        .chat("amazon.nova-pro-v1:0")
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();
    let seen = &fetch.seen()[0];
    assert_eq!(seen.headers["authorization"], "Bearer test-api-key");
}

/// TS: "should use API key from environment variable" — `AWS_BEARER_TOKEN_BEDROCK`
/// takes precedence over SigV4 environment credentials.
#[tokio::test]
#[serial]
async fn provider_from_env_bearer_token_precedence() {
    clear_bedrock_env();
    unsafe {
        std::env::set_var("AWS_BEARER_TOKEN_BEDROCK", "env-bearer");
        std::env::set_var("AWS_ACCESS_KEY_ID", "should-not-be-used");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "should-not-be-used");
        std::env::set_var("AWS_REGION", "us-west-2");
    }

    let fetch = MockFetch::new(vec![Canned::json(&ok_converse_body())]);
    let provider = create_amazon_bedrock(AmazonBedrockProviderSettings {
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap();
    provider
        .chat("amazon.nova-pro-v1:0")
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();
    let seen = &fetch.seen()[0];
    assert_eq!(seen.headers["authorization"], "Bearer env-bearer");
    assert!(seen.url.contains("us-west-2"));
    clear_bedrock_env();
}

/// TS: "should fall back to SigV4 when no API key provided" — the factory uses
/// SigV4 when `AWS_BEARER_TOKEN_BEDROCK` is absent.
#[tokio::test]
#[serial]
async fn provider_from_env_sigv4_fallback() {
    clear_bedrock_env();
    unsafe {
        std::env::set_var("AWS_ACCESS_KEY_ID", "test-ak");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "test-sk");
        std::env::set_var("AWS_REGION", "eu-west-1");
    }

    let fetch = MockFetch::new(vec![Canned::json(&ok_converse_body())]);
    let provider = create_amazon_bedrock(AmazonBedrockProviderSettings {
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap();
    provider
        .chat("amazon.nova-pro-v1:0")
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();
    let seen = &fetch.seen()[0];
    assert!(seen.headers["authorization"].starts_with("AWS4-HMAC-SHA256 Credential=test-ak/"));
    assert!(seen.headers["authorization"].contains("/eu-west-1/bedrock/"));
    assert!(!seen.headers.contains_key("x-amz-security-token"));
    clear_bedrock_env();
}

/// TS: "should maintain backward compatibility with existing SigV4
/// authentication" — `AWS_SESSION_TOKEN` is loaded from the environment.
#[tokio::test]
#[serial]
async fn provider_from_env_session_token() {
    clear_bedrock_env();
    unsafe {
        std::env::set_var("AWS_ACCESS_KEY_ID", "ak");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "sk");
        std::env::set_var("AWS_REGION", "us-east-1");
        std::env::set_var("AWS_SESSION_TOKEN", "sts-token");
    }

    let fetch = MockFetch::new(vec![Canned::json(&ok_converse_body())]);
    let provider = create_amazon_bedrock(AmazonBedrockProviderSettings {
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap();
    provider
        .chat("amazon.nova-pro-v1:0")
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();
    let seen = &fetch.seen()[0];
    assert_eq!(seen.headers["x-amz-security-token"], "sts-token");
    clear_bedrock_env();
}

/// The `BedrockProvider` exposes its name and can vend language models.
#[test]
fn provider_name_and_language_model() {
    let provider = create_amazon_bedrock(AmazonBedrockProviderSettings::default()).unwrap();
    let model = provider.chat("anthropic.claude-3-5-sonnet-20240620-v1:0");
    assert_eq!(
        model.model_id(),
        "anthropic.claude-3-5-sonnet-20240620-v1:0"
    );
    assert_eq!(model.provider(), "amazon-bedrock");
}

/// TS: "should prioritize options.apiKey over environment variable" — explicit
/// bearer token construction does not consult env vars.
#[tokio::test]
#[serial]
async fn provider_explicit_bearer_over_env() {
    clear_bedrock_env();
    unsafe {
        std::env::set_var("AWS_BEARER_TOKEN_BEDROCK", "env-bearer");
    }
    // Explicit construction with a different token should win over env.
    let fetch = MockFetch::new(vec![Canned::json(&ok_converse_body())]);
    let provider = create_amazon_bedrock(AmazonBedrockProviderSettings {
        region: Some("us-east-1".into()),
        api_key: Some(Resolvable::Value("explicit-tok".into())),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap();
    provider
        .chat("amazon.nova-pro-v1:0")
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();
    let seen = &fetch.seen()[0];
    assert_eq!(seen.headers["authorization"], "Bearer explicit-tok");
    clear_bedrock_env();
}

fn tool_msg(content: Vec<ToolPart>) -> LanguageModelMessage {
    LanguageModelMessage::Tool {
        content,
        provider_options: None,
    }
}

fn assistant_msg(content: Vec<AssistantPart>) -> LanguageModelMessage {
    LanguageModelMessage::Assistant {
        content,
        provider_options: None,
    }
}

/// TS: "should normalize tool call IDs in tool results when isMistral is true"
#[test]
fn convert_mistral_normalize_tool_result_id() {
    let original_id = "tooluse_bpe71yCfRu2b5i-nKGDr5g";
    let prompt = vec![tool_msg(vec![tool_result(
        original_id.to_string(),
        json!({ "type": "text", "value": "The result is 42" }),
    )])];
    let mut opts = default_options(prompt);
    opts.tools = Some(vec![test_tool().into()]);
    let (body, _, _, _) =
        build_request_body_checked("mistral.mistral-large-2402-v1:0", &opts, None).unwrap();
    let messages = &body["messages"];
    assert_eq!(
        messages[0]["content"][0]["toolResult"]["toolUseId"],
        json!("8eHypBDcw")
    );
}

/// TS: "should normalize tool call IDs in tool calls when isMistral is true"
#[test]
fn convert_mistral_normalize_tool_call_id() {
    let original_id = "tooluse_xyz123ABC456-def";
    let prompt = vec![assistant_msg(vec![tool_call(
        original_id.to_string(),
        "test-tool".to_string(),
        json!({ "query": "test" }),
    )])];
    let mut opts = default_options(prompt);
    opts.tools = Some(vec![test_tool().into()]);
    let (body, _, _, _) =
        build_request_body_checked("mistral.mistral-large-2402-v1:0", &opts, None).unwrap();
    let messages = &body["messages"];
    assert_eq!(
        messages[0]["content"][0]["toolUse"]["toolUseId"],
        json!("De61BW1Dz")
    );
}

// ════════════════════════════════════════════════════════════════════════════
// amazon-bedrock-chat-language-model.test.ts — request URL / body / response
// ════════════════════════════════════════════════════════════════════════════

// ── supportedUrls ────────────────────────────────────────────────────────────

/// TS: "should support S3 URLs for image and video parts".
#[test]
fn model_supported_urls_s3() {
    let model = create_amazon_bedrock(AmazonBedrockProviderSettings::default())
        .unwrap()
        .chat("amazon.nova-pro-v1:0");
    let urls = model.supported_urls();
    assert_eq!(urls.0.len(), 2);
    for kind in ["image/*", "video/*"] {
        assert_eq!(urls.0[kind].len(), 1);
        assert_eq!(urls.0[kind][0].as_str(), "^s3://");
    }
}

// ── ARN model IDs containing a slash ─────────────────────────────────────────

/// TS: "should generate text through the encoded Converse route".
///
/// ARN inference-profile IDs contain a `/` which must be percent-encoded in the
/// URL path. The upstream uses `encodeURIComponent(modelId)`.
#[tokio::test]
async fn arn_model_id_encoded_generate_route() {
    let arn = "arn:aws:bedrock:eu-west-1:474668406012:inference-profile/eu.amazon.nova-lite-v1:0";
    let encoded = "arn%3Aaws%3Abedrock%3Aeu-west-1%3A474668406012%3Ainference-profile%2Feu.amazon.nova-lite-v1%3A0";

    let server = mock_converse_json(
        200,
        json!({
            "output": { "message": { "role": "assistant", "content": [{ "text": "Hello!" }] } },
            "stopReason": "end_turn",
            "usage": { "inputTokens": 1, "outputTokens": 1 }
        }),
    );

    let model = create_amazon_bedrock(AmazonBedrockProviderSettings {
        fetch: Some(server.transport()),
        base_url: Some("https://bedrock.test".into()),
        api_key: Some(Resolvable::Value("tok".into())),
        ..Default::default()
    })
    .unwrap()
    .chat(arn);

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed via the encoded route");
    assert_eq!(as_text(&result.content[0]), "Hello!");
    assert_eq!(
        server.seen()[0].url,
        format!("https://bedrock.test/model/{encoded}/converse")
    );
}

/// TS: "should stream text through the encoded Converse route".
#[tokio::test]
async fn arn_model_id_encoded_stream_route() {
    let arn = "arn:aws:bedrock:eu-west-1:474668406012:inference-profile/eu.amazon.nova-lite-v1:0";
    let encoded = "arn%3Aaws%3Abedrock%3Aeu-west-1%3A474668406012%3Ainference-profile%2Feu.amazon.nova-lite-v1%3A0";

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

    let server = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![(
            "content-type".into(),
            "application/vnd.amazon.eventstream".into(),
        )],
        body: body_bytes,
    }]);

    let model = create_amazon_bedrock(AmazonBedrockProviderSettings {
        fetch: Some(server.transport()),
        base_url: Some("https://bedrock.test".into()),
        api_key: Some(Resolvable::Value("tok".into())),
        ..Default::default()
    })
    .unwrap()
    .chat(arn);

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
    assert_eq!(
        server.seen()[0].url,
        format!("https://bedrock.test/model/{encoded}/converse-stream")
    );
}

// ── temperature clamping ─────────────────────────────────────────────────────

/// TS: "should clamp temperature above 1 to 1 and add warning".
#[tokio::test]
async fn temperature_clamped_above_1() {
    let server = mock_converse_json(200, ok_converse_body());

    let model = make_model(&server);
    let mut opts = default_options(test_prompt());
    opts.temperature = Some(1.5);

    let result = model.do_generate(&opts).await.expect("should succeed");

    let body = result.request.unwrap().body.expect("request body");
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
async fn temperature_clamped_below_0() {
    let server = mock_converse_json(200, ok_converse_body());

    let model = make_model(&server);
    let mut opts = default_options(test_prompt());
    opts.temperature = Some(-0.5);

    let result = model.do_generate(&opts).await.expect("should succeed");

    let body = result.request.unwrap().body.expect("request body");
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
    let server = mock_converse_json(200, ok_converse_body());

    let model = make_model(&server);
    let mut opts = default_options(test_prompt());
    opts.temperature = Some(0.7);

    let result = model.do_generate(&opts).await.expect("should succeed");

    let body = result.request.unwrap().body.expect("request body");
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
async fn guardrail_config_in_request_body() {
    let server = mock_converse_json(200, ok_converse_body());

    let model = make_model(&server);
    let mut opts = default_options(test_prompt());
    let po = provider_namespace(
        "bedrock",
        json!({
            "guardrailConfig": {
                "guardrailIdentifier": "-1",
                "guardrailVersion": "1",
                "trace": "enabled"
            }
        }),
    )
    .unwrap();
    opts.provider_options = Some(po);

    let result = model.do_generate(&opts).await.expect("should succeed");
    let body = result.request.unwrap().body.expect("request body");
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
async fn trace_in_provider_metadata() {
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
    let server = mock_converse_json(
        200,
        json!({
            "output": { "message": { "role": "assistant", "content": [{ "text": "Hello, World!" }] } },
            "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
            "stopReason": "stop_sequence",
            "trace": trace
        }),
    );

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
async fn stop_sequence_in_provider_metadata() {
    let server = mock_converse_json(
        200,
        json!({
            "output": { "message": { "role": "assistant", "content": [{ "text": "Hello, World!" }] } },
            "stopReason": "stop_sequence",
            "additionalModelResponseFields": { "delta": { "stop_sequence": "STOP" } },
            "usage": { "inputTokens": 4, "outputTokens": 30, "totalTokens": 34 }
        }),
    );

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
/// accumulates an empty string and yields `input: "{}"`.
#[tokio::test]
async fn stream_tool_call_empty_input() {
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

    let server = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![(
            "content-type".into(),
            "application/vnd.amazon.eventstream".into(),
        )],
        body: body_bytes,
    }]);

    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    let tool_calls: Vec<_> = parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ToolCall(RawToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            }) => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].0, "tool_1");
    assert_eq!(tool_calls[0].1, "updateIssueList");
    assert_eq!(tool_calls[0].2, "{}");
}

// ── omit toolConfig ──────────────────────────────────────────────────────────

/// TS: "should omit toolConfig and filter tool content when conversation has
/// tool calls but no active tools".
///
#[tokio::test]
async fn omit_tool_config_when_no_active_tools() {
    let server = mock_converse_json(200, ok_converse_body());

    let prompt: LanguageModelPrompt = vec![
        LanguageModelMessage::User {
            content: vec![text("What is the weather in Toronto?")],
            provider_options: None,
        },
        LanguageModelMessage::Assistant {
            content: vec![tool_call(
                "tool-call-1".to_string(),
                "weather".to_string(),
                json!({ "city": "Toronto" }),
            )],
            provider_options: None,
        },
        LanguageModelMessage::Tool {
            content: vec![tool_result(
                "tool-call-1".to_string(),
                json!({ "type": "text", "value": "The weather in Toronto is 20°C." }),
            )],
            provider_options: None,
        },
        LanguageModelMessage::User {
            content: vec![text("Now give me a summary.")],
            provider_options: None,
        },
    ];

    let model = make_model(&server);
    let opts = CallOptions {
        tools: Some(vec![]),
        ..default_options(prompt)
    };

    let result = model.do_generate(&opts).await.expect("should succeed");
    let body = result.request.unwrap().body.expect("request body");
    assert!(
        body.get("toolConfig").is_none(),
        "toolConfig should be absent when tools list is empty"
    );
}

/// TS: (same test) the assistant's `toolUse` block should be filtered out of
/// the messages when no tools are active.
#[tokio::test]
async fn filter_tool_content_when_no_active_tools() {
    let server = mock_converse_json(200, ok_converse_body());

    let prompt: LanguageModelPrompt = vec![
        LanguageModelMessage::User {
            content: vec![text("What is the weather in Toronto?")],
            provider_options: None,
        },
        LanguageModelMessage::Assistant {
            content: vec![tool_call(
                "tool-call-1".to_string(),
                "weather".to_string(),
                json!({ "city": "Toronto" }),
            )],
            provider_options: None,
        },
        LanguageModelMessage::Tool {
            content: vec![tool_result(
                "tool-call-1".to_string(),
                json!({ "type": "text", "value": "The weather in Toronto is 20°C." }),
            )],
            provider_options: None,
        },
        LanguageModelMessage::User {
            content: vec![text("Now give me a summary.")],
            provider_options: None,
        },
    ];

    let model = make_model(&server);
    let opts = CallOptions {
        tools: Some(vec![]),
        ..default_options(prompt)
    };

    let result = model.do_generate(&opts).await.expect("should succeed");
    let body = result.request.unwrap().body.expect("request body");
    assert_eq!(
        body["messages"],
        json!([{ "role": "user", "content": [{ "text": "What is the weather in Toronto?" }, { "text": "Now give me a summary." }] }])
    );
}

// ── doGenerate: tool call with empty input (non-streaming) ───────────────────

/// TS: "should support tool calls with empty input (no arguments)" (generate).
#[tokio::test]
async fn generate_tool_call_empty_input() {
    let server = mock_converse_json(
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
    );

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.content.len(), 1);
    let (id, name, input) = as_tool_call(&result.content[0]);
    assert_eq!(id, "tool_1");
    assert_eq!(name, "updateIssueList");
    assert_eq!(input, "{}");
}

// ── doGenerate: basic text + finish reason (sanity, already covered but
//    exercised here against the remaining-test helper set) ────────────────────

/// TS: "should pass the model and the messages" — the request body carries the
/// model id implicitly via the URL path and the messages array.
#[tokio::test]
async fn request_body_messages_shape() {
    let server = mock_converse_json(200, ok_converse_body());

    let prompt: LanguageModelPrompt = vec![
        LanguageModelMessage::System {
            content: "System Prompt".into(),
            provider_options: None,
        },
        LanguageModelMessage::User {
            content: vec![text("Hello")],
            provider_options: None,
        },
    ];

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(prompt))
        .await
        .expect("ok");
    let body = result.request.unwrap().body.expect("request body");
    assert_eq!(body["system"], json!([{ "text": "System Prompt" }]));
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"][0]["text"], "Hello");
}

/// TS: "should extract finish reason" — `guardrail_intervened` → ContentFilter.
#[tokio::test]
async fn finish_reason_guardrail_intervened() {
    let server = mock_converse_json(
        200,
        json!({
            "output": { "message": { "role": "assistant", "content": [{ "text": "" }] } },
            "stopReason": "guardrail_intervened",
            "usage": { "inputTokens": 4, "outputTokens": 1 }
        }),
    );

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

/// TS: "should handle throttlingException error" (stream).
#[tokio::test]
async fn stream_throttling_error() {
    let body_bytes = aimux_providers::bedrock::event_stream::encode_messages(&[(
        "exception",
        "throttlingException",
        r#"{"message":"Throttling Error"}"#,
    )]);
    let server = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![(
            "content-type".into(),
            "application/vnd.amazon.eventstream".into(),
        )],
        body: body_bytes,
    }]);
    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .unwrap();
    let parts = collect_stream(result).await;
    assert!(parts.iter().any(
        |part| matches!(part, StreamPart::Error { error } if error.status_code() == Some(429))
    ));
    assert!(parts.iter().any(|part| matches!(part, StreamPart::Finish { finish_reason, .. } if finish_reason.unified == FinishReasonUnified::Error)));
}

/// TS: "should handle validationException error" (stream).
#[tokio::test]
async fn stream_validation_error() {
    let body_bytes = aimux_providers::bedrock::event_stream::encode_messages(&[(
        "exception",
        "validationException",
        r#"{"message":"Validation Error"}"#,
    )]);
    let server = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![(
            "content-type".into(),
            "application/vnd.amazon.eventstream".into(),
        )],
        body: body_bytes,
    }]);
    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .unwrap();
    let parts = collect_stream(result).await;
    assert!(parts.iter().any(
        |part| matches!(part, StreamPart::Error { error } if error.status_code() == Some(400))
    ));
    assert!(parts.iter().any(|part| matches!(part, StreamPart::Finish { finish_reason, .. } if finish_reason.unified == FinishReasonUnified::Error)));
}

// ── doStream: error handling ─────────────────────────────────────────────────

// ── doStream: request body parity ────────────────────────────────────────────

/// TS: "should return the request body" (stream) — the stream result carries
/// the request body for debugging.
#[tokio::test]
async fn stream_request_body_available() {
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

    let server = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![(
            "content-type".into(),
            "application/vnd.amazon.eventstream".into(),
        )],
        body: body_bytes,
    }]);

    let model = make_model(&server);
    let mut opts = default_options(test_prompt());
    opts.max_output_tokens = Some(256);

    let result = model.do_stream(&opts).await.expect("should succeed");
    let body = result.request.unwrap().body.expect("stream request body");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["inferenceConfig"]["maxTokens"], 256);
}
