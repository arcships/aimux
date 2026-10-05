// Panic convert wrappers are #[deprecated]; these tests still use them.
#![allow(deprecated)]
//! Regression tests for two issues reported against aimux 0.1.1 when driving
//! OpenAI-compatible thinking models (e.g. DeepSeek `deepseek-v4-flash`) in
//! multi-turn tool-call conversations:
//!
//! 1. **`tool` role with `ContentPart[]` was rejected.** `ModelPrompt`
//!    deserialization of a `tool_result` part built with the legacy `output`
//!    field (the shape emitted by the Vercel AI SDK and the 0.1.0 TypeScript
//!    bindings) failed with "data did not match any variant of untagged enum
//!    ModelPrompt". `ContentPart::ToolResult.result` now accepts `output` as a
//!    deserialization alias (`#[serde(alias = "output")]`).
//!
//! 2. **`reasoning` ContentPart was dropped on the request side.** Thinking
//!    models require prior assistant `reasoning_content` to be replayed on
//!    later turns, including tool-call turns. The OpenAI message converter
//!    now lifts `ContentPart::Reasoning` parts to a top-level
//!    `reasoning_content` string on assistant messages (mirroring the Vercel
//!    AI SDK `openai-compatible` assistant conversion).
//!
//! These are pure-function tests against
//! `convert_prompt_to_openai_messages` (no network).

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::convert_to_language_model_prompt;
use aimux_core::message::{ModelMessage, ModelPrompt, Role};
use aimux_providers::openai::convert::convert_prompt_to_openai_messages;
use serde_json::json;

// ── Issue 1: tool role ContentPart[] / `output` alias ───────────────────────

/// A `tool` message whose `tool_result` part uses the legacy `output` field
/// (Vercel AI SDK / 0.1.0 TS bindings shape) round-trips through `ModelPrompt`.
#[test]
fn tool_message_with_tool_result_output_field_deserializes() {
    let json_str = r#"[
        {"role":"tool","content":[{"type":"tool_result","tool_call_id":"tc1","output":"ok"}]}
    ]"#;
    let prompt: ModelPrompt =
        serde_json::from_str(json_str).expect("output-field tool_result must deserialize");
    let msgs = match prompt {
        ModelPrompt::Messages(m) => m,
        other => panic!("expected Messages, got {other:?}"),
    };
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].role, Role::Tool);
    // The alias only affects deserialization; the in-memory field is `result`.
    match &msgs[0].content {
        aimux_core::message::MessageContent::Parts(parts) => {
            assert_eq!(parts.len(), 1);
            match &parts[0] {
                ContentPart::ToolResult {
                    tool_call_id,
                    result,
                    ..
                } => {
                    assert_eq!(tool_call_id, "tc1");
                    assert_eq!(result, &json!("ok"));
                }
                other => panic!("expected ToolResult, got {other:?}"),
            }
        }
        other => panic!("expected Parts, got {other:?}"),
    }
}

/// The current `result` field name still deserializes (no regression).
#[test]
fn tool_message_with_tool_result_result_field_deserializes() {
    let json_str = r#"[
        {"role":"tool","content":[{"type":"tool_result","tool_call_id":"tc1","result":"ok"}]}
    ]"#;
    let prompt: ModelPrompt =
        serde_json::from_str(json_str).expect("result-field tool_result must deserialize");
    assert!(matches!(prompt, ModelPrompt::Messages(_)));
}

/// A full tool round-trip (assistant tool_call → tool result → …) built from
/// raw JSON reproducing the user's failing case now converts to OpenAI
/// messages with `tool_call_id` on the tool message.
#[test]
fn full_tool_round_trip_from_json_converts_with_tool_call_id() {
    let json_str = r#"[
        {"role":"user","content":"write a file"},
        {"role":"assistant","content":[
            {"type":"tool_call","tool_call_id":"tc1","tool_name":"write_file","input":{"path":"/tmp/test.txt"}}
        ]},
        {"role":"tool","content":[
            {"type":"tool_result","tool_call_id":"tc1","output":"Successfully wrote to /tmp/test.txt"}
        ]}
    ]"#;
    let prompt: ModelPrompt = serde_json::from_str(json_str).expect("must deserialize");
    let msgs = match prompt {
        ModelPrompt::Messages(m) => m,
        _ => unreachable!(),
    };
    // Convert to provider-facing prompt then to OpenAI messages.
    let provider_prompt = convert_to_language_model_prompt(&msgs, None).unwrap();
    let out = convert_prompt_to_openai_messages(&provider_prompt);
    assert_eq!(out.len(), 3);
    // The tool message must carry tool_call_id (the core of issue 1).
    assert_eq!(out[2]["role"], json!("tool"));
    assert_eq!(out[2]["tool_call_id"], json!("tc1"));
    assert_eq!(
        out[2]["content"],
        json!("Successfully wrote to /tmp/test.txt")
    );
}

// ── Issue 2: reasoning_content replay on the request side ───────────────────

/// The user-facing `ModelMessage` + `ModelPrompt` path (not just the
/// provider-facing prompt) also surfaces reasoning_content, exercising the
/// `convert_to_language_model_prompt` → `convert_prompt_to_openai_messages`
/// pipeline end to end.
#[test]
fn model_prompt_path_emits_reasoning_content() {
    let prompt = ModelPrompt::Messages(vec![
        ModelMessage::user("hi"),
        ModelMessage {
            role: Role::Assistant,
            content: aimux_core::message::MessageContent::Parts(vec![
                ContentPart::reasoning("thinking about hi"),
                ContentPart::text("hello!"),
            ]),
        },
    ]);
    let msgs = match prompt {
        ModelPrompt::Messages(m) => m,
        _ => unreachable!(),
    };
    let provider_prompt = convert_to_language_model_prompt(&msgs, None).unwrap();
    let out = convert_prompt_to_openai_messages(&provider_prompt);
    let assistant = &out[1];
    assert_eq!(assistant["reasoning_content"], json!("thinking about hi"));
    assert_eq!(assistant["content"], json!("hello!"));
}
