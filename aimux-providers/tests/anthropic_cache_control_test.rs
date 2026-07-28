//! Rust port of the Anthropic `cache_control` tests.
//!
//! Translated from the Vercel AI SDK TypeScript test suites:
//! - `packages/anthropic/src/convert-to-anthropic-prompt.test.ts`
//!   `describe('cache control')` (L3557-4263) — conversion-level tests that
//!   verify `cache_control` is emitted on the correct Anthropic content blocks.
//! - `packages/anthropic/src/anthropic-language-model.test.ts`
//!   `it('should support cache control')` (L2298) and
//!   `it('should support cache control and return extra fields in provider metadata')` (L2457)
//!   — exercised here at the conversion level (the request-body shape that
//!   `doGenerate` builds), since the cache_control feature lives in
//!   `convert_prompt_to_anthropic_full`.
//!
//! `cache_control` is read from `providerOptions.anthropic.cacheControl`
//! (`{ "type": "ephemeral" }`) at three levels:
//! 1. **Part-level** — on `ContentPart` (`Text`, `ToolCall`, `ToolResult`, …).
//! 2. **Message-level** — on `LanguageModelPromptMessage.provider_options`,
//!    applied to the last part of the message.
//! 3. **Tool-result output-level** — on a structured `output.providerOptions`
//!    or on the first content part of a `content` output.
//!
//! A `CacheControlValidator` caps the total breakpoints at 4 and rejects
//! `cache_control` on thinking / redacted-thinking blocks (emitting a warning).

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::types::Warning;
use aimux_providers::anthropic::convert::convert_prompt_to_anthropic_full;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn msg(role: Role, parts: Vec<ContentPart>) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role,
        content: parts,
        provider_options: None,
    }
}

/// Build a message with message-level `provider_options` (for message-level
/// cache_control).
fn msg_with_opts(role: Role, parts: Vec<ContentPart>, opts: Value) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role,
        content: parts,
        provider_options: Some(opts),
    }
}

fn prompt(msgs: Vec<LanguageModelPromptMessage>) -> Vec<LanguageModelPromptMessage> {
    msgs
}

fn convert_full(
    msgs: Vec<LanguageModelPromptMessage>,
    send_reasoning: bool,
) -> aimux_providers::anthropic::convert::AnthropicPromptConversion {
    convert_prompt_to_anthropic_full(&msgs, send_reasoning)
}

/// The message-level / part-level `providerOptions` shape carrying
/// `anthropic.cacheControl`.
fn cache_control_opts(cache_control: Value) -> Value {
    json!({ "anthropic": { "cacheControl": cache_control } })
}

/// Create a text part with `provider_options` set (for part-level cache_control).
fn text_part_with_cache_control(t: &str, cache_control: Value) -> ContentPart {
    ContentPart::Text {
        text: t.to_string(),
        provider_options: Some(cache_control_opts(cache_control)),
    }
}

/// Extract the content array from the first message in the conversion result.
fn first_message_content(
    result: &aimux_providers::anthropic::convert::AnthropicPromptConversion,
) -> &Vec<Value> {
    result
        .messages
        .first()
        .expect("expected at least one message")
        .get("content")
        .and_then(|c| c.as_array())
        .expect("expected content array")
}

/// Extract the first system block from the conversion result.
fn first_system_block(
    result: &aimux_providers::anthropic::convert::AnthropicPromptConversion,
) -> &Value {
    result
        .system
        .as_ref()
        .expect("expected system blocks")
        .first()
        .expect("expected at least one system block")
}

// ===========================================================================
// describe('cache control') > describe('system message')
// ===========================================================================

mod system_message {
    use super::*;

    // TS: "should set cache_control on system message with message cache control"
    //
    // Message-level `providerOptions.anthropic.cacheControl` on a system
    // message. The cache_control is applied to the system text block.
    #[test]
    fn should_set_cache_control_on_system_message_with_message_cache_control() {
        let p = prompt(vec![msg_with_opts(
            Role::System,
            vec![ContentPart::text("system message")],
            cache_control_opts(json!({ "type": "ephemeral" })),
        )]);
        let result = convert_full(p, true);
        let system = first_system_block(&result);
        assert_eq!(
            system.get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
    }
}

// ===========================================================================
// describe('cache control') > describe('user message')
// ===========================================================================

mod user_message {
    use super::*;

    // TS: "should set cache_control on user message part with part cache control"
    //
    // Part-level `providerOptions.anthropic.cacheControl` on a text part.
    #[test]
    fn should_set_cache_control_on_user_message_part_with_part_cache_control() {
        let p = prompt(vec![msg(
            Role::User,
            vec![text_part_with_cache_control(
                "test",
                json!({ "type": "ephemeral" }),
            )],
        )]);
        let result = convert_full(p, true);
        let content = first_message_content(&result);
        assert_eq!(content.len(), 1);
        assert_eq!(content[0].get("type"), Some(&json!("text")));
        assert_eq!(content[0].get("text"), Some(&json!("test")));
        assert_eq!(
            content[0].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
    }

    // TS: "should set cache_control on last user message part with message cache control"
    //
    // Message-level `providerOptions.anthropic.cacheControl` applied to the
    // last part of the message.
    #[test]
    fn should_set_cache_control_on_last_user_message_part_with_message_cache_control() {
        let p = prompt(vec![msg_with_opts(
            Role::User,
            vec![ContentPart::text("part1"), ContentPart::text("part2")],
            cache_control_opts(json!({ "type": "ephemeral" })),
        )]);
        let result = convert_full(p, true);
        let content = first_message_content(&result);
        assert_eq!(content.len(), 2);
        // First part: no cache_control
        assert!(content[0].get("cache_control").is_none());
        // Last part: cache_control from message-level providerOptions
        assert_eq!(
            content[1].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
    }
}

// ===========================================================================
// describe('cache control') > describe('assistant message')
// ===========================================================================

mod assistant_message {
    use super::*;

    // TS: "should set cache_control on assistant message text part with part cache control"
    //
    // Part-level `providerOptions.anthropic.cacheControl` on an assistant text part.
    #[test]
    fn should_set_cache_control_on_assistant_message_text_part_with_part_cache_control() {
        let p = prompt(vec![
            msg(Role::User, vec![ContentPart::text("user-content")]),
            msg(
                Role::Assistant,
                vec![text_part_with_cache_control(
                    "test",
                    json!({ "type": "ephemeral" }),
                )],
            ),
        ]);
        let result = convert_full(p, true);
        // messages[0] = user, messages[1] = assistant
        let assistant_content = result
            .messages
            .get(1)
            .expect("expected assistant message")
            .get("content")
            .and_then(|c| c.as_array())
            .expect("expected content array");
        assert_eq!(assistant_content.len(), 1);
        assert_eq!(assistant_content[0].get("type"), Some(&json!("text")));
        assert_eq!(assistant_content[0].get("text"), Some(&json!("test")));
        assert_eq!(
            assistant_content[0].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
    }

    // TS: "should set cache_control on assistant tool call part with part cache control"
    //
    // Part-level `providerOptions` on a `tool-call` content part.
    #[test]
    fn should_set_cache_control_on_assistant_tool_call_part_with_part_cache_control() {
        let p = prompt(vec![
            msg(Role::User, vec![ContentPart::text("user-content")]),
            msg(
                Role::Assistant,
                vec![ContentPart::ToolCall {
                    tool_call_id: "test-id".to_string(),
                    tool_name: "test-tool".to_string(),
                    input: json!({ "some": "arg" }),
                    provider_options: Some(cache_control_opts(json!({ "type": "ephemeral" }))),
                }],
            ),
        ]);
        let result = convert_full(p, true);
        let assistant_content = result
            .messages
            .get(1)
            .expect("expected assistant message")
            .get("content")
            .and_then(|c| c.as_array())
            .expect("expected content array");
        assert_eq!(assistant_content[0].get("type"), Some(&json!("tool_use")));
        assert_eq!(assistant_content[0].get("name"), Some(&json!("test-tool")));
        assert_eq!(assistant_content[0].get("id"), Some(&json!("test-id")));
        assert_eq!(
            assistant_content[0].get("input"),
            Some(&json!({ "some": "arg" }))
        );
        assert_eq!(
            assistant_content[0].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
    }

    // TS: "should wrap non-object (invalid) tool call input in an object"
    //
    // This lives inside the `describe('assistant message')` cache_control block
    // in the TS source. The same case is also covered in
    // `anthropic_convert_test.rs`; reproduced here for parity with the TS suite.
    #[test]
    fn should_wrap_non_object_invalid_tool_call_input_in_an_object() {
        let p = prompt(vec![
            msg(Role::User, vec![ContentPart::text("user-content")]),
            msg(
                Role::Assistant,
                vec![ContentPart::tool_call(
                    "test-id".to_string(),
                    "test-tool".to_string(),
                    json!("invalid"),
                )],
            ),
        ]);
        let result = convert_full(p, true);
        let assistant_content = result
            .messages
            .get(1)
            .expect("expected assistant message")
            .get("content")
            .and_then(|c| c.as_array())
            .expect("expected content array");
        assert_eq!(assistant_content[0].get("type"), Some(&json!("tool_use")));
        assert_eq!(
            assistant_content[0].get("input"),
            Some(&json!({ "rawInvalidInput": "invalid" }))
        );
    }

    // TS: "should set cache_control on last assistant message part with message cache control"
    //
    // Message-level `providerOptions.anthropic.cacheControl`.
    #[test]
    fn should_set_cache_control_on_last_assistant_message_part_with_message_cache_control() {
        let p = prompt(vec![
            msg(Role::User, vec![ContentPart::text("user-content")]),
            msg_with_opts(
                Role::Assistant,
                vec![ContentPart::text("part1"), ContentPart::text("part2")],
                cache_control_opts(json!({ "type": "ephemeral" })),
            ),
        ]);
        let result = convert_full(p, true);
        let assistant_content = result
            .messages
            .get(1)
            .expect("expected assistant message")
            .get("content")
            .and_then(|c| c.as_array())
            .expect("expected content array");
        assert_eq!(assistant_content.len(), 2);
        // First part: no cache_control
        assert!(assistant_content[0].get("cache_control").is_none());
        // Last part: cache_control from message-level providerOptions
        assert_eq!(
            assistant_content[1].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
    }
}

// ===========================================================================
// describe('cache control') > describe('tool message')
// ===========================================================================

mod tool_message {
    use super::*;

    // TS: "should set cache_control on tool result message part with part cache control"
    //
    // Part-level `providerOptions` on a `tool-result` content part.
    #[test]
    fn should_set_cache_control_on_tool_result_message_part_with_part_cache_control() {
        let p = prompt(vec![msg(
            Role::Tool,
            vec![ContentPart::ToolResult {
                tool_call_id: "test".to_string(),
                output: json!({ "test": "test" }),
                provider_options: Some(cache_control_opts(json!({ "type": "ephemeral" }))),
            }],
        )]);
        let result = convert_full(p, true);
        let content = first_message_content(&result);
        assert_eq!(content[0].get("type"), Some(&json!("tool_result")));
        assert_eq!(content[0].get("tool_use_id"), Some(&json!("test")));
        assert_eq!(
            content[0].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
    }

    // TS: "should set cache_control on tool result with output cache control"
    //
    // The `output` of a tool-result carries `providerOptions` (here a `text`
    // output with `providerOptions.anthropic.cacheControl`).
    #[test]
    fn should_set_cache_control_on_tool_result_with_output_cache_control() {
        let p = prompt(vec![msg(
            Role::Tool,
            vec![ContentPart::tool_result(
                "test".to_string(),
                json!({
                    "type": "text",
                    "value": "test",
                    "providerOptions": cache_control_opts(json!({ "type": "ephemeral" })),
                }),
            )],
        )]);
        let result = convert_full(p, true);
        let content = first_message_content(&result);
        assert_eq!(content[0].get("type"), Some(&json!("tool_result")));
        assert_eq!(
            content[0].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
    }

    // TS: "should set cache_control on tool result with content output cache control"
    //
    // The `output` is a `content` output whose first content part carries
    // `providerOptions.anthropic.cacheControl`.
    #[test]
    fn should_set_cache_control_on_tool_result_with_content_output_cache_control() {
        let p = prompt(vec![msg(
            Role::Tool,
            vec![ContentPart::tool_result(
                "test".to_string(),
                json!({
                    "type": "content",
                    "value": [{
                        "type": "text",
                        "text": "test",
                        "providerOptions": cache_control_opts(json!({ "type": "ephemeral" })),
                    }],
                }),
            )],
        )]);
        let result = convert_full(p, true);
        let content = first_message_content(&result);
        assert_eq!(content[0].get("type"), Some(&json!("tool_result")));
        assert_eq!(
            content[0].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
    }

    // TS: "should set cache_control on last tool result message part with message cache control"
    //
    // Message-level `providerOptions.anthropic.cacheControl`.
    #[test]
    fn should_set_cache_control_on_last_tool_result_message_part_with_message_cache_control() {
        let p = prompt(vec![msg_with_opts(
            Role::Tool,
            vec![
                ContentPart::tool_result("part1".to_string(), json!({ "test": "part1" })),
                ContentPart::tool_result("part2".to_string(), json!({ "test": "part2" })),
            ],
            cache_control_opts(json!({ "type": "ephemeral" })),
        )]);
        let result = convert_full(p, true);
        let content = first_message_content(&result);
        assert_eq!(content.len(), 2);
        // First part: no cache_control
        assert!(content[0].get("cache_control").is_none());
        // Last part: cache_control from message-level providerOptions
        assert_eq!(
            content[1].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
    }
}

// ===========================================================================
// describe('cache control') > describe('cache control validation')
// ===========================================================================

mod cache_control_validation {
    use super::*;

    // TS: "should reject cache_control on thinking blocks"
    //
    // When a reasoning part (with a signature → thinking block) has
    // `providerOptions.anthropic.cacheControl`, the CacheControlValidator emits
    // a warning and does NOT include cache_control on the thinking block.
    #[test]
    fn should_reject_cache_control_on_thinking_blocks() {
        let p = prompt(vec![msg(
            Role::Assistant,
            vec![ContentPart::Reasoning {
                text: "thinking content".to_string(),
                signature: Some("test-sig".to_string()),
                provider_options: Some(cache_control_opts(json!({ "type": "ephemeral" }))),
            }],
        )]);
        let result = convert_full(p, true);
        let content = first_message_content(&result);
        assert_eq!(content[0].get("type"), Some(&json!("thinking")));
        assert_eq!(content[0].get("thinking"), Some(&json!("thinking content")));
        assert_eq!(content[0].get("signature"), Some(&json!("test-sig")));
        // cache_control should NOT be present on thinking blocks
        assert!(content[0].get("cache_control").is_none());

        // A warning should be emitted about the rejected cache_control.
        let has_warning = result.warnings.iter().any(|w| match w {
            Warning::Unsupported {
                feature, details, ..
            } => {
                feature == "cache_control on non-cacheable context"
                    && details
                        .as_deref()
                        .is_some_and(|d| d.contains("thinking block"))
            }
            _ => false,
        });
        assert!(
            has_warning,
            "expected a cache_control rejection warning, got: {:?}",
            result.warnings
        );
    }

    // TS: "should reject cache_control on redacted thinking blocks"
    //
    // A reasoning part with `providerOptions.anthropic.redactedData` (and no
    // signature) becomes a `redacted_thinking` block; cache_control is rejected.
    #[test]
    fn should_reject_cache_control_on_redacted_thinking_blocks() {
        let p = prompt(vec![msg(
            Role::Assistant,
            vec![ContentPart::Reasoning {
                text: "redacted".to_string(),
                signature: None,
                provider_options: Some(json!({
                    "anthropic": {
                        "redactedData": "abc123",
                        "cacheControl": { "type": "ephemeral" },
                    }
                })),
            }],
        )]);
        let result = convert_full(p, true);
        let content = first_message_content(&result);
        assert_eq!(content[0].get("type"), Some(&json!("redacted_thinking")));
        assert_eq!(content[0].get("data"), Some(&json!("abc123")));
        // cache_control should NOT be present on redacted thinking blocks
        assert!(content[0].get("cache_control").is_none());

        let has_warning = result.warnings.iter().any(|w| match w {
            Warning::Unsupported {
                feature, details, ..
            } => {
                feature == "cache_control on non-cacheable context"
                    && details
                        .as_deref()
                        .is_some_and(|d| d.contains("redacted thinking block"))
            }
            _ => false,
        });
        assert!(
            has_warning,
            "expected a cache_control rejection warning, got: {:?}",
            result.warnings
        );
    }
}

// ===========================================================================
// describe('cache control') > "should limit cache breakpoints to 4"
// ===========================================================================

mod breakpoint_limit {
    use super::*;

    // TS: "should limit cache breakpoints to 4"
    //
    // The CacheControlValidator tracks the number of cache_control breakpoints
    // across the entire prompt and rejects any beyond the 4th. The first 4
    // get cache_control; the 5th is rejected with a warning.
    #[test]
    fn should_limit_cache_breakpoints_to_4() {
        // Build a prompt with 5 cache_control breakpoints:
        // 1. system 1 (cache_control)
        // 2. system 2 (cache_control)
        // 3. user 1 (cache_control)
        // 4. assistant 1 (cache_control)
        // 5. user 2 (cache_control — should be rejected)
        let p = prompt(vec![
            msg(
                Role::System,
                vec![text_part_with_cache_control(
                    "system 1",
                    json!({ "type": "ephemeral" }),
                )],
            ),
            msg(
                Role::System,
                vec![text_part_with_cache_control(
                    "system 2",
                    json!({ "type": "ephemeral" }),
                )],
            ),
            msg(
                Role::User,
                vec![text_part_with_cache_control(
                    "user 1",
                    json!({ "type": "ephemeral" }),
                )],
            ),
            msg(
                Role::Assistant,
                vec![text_part_with_cache_control(
                    "assistant 1",
                    json!({ "type": "ephemeral" }),
                )],
            ),
            msg(
                Role::User,
                vec![text_part_with_cache_control(
                    "user 2 (should be rejected)",
                    json!({ "type": "ephemeral" }),
                )],
            ),
        ]);
        let result = convert_full(p, true);

        // First 4 should have cache_control
        let system = result.system.as_ref().expect("expected system blocks");
        assert_eq!(system.len(), 2);
        assert_eq!(
            system[0].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
        assert_eq!(
            system[1].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );

        let msg0_content = result
            .messages
            .first()
            .expect("expected message 0")
            .get("content")
            .and_then(|c| c.as_array())
            .expect("expected content array");
        assert_eq!(
            msg0_content[0].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );

        let msg1_content = result
            .messages
            .get(1)
            .expect("expected message 1")
            .get("content")
            .and_then(|c| c.as_array())
            .expect("expected content array");
        assert_eq!(
            msg1_content[0].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );

        // 5th should be rejected (no cache_control)
        let msg2_content = result
            .messages
            .get(2)
            .expect("expected message 2")
            .get("content")
            .and_then(|c| c.as_array())
            .expect("expected content array");
        assert!(
            msg2_content[0].get("cache_control").is_none(),
            "5th breakpoint should be rejected"
        );

        // Should have warning about exceeding limit
        let has_limit_warning = result.warnings.iter().any(|w| match w {
            Warning::Unsupported {
                feature, details, ..
            } => {
                feature == "cacheControl breakpoint limit"
                    && details
                        .as_deref()
                        .is_some_and(|d| d.contains("Maximum 4 cache breakpoints exceeded"))
            }
            _ => false,
        });
        assert!(
            has_limit_warning,
            "expected a breakpoint limit warning, got: {:?}",
            result.warnings
        );
    }
}

// ===========================================================================
// Integration tests from anthropic-language-model.test.ts
//
// The TS integration tests exercise the full `doGenerate` path with a mock
// server. The cache_control feature itself lives in
// `convert_prompt_to_anthropic_full` (the request-body builder), so these are
// exercised here at the conversion level — verifying the request shape that
// `doGenerate` would send.
// ===========================================================================

mod integration {
    use super::*;

    // TS: "should support cache control" (L2298)
    //
    // A user message with message-level `providerOptions.anthropic.cacheControl`
    // produces a request whose text block carries `cache_control`.
    #[test]
    fn should_support_cache_control() {
        let p = prompt(vec![msg_with_opts(
            Role::User,
            vec![ContentPart::text("Hello")],
            cache_control_opts(json!({ "type": "ephemeral" })),
        )]);
        let result = convert_full(p, true);
        let content = first_message_content(&result);
        assert_eq!(content[0].get("type"), Some(&json!("text")));
        assert_eq!(content[0].get("text"), Some(&json!("Hello")));
        assert_eq!(
            content[0].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
    }

    // TS: "should support cache control and return extra fields in provider metadata" (L2457)
    //
    // Same as above but with `ttl: '1h'`; the request block carries the full
    // `cache_control` object including `ttl`.
    #[test]
    fn should_support_cache_control_and_return_extra_fields_in_provider_metadata() {
        let p = prompt(vec![msg_with_opts(
            Role::User,
            vec![ContentPart::text("Hello")],
            cache_control_opts(json!({ "type": "ephemeral", "ttl": "1h" })),
        )]);
        let result = convert_full(p, true);
        let content = first_message_content(&result);
        assert_eq!(
            content[0].get("cache_control"),
            Some(&json!({ "type": "ephemeral", "ttl": "1h" }))
        );
    }
}
