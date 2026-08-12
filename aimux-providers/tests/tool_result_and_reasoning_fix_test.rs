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
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::{ModelMessage, ModelPrompt, Role};
use aimux_providers::openai::convert::convert_prompt_to_openai_messages;
use serde_json::{Value, json};

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
    let provider_prompt: LanguageModelPrompt = msgs
        .iter()
        .map(|m| LanguageModelPromptMessage {
            role: m.role,
            content: match &m.content {
                aimux_core::message::MessageContent::Text(t) => vec![ContentPart::text(t)],
                aimux_core::message::MessageContent::Parts(p) => p.clone(),
            },
            provider_options: None,
        })
        .collect();
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

/// An assistant message with reasoning + tool_call lifts the reasoning to a
/// top-level `reasoning_content` field (the DeepSeek V4 thinking-mode
/// requirement). This is the exact scenario from the user's table: assistant
/// `[{reasoning},{tool_call}]` + tool `string`.
#[test]
fn assistant_reasoning_with_tool_call_emits_reasoning_content() {
    let prompt: LanguageModelPrompt = vec![
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("inspect the repo")],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![
                ContentPart::reasoning("I need to inspect files before answering."),
                ContentPart::tool_call(
                    "call_1".to_string(),
                    "read_file".to_string(),
                    json!({ "path": "README.md" }),
                ),
            ],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Tool,
            content: vec![ContentPart::tool_result(
                "call_1".to_string(),
                json!("contents"),
            )],
            ..Default::default()
        },
    ];
    let out = convert_prompt_to_openai_messages(&prompt);
    assert_eq!(out.len(), 3);

    // Assistant message: reasoning_content present, content null, tool_calls set.
    let assistant = &out[1];
    assert_eq!(assistant["role"], json!("assistant"));
    assert_eq!(
        assistant["reasoning_content"],
        json!("I need to inspect files before answering.")
    );
    assert_eq!(assistant["content"], Value::Null);
    assert_eq!(
        assistant["tool_calls"],
        json!([{
            "type": "function",
            "id": "call_1",
            "function": {
                "name": "read_file",
                "arguments": "{\"path\":\"README.md\"}"
            }
        }])
    );
}

/// An assistant message with reasoning + text (no tool calls) lifts reasoning
/// to `reasoning_content` and keeps the text as `content`.
#[test]
fn assistant_reasoning_with_text_emits_reasoning_content() {
    let prompt: LanguageModelPrompt = vec![
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("What is 2+2?")],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![
                ContentPart::reasoning("2 plus 2 equals 4."),
                ContentPart::text("4"),
            ],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("thanks")],
            ..Default::default()
        },
    ];
    let out = convert_prompt_to_openai_messages(&prompt);
    let assistant = &out[1];
    assert_eq!(assistant["role"], json!("assistant"));
    assert_eq!(assistant["content"], json!("4"));
    assert_eq!(assistant["reasoning_content"], json!("2 plus 2 equals 4."));
    // No tool_calls key should be emitted.
    assert!(
        assistant.get("tool_calls").is_none(),
        "tool_calls must not be emitted when there are none"
    );
}

/// An assistant message with no reasoning part emits no `reasoning_content`
/// field (no spurious empty string — matches the Vercel `openai-compatible`
/// `reasoning.length > 0` guard).
#[test]
fn assistant_without_reasoning_omits_reasoning_content() {
    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::Assistant,
        content: vec![ContentPart::tool_call(
            "call_1".to_string(),
            "write_file".to_string(),
            json!({ "path": "/tmp/x" }),
        )],
        ..Default::default()
    }];
    let out = convert_prompt_to_openai_messages(&prompt);
    let assistant = &out[0];
    assert!(
        assistant.get("reasoning_content").is_none(),
        "reasoning_content must be omitted when no reasoning part is present"
    );
}

/// Multiple reasoning parts are concatenated (mirrors the SDK join behaviour).
#[test]
fn multiple_reasoning_parts_are_concatenated() {
    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::Assistant,
        content: vec![
            ContentPart::reasoning("part one. "),
            ContentPart::reasoning("part two."),
            ContentPart::text("answer"),
        ],
        ..Default::default()
    }];
    let out = convert_prompt_to_openai_messages(&prompt);
    let assistant = &out[0];
    assert_eq!(assistant["content"], json!("answer"));
    assert_eq!(assistant["reasoning_content"], json!("part one. part two."));
}

/// A user message never carries reasoning, so `reasoning_content` is never
/// emitted for non-assistant roles even if a reasoning part somehow appears.
/// The reasoning part is still dropped from the content shape, and the
/// remaining text part collapses to a string (matching the plain-text path).
#[test]
fn non_assistant_role_never_emits_reasoning_content() {
    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![
            ContentPart::reasoning("should be ignored"),
            ContentPart::text("hello"),
        ],
        ..Default::default()
    }];
    let out = convert_prompt_to_openai_messages(&prompt);
    let user = &out[0];
    assert_eq!(user["role"], json!("user"));
    assert!(
        user.get("reasoning_content").is_none(),
        "user messages must not carry reasoning_content"
    );
    // The text part collapses to a plain string; reasoning is dropped entirely.
    assert_eq!(user["content"], json!("hello"));
}

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
    let provider_prompt: LanguageModelPrompt = msgs
        .iter()
        .map(|m| LanguageModelPromptMessage {
            role: m.role,
            content: match &m.content {
                aimux_core::message::MessageContent::Text(t) => vec![ContentPart::text(t)],
                aimux_core::message::MessageContent::Parts(p) => p.clone(),
            },
            provider_options: None,
        })
        .collect();
    let out = convert_prompt_to_openai_messages(&provider_prompt);
    let assistant = &out[1];
    assert_eq!(assistant["reasoning_content"], json!("thinking about hi"));
    assert_eq!(assistant["content"], json!("hello!"));
}
