//! Bedrock reasoning / thinking tests, translated from the TypeScript suite.
//!
//! Translation sources (under `reference/ai/packages/amazon-bedrock/src/`):
//! - `amazon-bedrock-chat-language-model.test.ts`
//!     - `describe('reasoning')` under `doGenerate` — "should extract reasoning
//!       and text response" (fixture `amazon-bedrock-reasoning.json`).
//!     - `describe('reasoning')` under `doStream` — "should stream reasoning and
//!       text parts" (fixture `amazon-bedrock-reasoning.chunks.txt`).
//!     - "should stream reasoning text deltas" (inline chunks).
//!     - "should extract reasoning text with signature" / "…without signature".
//!     - "should preserve empty text blocks between reasoning blocks".
//!     - "should extract redacted reasoning".
//!     - "should handle multiple reasoning blocks".
//!     - "should transform reasoningConfig to thinking in stream requests".
//!     - "merges user additionalModelRequestFields with derived thinking (stream)".
//! - `convert-to-amazon-bedrock-chat-messages.test.ts` — the reasoning
//!   `it(...)` cases (prompt-side replay of signed/unsigned reasoning).
//!
//! Tests that require a data model the Rust crate does not expose are marked
//! `#[ignore]` with an inline reason:
//! - **Redacted reasoning replay** — `ContentPart::Reasoning` has no
//!   `redactedData` field and `convert_prompt_to_bedrock` has no
//!   `redactedReasoning` branch.
//! - **Foreign-provider reasoning replay** — `ContentPart::Reasoning.signature`
//!   is provider-agnostic; the converter cannot distinguish an `anthropic`
//!   signature (which must be dropped) from a `bedrock` one (which is kept).

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
use aimux_core::result::{GenerateContent, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::FinishReasonUnified;

use aimux_providers::bedrock::convert::convert_prompt_to_bedrock;
use aimux_providers::bedrock::event_stream;
use aimux_providers::bedrock::{BedrockAuth, BedrockConfig, BedrockModel};

// ── Constants ────────────────────────────────────────────────────────────────

/// Model id used by the TS reasoning tests (matches the snapshot `modelId`).
const MODEL_ID: &str = "anthropic.claude-3-haiku-20240307-v1:0";

/// Signature on the `amazon-bedrock-reasoning.json` fixture's reasoning block.
const REASONING_SIGNATURE: &str = "EvYBCkgICxABGAIqQA9E9VC377UnbjdfXCw4RwQaaIXsqocZKzI3WwWtXT/wBjzAkBOfIiVkP/mhvsPGb5oQW/2j+E1tZcNFoXIb26YSDBTSbDrbHwXJoAl3/hoMA/Qo4rIHvObP/yBrIjAT+l69mh9k7evFO+w+ransSD5YGq4hYrieOyJH3k/zHv8nBJwiz3Nlrb3jNFi2Ib0qXJH7BIK0hR85yYIoccReoJUpeLEMxeWbsO6f0EpOSMSi3YDLd1U9NAw7Itjeay1fEBPHwvawt2M4e/rn52SsuEz3p2DfOZm+N3bnL63rg0Cb9lFcp3k5kuU1h6hcGAE=";

/// Signature carried by the streaming reasoning fixture's final delta.
const STREAM_SIGNATURE: &str = "Ep0CCkgICxABGAIqQOWPB6/PmA5SW9jC6FvaNq3E+U9ev4FMWcFWuAho+VGLCtazKc5WDjQ5i0MuxsY0o5pKDSVWVKii8KJDusXH4eASDK7jyzuk8iij7fJNihoMxHO9haYzt48R36HVIjCb/EmIFrJLIXShqN6DN//T6vZBtO9qj1QhNWJa3CGm8VZoq80S2/Ok4U0aVaIDiZcqggHC2b8BHuv8BHZrmsR0wjU1ynansBGMdfjnG+iIv8R5lPpRmYGhSVwNybwP3aQZ6o8Dr48Rau8TJfdsArW+r7bvL7bPs4f5nnlp2vG7WkMzWwABHK3fdM44zZ1GZQaWyECNWR2GfY6dXiklo94vgpFTPuZ97mfiN3LY6uYyBwL8RkDaGAE=";

// ── Helpers ──────────────────────────────────────────────────────────────────

/// TS `TEST_PROMPT`: a system message + a user "Hello" message.
fn test_prompt() -> LanguageModelPrompt {
    vec![
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
    ]
}

fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

fn make_model(server: &MockServer) -> BedrockModel {
    BedrockModel::new(
        MODEL_ID.to_string(),
        BedrockConfig {
            base_url: server.uri(),
            auth: BedrockAuth::BearerToken("test-token".to_string()),
            retry_config: aimux_provider_utils::RetryConfig::default(),
            api_key_source: None,
        },
    )
}

/// Wrap a `bedrock`-keyed provider-options value for `CallOptions.provider_options`.
fn bedrock_provider_options(value: Value) -> Option<HashMap<String, Value>> {
    let mut map = HashMap::new();
    map.insert("bedrock".to_string(), value);
    Some(map)
}

/// Mock the non-streaming `/converse` endpoint with a JSON body (HTTP 200).
async fn mock_converse(server: &MockServer, body: Value) {
    let model_path = format!("/model/{MODEL_ID}/converse");
    Mock::given(method("POST"))
        .and(path(model_path.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Encode a sequence of `(message-type, event-type, payload)` events into the
/// AWS event stream binary format used by `/converse-stream`.
fn encode_events(events: &[(&str, &str, Value)]) -> Vec<u8> {
    let mut buf = Vec::new();
    for (mt, et, payload) in events {
        buf.extend_from_slice(&event_stream::encode_message(mt, et, &payload.to_string()));
    }
    buf
}

/// Mock the streaming `/converse-stream` endpoint with the given events.
async fn mock_stream(server: &MockServer, events: &[(&str, &str, Value)]) {
    let body_bytes = encode_events(events);
    let model_path = format!("/model/{MODEL_ID}/converse-stream");
    Mock::given(method("POST"))
        .and(path(model_path.as_str()))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.amazon.eventstream")
                .set_body_raw(body_bytes, "application/vnd.amazon.eventstream"),
        )
        .mount(server)
        .await;
}

async fn collect_stream(result: StreamResult) -> Vec<StreamPart> {
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

fn as_text(item: &GenerateContent) -> &str {
    match item {
        GenerateContent::Text { text, .. } => text,
        _ => panic!("expected Text content, got {item:?}"),
    }
}

fn as_reasoning(item: &GenerateContent) -> (&str, &Option<Value>) {
    match item {
        GenerateContent::Reasoning {
            text,
            provider_metadata,
        } => (text.as_str(), provider_metadata),
        _ => panic!("expected Reasoning content, got {item:?}"),
    }
}

fn reasoning_deltas(parts: &[StreamPart]) -> Vec<String> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ReasoningDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect()
}

/// Reasoning deltas that carry actual text, i.e. excluding the zero-length
/// deltas that only transport a `signature` / `redactedData` in
/// `provider_metadata` (upstream emits those with `delta: ''`).
fn reasoning_text_deltas(parts: &[StreamPart]) -> Vec<String> {
    reasoning_deltas(parts)
        .into_iter()
        .filter(|d| !d.is_empty())
        .collect()
}

/// The `signature` values carried on reasoning-block ends, in emission order.
/// The signature arrives on a trailing text-less delta and is attached to the
/// concluding `ReasoningEnd`.
fn reasoning_signatures(parts: &[StreamPart]) -> Vec<String> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::ReasoningEnd {
                provider_metadata: Some(pm),
                ..
            } => pm
                .get("amazonBedrock")
                .and_then(|b| b.get("signature"))
                .and_then(|s| s.as_str())
                .map(std::string::ToString::to_string),
            _ => None,
        })
        .collect()
}

fn text_deltas(parts: &[StreamPart]) -> Vec<String> {
    parts
        .iter()
        .filter_map(|p| match p {
            StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect()
}

fn has_reasoning_start(parts: &[StreamPart]) -> bool {
    parts
        .iter()
        .any(|p| matches!(p, StreamPart::ReasoningStart { .. }))
}

fn has_reasoning_end(parts: &[StreamPart]) -> bool {
    parts
        .iter()
        .any(|p| matches!(p, StreamPart::ReasoningEnd { .. }))
}

// ── convert helpers (prompt-side) ────────────────────────────────────────────

fn msg(role: Role, content: Vec<ContentPart>) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role,
        content,
        provider_options: None,
    }
}

fn user(content: Vec<ContentPart>) -> LanguageModelPromptMessage {
    msg(Role::User, content)
}

fn assistant(content: Vec<ContentPart>) -> LanguageModelPromptMessage {
    msg(Role::Assistant, content)
}

fn tool_msg(content: Vec<ContentPart>) -> LanguageModelPromptMessage {
    msg(Role::Tool, content)
}

fn reasoning(text: &str, signature: Option<&str>) -> ContentPart {
    ContentPart::Reasoning {
        text: text.to_string(),
        signature: signature.map(std::string::ToString::to_string),
        provider_options: None,
    }
}

fn tool_call(id: &str, name: &str, input: Value) -> ContentPart {
    ContentPart::tool_call(id.to_string(), name.to_string(), input)
}

fn tool_result(id: &str, output: Value) -> ContentPart {
    ContentPart::tool_result(id.to_string(), output)
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — reasoning response extraction (TDD red until implemented)
// ════════════════════════════════════════════════════════════════════════════

/// TS: `doGenerate > reasoning > should extract reasoning and text response`.
///
/// Uses the `amazon-bedrock-reasoning.json` fixture: an assistant message with
/// a `reasoningContent.reasoningText` block (text + signature) followed by a
/// `text` block. The model should emit `Reasoning` then `Text`.
#[tokio::test]
async fn bedrock_generate_reasoning_response() {
    let server = MockServer::start().await;
    mock_converse(
        &server,
        json!({
            "metrics": { "latencyMs": 2202 },
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [
                        {
                            "reasoningContent": {
                                "reasoningText": {
                                    "signature": REASONING_SIGNATURE,
                                    "text": "Let me count the r's in \"strawberry\":\n\ns-t-r-a-w-b-e-r-r-y\n\nThere are 3 r's."
                                }
                            }
                        },
                        {
                            "text": "There are **3** r's in \"strawberry\":\n\n1. st**r**awbe**r****r**y"
                        }
                    ]
                }
            },
            "stopReason": "end_turn",
            "usage": {
                "cacheReadInputTokenCount": 0,
                "cacheReadInputTokens": 0,
                "cacheWriteInputTokenCount": 0,
                "cacheWriteInputTokens": 0,
                "inputTokens": 51,
                "outputTokens": 78,
                "serverToolUsage": {},
                "totalTokens": 129
            }
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    // content[0] — reasoning with signature under both `amazonBedrock` and
    // `bedrock` provider-metadata keys.
    assert_eq!(result.content.len(), 2, "expected reasoning + text content");
    let (rtext, rmeta) = as_reasoning(&result.content[0]);
    assert_eq!(
        rtext,
        "Let me count the r's in \"strawberry\":\n\ns-t-r-a-w-b-e-r-r-y\n\nThere are 3 r's."
    );
    let rmeta = rmeta
        .as_ref()
        .expect("reasoning provider_metadata should be set");
    assert_eq!(rmeta["amazonBedrock"]["signature"], REASONING_SIGNATURE);
    assert_eq!(rmeta["bedrock"]["signature"], REASONING_SIGNATURE);

    // content[1] — text.
    assert_eq!(
        as_text(&result.content[1]),
        "There are **3** r's in \"strawberry\":\n\n1. st**r**awbe**r****r**y"
    );

    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("end_turn"));
    assert_eq!(result.usage.input_tokens.total, Some(51));
    assert_eq!(result.usage.output_tokens.total, Some(78));
}

/// TS: "should extract reasoning text with signature".
#[tokio::test]
async fn bedrock_generate_reasoning_text_with_signature() {
    let server = MockServer::start().await;
    mock_converse(
        &server,
        json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [
                        {
                            "reasoningContent": {
                                "reasoningText": {
                                    "text": "I need to think about this problem carefully...",
                                    "signature": "abc123signature"
                                }
                            }
                        },
                        { "type": "text", "text": "The answer is 42." }
                    ]
                }
            },
            "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
            "stopReason": "stop_sequence"
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 2);
    let (rtext, rmeta) = as_reasoning(&result.content[0]);
    assert_eq!(rtext, "I need to think about this problem carefully...");
    let rmeta = rmeta.as_ref().expect("provider_metadata should be set");
    assert_eq!(rmeta["amazonBedrock"]["signature"], "abc123signature");
    assert_eq!(rmeta["bedrock"]["signature"], "abc123signature");

    assert_eq!(as_text(&result.content[1]), "The answer is 42.");
}

/// TS: "should preserve empty text blocks between reasoning blocks".
///
/// The empty `{ text: "" }` block sitting between two reasoning blocks must be
/// preserved as a `Text` content item (not dropped).
#[tokio::test]
async fn bedrock_generate_reasoning_preserve_empty_text_between_reasoning() {
    let server = MockServer::start().await;
    mock_converse(
        &server,
        json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [
                        { "reasoningContent": { "reasoningText": { "text": "thinking...", "signature": "sig-1" } } },
                        { "text": "" },
                        { "reasoningContent": { "reasoningText": { "text": "more thinking...", "signature": "sig-2" } } },
                        { "text": "The answer is 42." }
                    ]
                }
            },
            "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
            "stopReason": "stop_sequence"
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 4);
    let (t0, m0) = as_reasoning(&result.content[0]);
    assert_eq!(t0, "thinking...");
    assert_eq!(m0.as_ref().unwrap()["bedrock"]["signature"], "sig-1");

    assert_eq!(as_text(&result.content[1]), "");

    let (t2, m2) = as_reasoning(&result.content[2]);
    assert_eq!(t2, "more thinking...");
    assert_eq!(m2.as_ref().unwrap()["bedrock"]["signature"], "sig-2");

    assert_eq!(as_text(&result.content[3]), "The answer is 42.");
}

/// TS: "should extract reasoning text without signature".
///
/// A `reasoningText` block with no `signature` yields a `Reasoning` content item
/// with no provider metadata.
#[tokio::test]
async fn bedrock_generate_reasoning_text_without_signature() {
    let server = MockServer::start().await;
    mock_converse(
        &server,
        json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [
                        { "reasoningContent": { "reasoningText": { "text": "I need to think about this problem carefully..." } } },
                        { "type": "text", "text": "The answer is 42." }
                    ]
                }
            },
            "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
            "stopReason": "stop_sequence"
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 2);
    let (rtext, rmeta) = as_reasoning(&result.content[0]);
    assert_eq!(rtext, "I need to think about this problem carefully...");
    assert!(
        rmeta.is_none(),
        "reasoning without signature should have no provider_metadata"
    );
    assert_eq!(as_text(&result.content[1]), "The answer is 42.");
}

/// TS: "should extract redacted reasoning".
///
/// A `redactedReasoning` block yields a `Reasoning` content item with empty text
/// and `redactedData` under both `amazonBedrock` and `bedrock` provider metadata.
#[tokio::test]
async fn bedrock_generate_reasoning_redacted() {
    let server = MockServer::start().await;
    mock_converse(
        &server,
        json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [
                        { "reasoningContent": { "redactedReasoning": { "data": "redacted-reasoning-data" } } },
                        { "type": "text", "text": "The answer is 42." }
                    ]
                }
            },
            "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
            "stopReason": "stop_sequence"
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 2);
    let (rtext, rmeta) = as_reasoning(&result.content[0]);
    assert_eq!(rtext, "");
    let rmeta = rmeta.as_ref().expect("provider_metadata should be set");
    assert_eq!(
        rmeta["amazonBedrock"]["redactedData"],
        "redacted-reasoning-data"
    );
    assert_eq!(rmeta["bedrock"]["redactedData"], "redacted-reasoning-data");
    assert_eq!(as_text(&result.content[1]), "The answer is 42.");
}

/// TS: "should handle multiple reasoning blocks".
///
/// A signed `reasoningText` block followed by a `redactedReasoning` block then
/// text — three content items in order.
#[tokio::test]
async fn bedrock_generate_reasoning_multiple_blocks() {
    let server = MockServer::start().await;
    mock_converse(
        &server,
        json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [
                        { "reasoningContent": { "reasoningText": { "text": "First reasoning block", "signature": "sig1" } } },
                        { "reasoningContent": { "redactedReasoning": { "data": "redacted-data" } } },
                        { "type": "text", "text": "The answer is 42." }
                    ]
                }
            },
            "usage": { "inputTokens": 4, "outputTokens": 34, "totalTokens": 38 },
            "stopReason": "stop_sequence"
        }),
    )
    .await;

    let model = make_model(&server);
    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("do_generate should succeed");

    assert_eq!(result.content.len(), 3);

    let (t0, m0) = as_reasoning(&result.content[0]);
    assert_eq!(t0, "First reasoning block");
    assert_eq!(m0.as_ref().unwrap()["bedrock"]["signature"], "sig1");

    let (t1, m1) = as_reasoning(&result.content[1]);
    assert_eq!(t1, "");
    assert_eq!(
        m1.as_ref().unwrap()["bedrock"]["redactedData"],
        "redacted-data"
    );

    assert_eq!(as_text(&result.content[2]), "The answer is 42.");
}

// ════════════════════════════════════════════════════════════════════════════
// doStream — reasoning streaming (TDD red until implemented)
// ════════════════════════════════════════════════════════════════════════════

/// TS: `doStream > reasoning > should stream reasoning and text parts`.
///
/// Uses the `amazon-bedrock-reasoning.chunks.txt` fixture: a reasoning block
/// (index 0) streamed as `reasoningContent.text` deltas + a final
/// `reasoningContent.signature` delta, then a text block (index 1).
#[tokio::test]
async fn bedrock_stream_reasoning_and_text() {
    let server = MockServer::start().await;

    let events: Vec<(&str, &str, Value)> = vec![
        ("event", "messageStart", json!({ "role": "assistant" })),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": "Let me count the r" } } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": "'s in \"" } } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": "strawberry\":" } } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": "\n\ns-t-r-a" } } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": "-w-b-e-r" } } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": "-r-y\n\nr" } } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": " appears at positions " } } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": "3, 8" } } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": ", and 9.\n\nSo there" } } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": " are 3 r's." } } }),
        ),
        // Empty-text delta — TS skips emitting a reasoning-delta for this.
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": "" } } }),
        ),
        // Signature-only delta — emitted as a reasoning-delta with delta=""
        // carrying providerMetadata.signature, matching TS.
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "signature": STREAM_SIGNATURE } } }),
        ),
        (
            "event",
            "contentBlockStop",
            json!({ "contentBlockIndex": 0 }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 1, "delta": { "text": "There" } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 1, "delta": { "text": " are **3** r's in \"" } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 1, "delta": { "text": "strawberry\":" } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 1, "delta": { "text": "\n\n1" } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 1, "delta": { "text": ". st" } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 1, "delta": { "text": "**" } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 1, "delta": { "text": "r**awbe" } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 1, "delta": { "text": "**r****" } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 1, "delta": { "text": "r**y" } }),
        ),
        (
            "event",
            "contentBlockStop",
            json!({ "contentBlockIndex": 1 }),
        ),
        (
            "event",
            "messageStop",
            json!({ "additionalModelResponseFields": { "delta": { "stop_sequence": null } }, "stopReason": "end_turn" }),
        ),
        (
            "event",
            "metadata",
            json!({ "metrics": { "latencyMs": 2281 }, "usage": { "inputTokens": 51, "outputTokens": 94, "serverToolUsage": {}, "totalTokens": 145 } }),
        ),
    ];
    mock_stream(&server, &events).await;

    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    // Reasoning block (id "0"): start, non-empty text deltas, end.
    assert!(has_reasoning_start(&parts), "expected a ReasoningStart");
    assert_eq!(
        reasoning_text_deltas(&parts),
        vec![
            "Let me count the r",
            "'s in \"",
            "strawberry\":",
            "\n\ns-t-r-a",
            "-w-b-e-r",
            "-r-y\n\nr",
            " appears at positions ",
            "3, 8",
            ", and 9.\n\nSo there",
            " are 3 r's.",
        ]
    );
    assert!(has_reasoning_end(&parts), "expected a ReasoningEnd");
    // The signature on the final (text-less) reasoning delta is attached to
    // the concluding ReasoningEnd via provider_metadata, so extended-thinking
    // multi-turn can echo it back (#113).
    let reasoning_end_meta = parts.iter().find_map(|p| match p {
        StreamPart::ReasoningEnd {
            provider_metadata, ..
        } => provider_metadata.clone(),
        _ => None,
    });
    let reasoning_end_meta = reasoning_end_meta.expect("ReasoningEnd with provider_metadata");
    assert_eq!(
        reasoning_end_meta["bedrock"]["signature"].as_str(),
        Some(STREAM_SIGNATURE)
    );
    // Dual-key shape matches the non-streaming path.
    assert_eq!(
        reasoning_end_meta["amazonBedrock"]["signature"].as_str(),
        Some(STREAM_SIGNATURE)
    );

    // Text block (id "1").
    assert_eq!(
        text_deltas(&parts),
        vec![
            "There",
            " are **3** r's in \"",
            "strawberry\":",
            "\n\n1",
            ". st",
            "**",
            "r**awbe",
            "**r****",
            "r**y",
        ]
    );

    // Finish.
    let finish = parts.iter().find_map(|p| match p {
        StreamPart::Finish { finish_reason, .. } => Some(finish_reason.clone()),
        _ => None,
    });
    let finish = finish.expect("expected a Finish part");
    assert_eq!(finish.unified, FinishReasonUnified::Stop);
    assert_eq!(finish.raw.as_deref(), Some("end_turn"));
}

/// TS: "should stream reasoning text deltas".
///
/// Inline chunks: two reasoning text deltas, a signature-only delta, then a
/// text delta and `messageStop`.
#[tokio::test]
async fn bedrock_stream_reasoning_text_deltas() {
    let server = MockServer::start().await;
    let events: Vec<(&str, &str, Value)> = vec![
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": "I am thinking" } } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": " about this problem..." } } }),
        ),
        // Signature-only delta — emitted as a reasoning-delta with delta=""
        // carrying providerMetadata.signature, matching TS.
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 0, "delta": { "reasoningContent": { "signature": "abc123signature" } } }),
        ),
        (
            "event",
            "contentBlockDelta",
            json!({ "contentBlockIndex": 1, "delta": { "text": "Based on my reasoning, the answer is 42." } }),
        ),
        (
            "event",
            "messageStop",
            json!({ "stopReason": "stop_sequence" }),
        ),
    ];
    mock_stream(&server, &events).await;

    let model = make_model(&server);
    let result = model
        .do_stream(&default_options(test_prompt()))
        .await
        .expect("do_stream should succeed");
    let parts = collect_stream(result).await;

    assert!(has_reasoning_start(&parts), "expected a ReasoningStart");
    assert_eq!(
        reasoning_text_deltas(&parts),
        vec!["I am thinking", " about this problem..."]
    );
    assert_eq!(
        reasoning_signatures(&parts),
        vec!["abc123signature"],
        "the thinking-block signature must reach the caller"
    );
    assert!(has_reasoning_end(&parts), "expected a ReasoningEnd");

    assert_eq!(
        text_deltas(&parts),
        vec!["Based on my reasoning, the answer is 42."]
    );

    let finish = parts.iter().find_map(|p| match p {
        StreamPart::Finish { finish_reason, .. } => Some(finish_reason.clone()),
        _ => None,
    });
    let finish = finish.expect("expected a Finish part");
    assert_eq!(finish.unified, FinishReasonUnified::Stop);
    assert_eq!(finish.raw.as_deref(), Some("stop_sequence"));
}

// ════════════════════════════════════════════════════════════════════════════
// doStream — request-side reasoningConfig → thinking (TDD red until implemented)
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should transform reasoningConfig to thinking in stream requests".
///
/// `providerOptions.bedrock.reasoningConfig = { type: 'enabled', budgetTokens:
/// 2000 }` plus `maxOutputTokens: 100` must land in the request body as
/// `additionalModelRequestFields.thinking = { type: 'enabled', budget_tokens:
/// 2000 }` and `inferenceConfig.maxTokens = 2100` (100 + 2000), with no
/// top-level `reasoningConfig`.
#[tokio::test]
async fn bedrock_stream_transform_reasoning_config_to_thinking() {
    let server = MockServer::start().await;
    mock_stream(
        &server,
        &[(
            "event",
            "messageStop",
            json!({ "stopReason": "stop_sequence" }),
        )],
    )
    .await;

    let opts = CallOptions {
        max_output_tokens: Some(100),
        provider_options: bedrock_provider_options(json!({
            "reasoningConfig": { "type": "enabled", "budgetTokens": 2000 }
        })),
        ..default_options(test_prompt())
    };

    let model = make_model(&server);
    let result = model
        .do_stream(&opts)
        .await
        .expect("do_stream should succeed");
    let body = result.request_body.expect("request body should be present");

    // Thinking config derived from reasoningConfig.
    assert_eq!(
        body["additionalModelRequestFields"]["thinking"],
        json!({ "type": "enabled", "budget_tokens": 2000 })
    );
    // maxTokens is bumped by budgetTokens (100 + 2000).
    assert_eq!(body["inferenceConfig"]["maxTokens"], 2100);
    // reasoningConfig must not leak to the top level.
    assert!(
        body.get("reasoningConfig").is_none(),
        "reasoningConfig should not appear at the top level"
    );
}

/// TS: "merges user additionalModelRequestFields with derived thinking (stream)".
///
/// User-supplied `additionalModelRequestFields` (`foo`, `custom`) must be
/// merged with the derived `thinking` field, and `reasoningConfig` must not
/// appear at the top level.
#[tokio::test]
async fn bedrock_stream_merge_additional_model_request_fields_with_thinking() {
    let server = MockServer::start().await;
    mock_stream(
        &server,
        &[(
            "event",
            "messageStop",
            json!({ "stopReason": "stop_sequence" }),
        )],
    )
    .await;

    let opts = CallOptions {
        provider_options: bedrock_provider_options(json!({
            "reasoningConfig": { "type": "enabled", "budgetTokens": 500 },
            "additionalModelRequestFields": { "foo": "bar", "custom": 42 }
        })),
        ..default_options(test_prompt())
    };

    let model = make_model(&server);
    let result = model
        .do_stream(&opts)
        .await
        .expect("do_stream should succeed");
    let body = result.request_body.expect("request body should be present");

    assert!(
        body.get("reasoningConfig").is_none(),
        "reasoningConfig should not appear at the top level"
    );
    assert_eq!(body["additionalModelRequestFields"]["foo"], "bar");
    assert_eq!(body["additionalModelRequestFields"]["custom"], 42);
    assert_eq!(
        body["additionalModelRequestFields"]["thinking"],
        json!({ "type": "enabled", "budget_tokens": 500 })
    );
}

// ════════════════════════════════════════════════════════════════════════════
// convert_prompt_to_bedrock — prompt-side reasoning replay
// (green for signed/unsigned reasoning; #[ignore] where the data model cannot
//  express redacted / foreign-provider reasoning)
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should properly convert reasoning content type".
///
/// An assistant reasoning part carrying a `bedrock` signature is replayed as a
/// `reasoningContent.reasoningText { text, signature }` block.
#[test]
fn convert_reasoning_content_type() {
    let prompt = vec![
        user(vec![ContentPart::text("Explain your reasoning")]),
        assistant(vec![reasoning(
            "This is my step-by-step reasoning process",
            Some("test-signature"),
        )]),
    ];
    let (system, messages) = convert_prompt_to_bedrock(&prompt);

    assert_eq!(Value::Array(system), json!([]));
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "Explain your reasoning" }] },
            {
                "role": "assistant",
                "content": [
                    {
                        "reasoningContent": {
                            "reasoningText": {
                                "text": "This is my step-by-step reasoning process",
                                "signature": "test-signature"
                            }
                        }
                    }
                ]
            }
        ])
    );
}

/// TS: "should properly convert redacted-reasoning content type".
///
/// SKIPPED: `ContentPart::Reasoning` has no `redactedData` field and
/// `convert_prompt_to_bedrock` has no `redactedReasoning` branch, so redacted
/// reasoning cannot be replayed on the Bedrock side. The test body documents
/// the intended behaviour (a `reasoningContent.redactedReasoning { data }`
/// block) for when the data model gains support.
#[test]
#[ignore = "redacted reasoning is not modelled on ContentPart::Reasoning / convert"]
fn convert_reasoning_redacted_content_type() {
    let prompt = vec![
        user(vec![ContentPart::text("Explain your reasoning")]),
        assistant(vec![ContentPart::Reasoning {
            text: String::new(),
            signature: None,
            provider_options: Some(
                json!({ "bedrock": { "redactedData": "Redacted reasoning information" } }),
            ),
        }]),
    ];
    let (system, messages) = convert_prompt_to_bedrock(&prompt);

    assert_eq!(Value::Array(system), json!([]));
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "Explain your reasoning" }] },
            {
                "role": "assistant",
                "content": [
                    {
                        "reasoningContent": {
                            "redactedReasoning": { "data": "Redacted reasoning information" }
                        }
                    }
                ]
            }
        ])
    );
}

/// TS: "should omit assistant message reasoning parts signed by a foreign
/// provider".
///
/// SKIPPED: `ContentPart::Reasoning.signature` is provider-agnostic, so the
/// converter cannot distinguish a foreign-provider signature (which must be
/// dropped) from a Bedrock one (which is kept). The intended behaviour is that
/// the reasoning part is omitted and only the trailing text remains.
#[test]
#[ignore = "ContentPart::Reasoning cannot express provider distinction (foreign vs bedrock signature)"]
fn convert_reasoning_foreign_provider_omitted() {
    let prompt = vec![
        user(vec![ContentPart::text("Explain your reasoning")]),
        assistant(vec![
            // In TS this carries `providerOptions.anthropic.signature` and is
            // dropped because it is not a Bedrock signature.
            reasoning(
                "Anthropic-signed reasoning replayed to Bedrock",
                Some("anthropic-signature"),
            ),
            ContentPart::text("final answer"),
        ]),
    ];
    let (system, messages) = convert_prompt_to_bedrock(&prompt);

    assert_eq!(Value::Array(system), json!([]));
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "Explain your reasoning" }] },
            { "role": "assistant", "content": [{ "text": "final answer" }] }
        ])
    );
}

/// TS: "should preserve assistant message reasoning parts with amazonBedrock
/// providerOptions".
///
/// A reasoning part with a Bedrock signature is preserved. (The Rust data model
/// does not distinguish `amazonBedrock` from `bedrock` provider options — the
/// signature is a direct field — but the replay behaviour matches.)
#[test]
fn convert_reasoning_amazon_bedrock_provider_options_preserved() {
    let prompt = vec![
        user(vec![ContentPart::text("Explain your reasoning")]),
        assistant(vec![
            reasoning(
                "Bedrock-signed reasoning round-tripped to Bedrock",
                Some("bedrock-signature"),
            ),
            ContentPart::text("final answer"),
        ]),
    ];
    let (system, messages) = convert_prompt_to_bedrock(&prompt);

    assert_eq!(Value::Array(system), json!([]));
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "Explain your reasoning" }] },
            {
                "role": "assistant",
                "content": [
                    {
                        "reasoningContent": {
                            "reasoningText": {
                                "text": "Bedrock-signed reasoning round-tripped to Bedrock",
                                "signature": "bedrock-signature"
                            }
                        }
                    },
                    { "text": "final answer" }
                ]
            }
        ])
    );
}

/// TS: "should not trim reasoning text when a signature is present".
///
/// Trailing whitespace on reasoning text must be preserved verbatim (only the
/// last *text* part of the last block is trimmed, never reasoning).
#[test]
fn convert_reasoning_no_trim_with_signature() {
    let prompt = vec![
        user(vec![ContentPart::text("Explain your reasoning")]),
        assistant(vec![reasoning(
            "This is my reasoning with trailing space    ",
            Some("test-signature"),
        )]),
    ];
    let (system, messages) = convert_prompt_to_bedrock(&prompt);

    assert_eq!(Value::Array(system), json!([]));
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "Explain your reasoning" }] },
            {
                "role": "assistant",
                "content": [
                    {
                        "reasoningContent": {
                            "reasoningText": {
                                "text": "This is my reasoning with trailing space    ",
                                "signature": "test-signature"
                            }
                        }
                    }
                ]
            }
        ])
    );
}

/// TS: "should omit reasoning content without signature".
///
/// Unsigned reasoning is dropped; the trailing text is kept.
#[test]
fn convert_reasoning_omit_without_signature() {
    let prompt = vec![
        user(vec![ContentPart::text("Explain your reasoning")]),
        assistant(vec![
            reasoning("This is my reasoning with trailing space    ", None),
            ContentPart::text("final answer"),
        ]),
    ];
    let (system, messages) = convert_prompt_to_bedrock(&prompt);

    assert_eq!(Value::Array(system), json!([]));
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "Explain your reasoning" }] },
            { "role": "assistant", "content": [{ "text": "final answer" }] }
        ])
    );
}

/// TS: "should omit multiple reasoning parts without signatures".
#[test]
fn convert_reasoning_omit_multiple_without_signatures() {
    let prompt = vec![
        user(vec![ContentPart::text("Explain your reasoning")]),
        assistant(vec![
            reasoning("First reasoning with trailing space    ", None),
            reasoning("Second reasoning with trailing space    ", None),
            ContentPart::text("final answer"),
        ]),
    ];
    let (system, messages) = convert_prompt_to_bedrock(&prompt);

    assert_eq!(Value::Array(system), json!([]));
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "Explain your reasoning" }] },
            { "role": "assistant", "content": [{ "text": "final answer" }] }
        ])
    );
}

/// TS: "should omit unsigned reasoning while preserving tool calls in
/// multi-turn tool use".
#[test]
fn convert_reasoning_omit_unsigned_preserve_tool_calls() {
    let prompt = vec![
        user(vec![ContentPart::text("What is the weather?")]),
        assistant(vec![
            reasoning("I should call the weather tool.", None),
            tool_call("call-1", "getWeather", json!({ "city": "SF" })),
        ]),
        tool_msg(vec![tool_result(
            "call-1",
            json!({ "type": "text", "value": "Sunny, 72F" }),
        )]),
    ];
    let (system, messages) = convert_prompt_to_bedrock(&prompt);

    assert_eq!(Value::Array(system), json!([]));
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "What is the weather?" }] },
            {
                "role": "assistant",
                "content": [
                    {
                        "toolUse": {
                            "input": { "city": "SF" },
                            "name": "getWeather",
                            "toolUseId": "call-1"
                        }
                    }
                ]
            },
            {
                "role": "user",
                "content": [
                    {
                        "toolResult": {
                            "content": [{ "text": "Sunny, 72F" }],
                            "toolUseId": "call-1"
                        }
                    }
                ]
            }
        ])
    );
}

/// TS: "should preserve reasoning text with signature in multi-turn tool use".
#[test]
fn convert_reasoning_preserve_signed_in_multi_turn_tool_use() {
    let prompt = vec![
        user(vec![ContentPart::text("What is the weather?")]),
        assistant(vec![
            reasoning("Let me check the weather API.\n", Some("sig-abc123")),
            tool_call("call-1", "getWeather", json!({ "city": "SF" })),
        ]),
        tool_msg(vec![tool_result(
            "call-1",
            json!({ "type": "text", "value": "Sunny, 72F" }),
        )]),
        assistant(vec![
            reasoning("The weather is sunny and warm.\n", Some("sig-def456")),
            ContentPart::text("It is sunny and 72F in SF."),
        ]),
    ];
    let (system, messages) = convert_prompt_to_bedrock(&prompt);

    assert_eq!(Value::Array(system), json!([]));
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "What is the weather?" }] },
            {
                "role": "assistant",
                "content": [
                    {
                        "reasoningContent": {
                            "reasoningText": {
                                "text": "Let me check the weather API.\n",
                                "signature": "sig-abc123"
                            }
                        }
                    },
                    {
                        "toolUse": {
                            "input": { "city": "SF" },
                            "name": "getWeather",
                            "toolUseId": "call-1"
                        }
                    }
                ]
            },
            {
                "role": "user",
                "content": [
                    {
                        "toolResult": {
                            "content": [{ "text": "Sunny, 72F" }],
                            "toolUseId": "call-1"
                        }
                    }
                ]
            },
            {
                "role": "assistant",
                "content": [
                    {
                        "reasoningContent": {
                            "reasoningText": {
                                "text": "The weather is sunny and warm.\n",
                                "signature": "sig-def456"
                            }
                        }
                    },
                    { "text": "It is sunny and 72F in SF." }
                ]
            }
        ])
    );
}

/// TS: "should handle a mix of text and reasoning content types".
#[test]
fn convert_reasoning_mix_text_and_reasoning() {
    let prompt = vec![
        user(vec![ContentPart::text("Explain your reasoning")]),
        assistant(vec![
            ContentPart::text("My answer is 42."),
            reasoning(
                "I calculated this by analyzing the meaning of life",
                Some("reasoning-process"),
            ),
        ]),
    ];
    let (system, messages) = convert_prompt_to_bedrock(&prompt);

    assert_eq!(Value::Array(system), json!([]));
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "Explain your reasoning" }] },
            {
                "role": "assistant",
                "content": [
                    { "text": "My answer is 42." },
                    {
                        "reasoningContent": {
                            "reasoningText": {
                                "text": "I calculated this by analyzing the meaning of life",
                                "signature": "reasoning-process"
                            }
                        }
                    }
                ]
            }
        ])
    );
}
