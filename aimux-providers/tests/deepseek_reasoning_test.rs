//! DeepSeek `reasoning_content` tests, translated from the Vercel AI SDK
//! TypeScript suite.
//!
//! Translation sources:
//! - `packages/deepseek/src/chat/deepseek-chat-language-model.test.ts`
//!   - `describe('doGenerate') › describe('reasoning')` — request body with
//!     `thinking` + response extraction from the reasoning fixture.
//!   - `describe('doGenerate') › describe('top-level reasoning')` — mapping of
//!     top-level `reasoning` and `providerOptions.deepseek` to the
//!     `thinking` / `reasoning_effort` request fields.
//!   - `describe('doStream') › describe('reasoning')` — streaming the
//!     reasoning fixture.
//! - `packages/openai-compatible/src/chat/openai-compatible-chat-language-model.test.ts`
//!   - DeepSeek is a thin OpenAI-compatible wrapper, so the
//!     `reasoning_content` response-extraction tests (doGenerate + doStream)
//!     apply directly. These cover the core behaviour gap: the shared OpenAI
//!     response parser currently ignores `reasoning_content` (serde skips the
//!     unknown field), so reasoning text is not surfaced as a
//!     `GenerateContent::Reasoning` part / `StreamPart::Reasoning*` event.
//!
//! HTTP is mocked with `wiremock` (a real loopback server), replacing the TS
//! `createTestServer`. Each test starts its own `MockServer` so parallel
//! `#[tokio::test]` runs do not collide.
//!
//! # TDD status
//!
//! DeepSeek delegates to the shared [`OpenAIModel`], whose response parser does
//! not read `reasoning_content` / `reasoning` and whose request builder does
//! not emit a `thinking` field or do the DeepSeek-specific `reasoning` →
//! `reasoning_effort` remapping (xhigh→max, minimal→low with compatibility
//! warnings). Provider options keyed under `deepseek` are also not consulted
//! (the shared builder only reads the `openai` key).
//!
//! Tests that assert these unimplemented behaviours are written as TDD red
//! lights: they compile cleanly against the existing types
//! (`GenerateContent::Reasoning`, `StreamPart::Reasoning*`,
//! `Warning::Compatibility`) but fail at runtime until the feature lands.
//! No `#[ignore]` is used — every test compiles against types that exist today.
//!
//! # Mock URL note
//!
//! The real DeepSeek base URL is `https://api.deepseek.com/v1`, so production
//! requests hit `/v1/chat/completions`. In these tests `with_base_url` is
//! overridden with the mock server's root URI (no `/v1` suffix), so the
//! resulting request path is `/chat/completions` — matching the convention in
//! `openai_compatible_test.rs`.

use std::collections::HashMap;

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{ReasoningEffort, Warning};

use aimux_providers::{DeepSeekConfig, DeepSeekProvider};

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers (mirrors openai_compatible_test.rs / anthropic_model_test.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

/// `CallOptions` with only `prompt` set (everything else default/None).
fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// Build a DeepSeek provider whose base URL points at the mock server.
fn make_provider(server: &MockServer) -> DeepSeekProvider {
    let config = DeepSeekConfig::new("test-api-key").with_base_url(server.uri());
    DeepSeekProvider::new(config)
}

/// Wrap a value as `providerOptions.deepseek.<value>`.
///
/// The TS tests pass `providerOptions: { deepseek: { ... } }`. The Rust
/// `CallOptions.provider_options` is keyed by provider name.
fn deepseek_opts(value: Value) -> Option<HashMap<String, Value>> {
    let mut m = HashMap::new();
    m.insert("deepseek".to_string(), value);
    Some(m)
}

/// Mount a JSON chat-completion response on `/chat/completions`.
async fn mock_json(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Mount an SSE stream response on `/chat/completions`.
async fn mock_sse(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(server)
        .await;
}

/// Build a single SSE `data: <json>\n\n` event string.
fn sse_event(json_str: &str) -> String {
    format!("data: {}\n\n", json_str)
}

/// Concatenate SSE events and append the `[DONE]` sentinel.
fn sse_body(events: &[&str]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
    body.push_str("data: [DONE]\n\n");
    body
}

/// Collect every `StreamPart` from a `StreamResult` into a `Vec`.
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

/// Extract text deltas from a list of stream parts.
fn text_deltas(parts: &[StreamPart]) -> Vec<String> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect()
}

/// Extract reasoning deltas from a list of stream parts.
fn reasoning_deltas(parts: &[StreamPart]) -> Vec<String> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ReasoningDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect()
}

/// True if the warnings contain a `Compatibility` warning for `feature`.
fn has_reasoning_compatibility_warning(warnings: &[Warning]) -> bool {
    warnings.iter().any(|w| {
        matches!(
            w,
            Warning::Compatibility { feature, .. } if feature == "reasoning"
        )
    })
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — reasoning_content response extraction
//
// Translated from
// `openai-compatible-chat-language-model.test.ts` doGenerate reasoning tests
// and `deepseek-chat-language-model.test.ts` doGenerate › reasoning › "should
// extract text content".
//
// TS behaviour: `reasoning = message.reasoning_content ?? message.reasoning`
// (JS nullish coalescing — an empty string is NOT nullish, so `""` wins over
// `reasoning`); a non-empty reasoning string is pushed as a `{ type:
// "reasoning", text }` content part after the text part.
//
// Rust status: the shared OpenAI `MessageResponse` has no `reasoning_content`
// / `reasoning` field, so serde silently drops them — no `Reasoning` content
// part is ever produced. The extraction tests below are TDD red lights.
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should prefer reasoning_content over reasoning field when both are
/// provided" (openai-compatible test, line ~320).
///
/// RED LIGHT: Rust ignores `reasoning_content`, so only the `Text` part is
/// produced — the `Reasoning` assertion fails.
#[tokio::test]
async fn should_prefer_reasoning_content_over_reasoning_field_when_both_provided() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "deepseek-reasoner",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello, World!",
                    "reasoning_content": "This is from reasoning_content",
                    "reasoning": "This is from reasoning field"
                },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(
        result.content.len(),
        2,
        "expected [Text, Reasoning] — reasoning_content should be extracted"
    );
    match &result.content[0] {
        GenerateContent::Text { text, .. } => assert_eq!(text, "Hello, World!"),
        other => panic!("expected Text, got {:?}", other),
    }
    match &result.content[1] {
        GenerateContent::Reasoning { text, .. } => {
            assert_eq!(text, "This is from reasoning_content")
        }
        other => panic!("expected Reasoning from reasoning_content, got {:?}", other),
    }
}

/// TS: "should extract reasoning from reasoning field when reasoning_content
/// is not provided" (openai-compatible test, line ~300).
///
/// NOTE: the TS `prepareJsonResponse` helper defaults `reasoning_content` to
/// `""` (empty string). Because JS `??` does not treat `""` as nullish,
/// `"" ?? reasoning` resolves to `""`, whose `.length` is 0, so NO reasoning
/// part is emitted — the TS snapshot is text-only. This test replicates that
/// exact edge case. It currently passes in Rust because reasoning is ignored
/// entirely; after `reasoning_content` extraction is implemented (mirroring
/// the `??` + length-check) it must still pass.
#[tokio::test]
async fn should_extract_reasoning_from_reasoning_field_when_reasoning_content_not_provided() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "deepseek-reasoner",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello, World!",
                    "reasoning_content": "",
                    "reasoning": "This is the reasoning from the reasoning field"
                },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    // Empty-string reasoning_content suppresses the reasoning-field fallback
    // (TS `??` edge case), so only the text part is expected.
    assert_eq!(
        result.content.len(),
        1,
        "empty reasoning_content should not yield a reasoning part"
    );
    match &result.content[0] {
        GenerateContent::Text { text, .. } => assert_eq!(text, "Hello, World!"),
        other => panic!("expected Text only, got {:?}", other),
    }
}

/// TS: deepseek doGenerate › reasoning › "should extract text content"
/// (deepseek-chat-language-model.test.ts, line ~118), using the
/// `deepseek-reasoning` fixture.
///
/// The fixture carries both `content` and a long `reasoning_content`. The TS
/// snapshot surfaces the reasoning as a `{ type: "reasoning" }` part. The
/// `reasoning_content` text below is abbreviated from the fixture for
/// readability; the assertion still verifies extraction.
///
/// RED LIGHT: Rust ignores `reasoning_content`, so only `Text` is produced.
#[tokio::test]
async fn should_extract_reasoning_content_and_text_from_deepseek_reasoning_fixture() {
    let server = MockServer::start().await;
    mock_json(
        &server,
        json!({
            "id": "945bb10c-9bf3-47ff-a2a2-43bbe9705c72",
            "object": "chat.completion",
            "created": 1764660903,
            "model": "deepseek-reasoner",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "The word \"strawberry\" contains three instances of the letter \"r\": one after the \"t\" and two before the \"y\".",
                    "reasoning_content": "We are asked: \"How many 'r's are in the word 'strawberry'?\" We need to count the occurrences of the letter 'r'."
                },
                "logprobs": null,
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 18,
                "completion_tokens": 345,
                "total_tokens": 363,
                "completion_tokens_details": { "reasoning_tokens": 315 }
            },
            "system_fingerprint": "fp_eaab8d114b_prod0820_fp8_kvcache"
        }),
    )
    .await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(
        result.content.len(),
        2,
        "expected [Text, Reasoning] from the reasoning fixture"
    );
    match &result.content[0] {
        GenerateContent::Text { text, .. } => assert!(
            text.contains("strawberry"),
            "expected the fixture text, got: {}",
            text
        ),
        other => panic!("expected Text, got {:?}", other),
    }
    match &result.content[1] {
        GenerateContent::Reasoning { text, .. } => assert!(
            text.contains("How many"),
            "expected the fixture reasoning_content, got: {}",
            text
        ),
        other => panic!("expected Reasoning, got {:?}", other),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — request body: thinking & reasoning_effort
//
// Translated from `deepseek-chat-language-model.test.ts` doGenerate ›
// "reasoning" ("should send correct request body") and "top-level reasoning"
// describe blocks (lines ~82–311).
//
// TS behaviour: DeepSeek maps top-level `reasoning` and
// `providerOptions.deepseek` to a `thinking` field (`{ type: "enabled" |
// "disabled" | "adaptive" }`) and a `reasoning_effort` field, with
// xhigh→max / minimal→low remapping that emits `Compatibility` warnings.
//
// Rust status: the shared OpenAI request builder emits `reasoning_effort`
// straight from `options.reasoning` (Display string) and never emits
// `thinking`; it does not remap xhigh/minimal or emit compatibility warnings;
// and it only reads provider options under the `openai` key, so
// `providerOptions.deepseek.*` is ignored. The remapping / thinking tests
// below are TDD red lights.
// ════════════════════════════════════════════════════════════════════════════

/// TS: doGenerate › reasoning › "should send correct request body"
/// (line ~82) — `providerOptions.deepseek.thinking = { type: "enabled" }`
/// must land in the request body.
///
/// RED LIGHT: `thinking` is never emitted and `deepseek` provider options are
/// not consulted.
#[tokio::test]
async fn should_send_thinking_enabled_in_request_body_via_provider_options() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.provider_options = deepseek_opts(json!({ "thinking": { "type": "enabled" } }));

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["model"], json!("deepseek-reasoner"));
    assert_eq!(body["thinking"], json!({ "type": "enabled" }));
}

/// TS: "should map top-level reasoning to thinking enabled" (line ~132).
///
/// RED LIGHT: `thinking` is not emitted (only `reasoning_effort: "high"` is).
#[tokio::test]
async fn should_map_top_level_reasoning_high_to_thinking_enabled_and_reasoning_effort_high() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.reasoning = Some(ReasoningEffort::High);

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["thinking"], json!({ "type": "enabled" }));
    assert_eq!(body["reasoning_effort"], json!("high"));
}

/// TS: "should map top-level reasoning none to thinking disabled" (line ~145).
///
/// RED LIGHT: `thinking` is not emitted and `reasoning_effort` is set to
/// `"none"` instead of being omitted.
#[tokio::test]
async fn should_map_top_level_reasoning_none_to_thinking_disabled() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.reasoning = Some(ReasoningEffort::None);

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["thinking"], json!({ "type": "disabled" }));
    assert!(
        body.get("reasoning_effort").is_none(),
        "reasoning 'none' should map to thinking disabled, not set reasoning_effort"
    );
}

/// TS: "should map top-level reasoning xhigh to reasoning_effort max" (line ~158).
///
/// RED LIGHT: Rust emits `reasoning_effort: "xhigh"` (not `"max"`) and produces
/// no compatibility warning.
#[tokio::test]
async fn should_map_top_level_reasoning_xhigh_to_reasoning_effort_max_with_warning() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.reasoning = Some(ReasoningEffort::Xhigh);

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["reasoning_effort"], json!("max"));
    let warning = result
        .warnings
        .iter()
        .find(|w| matches!(w, Warning::Compatibility { feature, .. } if feature == "reasoning"));
    match warning {
        Some(Warning::Compatibility { details, .. }) => assert_eq!(
            details.as_deref(),
            Some(
                "reasoning \"xhigh\" is not directly supported by this model. mapped to effort \"max\"."
            )
        ),
        other => panic!(
            "expected a reasoning compatibility warning, got {:?}",
            other
        ),
    }
}

/// TS: "should map top-level reasoning low to reasoning_effort low without a
/// compatibility warning" (line ~175).
///
/// PASSES: Rust maps `Low` → `reasoning_effort: "low"` with no compatibility
/// warning.
#[tokio::test]
async fn should_map_top_level_reasoning_low_to_reasoning_effort_low_without_warning() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.reasoning = Some(ReasoningEffort::Low);

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["reasoning_effort"], json!("low"));
    assert!(
        !has_reasoning_compatibility_warning(&result.warnings),
        "low should not emit a compatibility warning"
    );
}

/// TS: "should map top-level reasoning medium to reasoning_effort medium"
/// (line ~192).
///
/// PASSES: Rust maps `Medium` → `reasoning_effort: "medium"`.
#[tokio::test]
async fn should_map_top_level_reasoning_medium_to_reasoning_effort_medium() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.reasoning = Some(ReasoningEffort::Medium);

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["reasoning_effort"], json!("medium"));
}

/// TS: "should map top-level reasoning minimal to reasoning_effort low with
/// compatibility warning" (line ~203).
///
/// RED LIGHT: Rust emits `reasoning_effort: "minimal"` (not `"low"`) and
/// produces no compatibility warning.
#[tokio::test]
async fn should_map_top_level_reasoning_minimal_to_reasoning_effort_low_with_warning() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.reasoning = Some(ReasoningEffort::Minimal);

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["reasoning_effort"], json!("low"));
    let warning = result
        .warnings
        .iter()
        .find(|w| matches!(w, Warning::Compatibility { feature, .. } if feature == "reasoning"));
    match warning {
        Some(Warning::Compatibility { details, .. }) => assert_eq!(
            details.as_deref(),
            Some(
                "reasoning \"minimal\" is not directly supported by this model. mapped to effort \"low\"."
            )
        ),
        other => panic!(
            "expected a reasoning compatibility warning, got {:?}",
            other
        ),
    }
}

/// TS: "should pass providerOptions reasoningEffort %s through to the API"
/// (line ~220, `it.each(['low', 'medium', 'xhigh'])`).
///
/// RED LIGHT: `providerOptions.deepseek.reasoningEffort` is ignored (the shared
/// builder only reads the `openai` key), so `reasoning_effort` is absent.
#[tokio::test]
async fn should_pass_provider_options_reasoning_effort_through_to_api() {
    for effort in ["low", "medium", "xhigh"] {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let provider = make_provider(&server);
        let model = provider.model("deepseek-reasoner");

        let mut options = default_options(test_prompt());
        options.provider_options = deepseek_opts(json!({ "reasoningEffort": effort }));

        let result = model.do_generate(&options).await.expect("should succeed");
        let body = result.request_body.expect("body");

        assert_eq!(
            body["reasoning_effort"],
            json!(effort),
            "reasoning_effort should pass through for effort={}",
            effort
        );
    }
}

/// TS: "should pass providerOptions thinking.type=adaptive through to the API"
/// (line ~238).
///
/// RED LIGHT: `thinking` is never emitted and `deepseek` provider options are
/// not consulted.
#[tokio::test]
async fn should_pass_provider_options_thinking_adaptive_through_to_api() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.provider_options = deepseek_opts(json!({ "thinking": { "type": "adaptive" } }));

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["thinking"], json!({ "type": "adaptive" }));
}

/// TS: "should pass providerOptions reasoningEffort" (line ~253) — only
/// `reasoningEffort: "max"` is set, so `thinking` must be undefined.
///
/// RED LIGHT: `reasoning_effort` is absent (deepseek options ignored).
#[tokio::test]
async fn should_pass_provider_options_reasoning_effort_max() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.provider_options = deepseek_opts(json!({ "reasoningEffort": "max" }));

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["reasoning_effort"], json!("max"));
    // When only reasoningEffort is set without thinking, thinking should be
    // undefined and rely on the API default (enabled).
    assert!(body.get("thinking").is_none());
}

/// TS: "should prefer providerOptions thinking over top-level reasoning"
/// (line ~270).
///
/// RED LIGHT: `thinking` is not emitted.
#[tokio::test]
async fn should_prefer_provider_options_thinking_over_top_level_reasoning() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.reasoning = Some(ReasoningEffort::None);
    options.provider_options = deepseek_opts(json!({ "thinking": { "type": "enabled" } }));

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["thinking"], json!({ "type": "enabled" }));
}

/// TS: "should prefer providerOptions reasoningEffort over top-level reasoning"
/// (line ~286).
///
/// RED LIGHT: the `deepseek` provider option is ignored, so `reasoning_effort`
/// falls back to the top-level `"high"`.
#[tokio::test]
async fn should_prefer_provider_options_reasoning_effort_over_top_level_reasoning() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let mut options = default_options(test_prompt());
    options.reasoning = Some(ReasoningEffort::High);
    options.provider_options = deepseek_opts(json!({ "reasoningEffort": "max" }));

    let result = model.do_generate(&options).await.expect("should succeed");
    let body = result.request_body.expect("body");

    assert_eq!(body["reasoning_effort"], json!("max"));
}

/// TS: "should not set thinking when reasoning is not specified" (line ~302).
///
/// PASSES: `thinking` is never emitted by the shared builder.
#[tokio::test]
async fn should_not_set_thinking_when_reasoning_not_specified() {
    let server = MockServer::start().await;
    mock_json(&server, text_completion_body()).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");
    let body = result.request_body.expect("body");

    assert!(
        body.get("thinking").is_none(),
        "thinking should be absent when no reasoning is specified"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// doStream — reasoning_content streaming
//
// Translated from `openai-compatible-chat-language-model.test.ts` doStream
// reasoning tests (lines ~1856–2149) and `deepseek-chat-language-model.test.ts`
// doStream › reasoning › "should stream reasoning" (line ~764).
//
// TS behaviour: `reasoningContent = delta.reasoning_content ?? delta.reasoning`;
// when truthy it emits `reasoning-start` / `reasoning-delta` / `reasoning-end`
// before the text deltas.
//
// Rust status: the shared `Delta` type has no `reasoning_content` / `reasoning`
// field, so serde drops them — no `StreamPart::Reasoning*` is ever emitted.
// These tests are TDD red lights.
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should stream reasoning content before text deltas" (line ~1856).
///
/// RED LIGHT: `reasoning_content` deltas are dropped, so no
/// `ReasoningStart` / `ReasoningDelta` / `ReasoningEnd` parts are emitted.
#[tokio::test]
async fn should_stream_reasoning_content_before_text_deltas() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1711357598,"model":"deepseek-reasoner","system_fingerprint":"fp","choices":[{"index":0,"delta":{"role":"assistant","content":"","reasoning_content":"Let me think"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1711357598,"model":"deepseek-reasoner","system_fingerprint":"fp","choices":[{"index":0,"delta":{"content":"","reasoning_content":" about this"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1711357598,"model":"deepseek-reasoner","system_fingerprint":"fp","choices":[{"index":0,"delta":{"content":"Here's"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1711357598,"model":"deepseek-reasoner","system_fingerprint":"fp","choices":[{"index":0,"delta":{"content":" my response"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1729171479,"model":"deepseek-reasoner","system_fingerprint":"fp","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":18,"completion_tokens":439}}"#,
        ),
    ]);
    mock_sse(&server, body).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    assert_eq!(
        reasoning_deltas(&parts),
        vec!["Let me think".to_string(), " about this".to_string()],
        "reasoning_content deltas should be emitted as reasoning-delta parts"
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningStart { .. })),
        "expected a ReasoningStart part"
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningEnd { .. })),
        "expected a ReasoningEnd part"
    );
    let deltas = text_deltas(&parts);
    assert!(deltas.iter().any(|d| d == "Here's"), "missing text delta");
    assert!(
        deltas.iter().any(|d| d == " my response"),
        "missing text delta"
    );
}

/// TS: "should stream reasoning from reasoning field when reasoning_content is
/// not provided" (line ~1959).
///
/// RED LIGHT: the `reasoning` delta field is dropped, so no reasoning parts.
#[tokio::test]
async fn should_stream_reasoning_from_reasoning_field_when_reasoning_content_not_provided() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1711357598,"model":"deepseek-reasoner","system_fingerprint":"fp","choices":[{"index":0,"delta":{"role":"assistant","content":"","reasoning":"Let me consider"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1711357598,"model":"deepseek-reasoner","system_fingerprint":"fp","choices":[{"index":0,"delta":{"content":"","reasoning":" this carefully"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1711357598,"model":"deepseek-reasoner","system_fingerprint":"fp","choices":[{"index":0,"delta":{"content":"My answer is"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1711357598,"model":"deepseek-reasoner","system_fingerprint":"fp","choices":[{"index":0,"delta":{"content":" correct"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1729171479,"model":"deepseek-reasoner","system_fingerprint":"fp","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":18,"completion_tokens":439}}"#,
        ),
    ]);
    mock_sse(&server, body).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    assert_eq!(
        reasoning_deltas(&parts),
        vec!["Let me consider".to_string(), " this carefully".to_string()],
        "reasoning field deltas should be emitted as reasoning-delta parts"
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningStart { .. })),
        "expected a ReasoningStart part"
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningEnd { .. })),
        "expected a ReasoningEnd part"
    );
}

/// TS: "should prefer reasoning_content over reasoning field in streaming when
/// both are provided" (line ~2062).
///
/// RED LIGHT: both fields are dropped, so no reasoning parts are emitted (and
/// certainly not from `reasoning_content`).
#[tokio::test]
async fn should_prefer_reasoning_content_over_reasoning_field_in_streaming() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1711357598,"model":"deepseek-reasoner","system_fingerprint":"fp","choices":[{"index":0,"delta":{"role":"assistant","content":"","reasoning_content":"From reasoning_content","reasoning":"From reasoning"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1711357598,"model":"deepseek-reasoner","system_fingerprint":"fp","choices":[{"index":0,"delta":{"content":"Final response"},"finish_reason":null}]}"#,
        ),
        &sse_event(
            r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1729171479,"model":"deepseek-reasoner","system_fingerprint":"fp","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":18,"completion_tokens":439}}"#,
        ),
    ]);
    mock_sse(&server, body).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    assert_eq!(
        reasoning_deltas(&parts),
        vec!["From reasoning_content".to_string()],
        "reasoning_content should be preferred over the reasoning field"
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningStart { .. })),
        "expected a ReasoningStart part"
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningEnd { .. })),
        "expected a ReasoningEnd part"
    );
    assert!(
        text_deltas(&parts).iter().any(|d| d == "Final response"),
        "missing text delta"
    );
}

/// TS: deepseek doStream › reasoning › "should stream reasoning" (line ~764),
/// using the `deepseek-reasoning` chunks fixture.
///
/// The fixture streams `reasoning_content` deltas ("We", " need", ...) before
/// the text content. The chunks below are an abbreviated inline subset of the
/// fixture; the assertion still verifies reasoning is streamed.
///
/// RED LIGHT: `reasoning_content` deltas are dropped, so no reasoning parts.
#[tokio::test]
async fn should_stream_reasoning_from_deepseek_reasoning_fixture() {
    let server = MockServer::start().await;
    let body = sse_body(&[
        &sse_event(
            r#"{"id":"cac7192e","object":"chat.completion.chunk","created":1764661832,"model":"deepseek-reasoner","system_fingerprint":"fp_eaab8d114b_prod0820_fp8_kvcache","choices":[{"index":0,"delta":{"role":"assistant","content":null,"reasoning_content":"We"},"logprobs":null,"finish_reason":null}],"usage":null}"#,
        ),
        &sse_event(
            r#"{"id":"cac7192e","object":"chat.completion.chunk","created":1764661832,"model":"deepseek-reasoner","system_fingerprint":"fp_eaab8d114b_prod0820_fp8_kvcache","choices":[{"index":0,"delta":{"content":null,"reasoning_content":" need"},"logprobs":null,"finish_reason":null}],"usage":null}"#,
        ),
        &sse_event(
            r#"{"id":"cac7192e","object":"chat.completion.chunk","created":1764661832,"model":"deepseek-reasoner","system_fingerprint":"fp_eaab8d114b_prod0820_fp8_kvcache","choices":[{"index":0,"delta":{"content":"The word","reasoning_content":null},"logprobs":null,"finish_reason":null}],"usage":null}"#,
        ),
        &sse_event(
            r#"{"id":"cac7192e","object":"chat.completion.chunk","created":1764661832,"model":"deepseek-reasoner","system_fingerprint":"fp_eaab8d114b_prod0820_fp8_kvcache","choices":[{"index":0,"delta":{},"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":18,"completion_tokens":345,"total_tokens":363}}"#,
        ),
    ]);
    mock_sse(&server, body).await;

    let provider = make_provider(&server);
    let model = provider.model("deepseek-reasoner");

    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    assert_eq!(
        reasoning_deltas(&parts),
        vec!["We".to_string(), " need".to_string()],
        "fixture reasoning_content deltas should be streamed"
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningStart { .. })),
        "expected a ReasoningStart part"
    );
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningEnd { .. })),
        "expected a ReasoningEnd part"
    );
    assert!(
        text_deltas(&parts).iter().any(|d| d == "The word"),
        "missing text delta"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Local fixtures
// ─────────────────────────────────────────────────────────────────────────────

/// A minimal non-streaming chat-completion body returning "Hello, World!".
/// Used by the request-body tests (the response content is irrelevant there).
fn text_completion_body() -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "deepseek-reasoner",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Hello, World!" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
    })
}
