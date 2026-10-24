//! Ported from upstream `ai/src/generate-text/parse-tool-call.test.ts`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_core::error::AiMuxError;
use aimux_core::parse_tool_call::{
    RepairToolCall, ToolCallRepair, ToolCallRepairContext, ToolCallRepairReply,
    apply_tool_call_repair, parse_tool_call,
};
use aimux_core::tool::{FunctionTool, RawToolCall, Tool};
use serde_json::json;

fn weather_tool() -> Tool {
    FunctionTool::new("weather", weather_tool_schema()).into()
}

fn raw(name: &str, input: &str) -> RawToolCall {
    RawToolCall {
        tool_call_id: "call-1".into(),
        tool_name: name.into(),
        input: input.into(),
        provider_executed: None,
        dynamic: None,
        provider_metadata: None,
    }
}

/// TS: should successfully parse a valid tool call
#[tokio::test]
async fn parses_and_validates_exact_json() {
    let mut tool_call = raw("weather", r#"{"city":"Singapore","days":3}"#);
    tool_call.dynamic = Some(true);
    let call = parse_tool_call(tool_call, Some(&[weather_tool()]), None, &[], None).await;

    assert_eq!(call.input, json!({ "city": "Singapore", "days": 3 }));
    assert_eq!(call.dynamic, None);
    assert_eq!(call.invalid, None);
    assert!(call.error.is_none());
}

/// A static call has no `dynamic` key at all (`ai@7.0.127
/// src/generate-text/parse-tool-call.ts:279-288`), as in the core output
/// example of the alignment doc.
#[tokio::test]
async fn a_static_call_serializes_without_dynamic() {
    let call = parse_tool_call(
        raw("weather", r#"{"city":"Shanghai"}"#),
        Some(&[weather_tool()]),
        None,
        &[],
        None,
    )
    .await;
    let wire = serde_json::to_value(&call).unwrap();
    assert_eq!(wire["input"], json!({ "city": "Shanghai" }));
    assert!(wire.get("dynamic").is_none(), "{wire}");
}

/// TS: should successfully process empty tool calls for tools that have no inputSchema
#[tokio::test]
async fn validates_empty_input_as_an_empty_object() {
    let no_arg_tool = FunctionTool::new(
        "ping",
        json!({ "type": "object", "additionalProperties": false }),
    );
    let call = parse_tool_call(
        raw("ping", " \n"),
        Some(&[no_arg_tool.into()]),
        None,
        &[],
        None,
    )
    .await;

    assert_eq!(call.input, json!({}));
    assert_eq!(call.invalid, None);
}

/// TS: should throw InvalidToolInputError when args are invalid
#[tokio::test]
async fn preserves_parsed_input_when_schema_validation_fails() {
    let input = r#"{"city":7}"#;
    let call = parse_tool_call(
        raw("weather", input),
        Some(&[weather_tool()]),
        None,
        &[],
        None,
    )
    .await;

    assert_eq!(call.input, json!({ "city": 7 }));
    assert_eq!(call.invalid, Some(true));
    assert!(matches!(
        call.error,
        Some(AiMuxError::InvalidToolInput { .. })
    ));
}

/// TS: should throw NoSuchToolError when tool is not found
#[tokio::test]
async fn unknown_tool_is_an_invalid_dynamic_call_with_available_tools() {
    let call = parse_tool_call(
        raw("forecast", r#"{"city":"Tokyo"}"#),
        Some(&[weather_tool()]),
        None,
        &[],
        None,
    )
    .await;

    // The arguments still parse: `response_messages` drops a non-structured
    // input from the next turn's transcript, so an unknown tool called with
    // perfectly good arguments must not arrive here as raw text.
    assert_eq!(call.input, json!({ "city": "Tokyo" }));
    assert_eq!(call.dynamic, Some(true));
    assert_eq!(call.invalid, Some(true));
    assert!(matches!(
        call.error,
        Some(AiMuxError::NoSuchTool { available_tools, .. })
            if available_tools == Some(vec!["weather".to_string()])
    ));
}

/// The input of an invalid call is back-filled with the same prototype-safe
/// parse as the main path: `ai@7.0.127 parse-tool-call.ts:115-117` uses
/// `safeParseJSON`, whose `secureJsonParse` rejects a `__proto__` key or a
/// `constructor.prototype` anywhere in the value
/// (`@ai-sdk/provider-utils@5.0.53 secure-json-parse.ts:49-67`), so such text
/// stays the verbatim string.
#[tokio::test]
async fn invalid_call_input_is_not_back_filled_with_forbidden_prototype_keys() {
    for text in [
        r#"{"city":"Tokyo","__proto__":{"polluted":true}}"#,
        r#"{"city":"Tokyo","days":[{"constructor":{"prototype":{"polluted":true}}}]}"#,
    ] {
        // A known tool: the main path rejects the text.
        let call = parse_tool_call(
            raw("weather", text),
            Some(&[weather_tool()]),
            None,
            &[],
            None,
        )
        .await;
        assert_eq!(call.invalid, Some(true), "{text}");
        assert!(
            matches!(call.error, Some(AiMuxError::InvalidToolInput { .. })),
            "{text}"
        );
        assert_eq!(call.input, json!(text), "{text}");

        // An unknown tool: invalid for another reason, but the back-fill
        // still refuses the value.
        let call = parse_tool_call(
            raw("forecast", text),
            Some(&[weather_tool()]),
            None,
            &[],
            None,
        )
        .await;
        assert!(
            matches!(call.error, Some(AiMuxError::NoSuchTool { .. })),
            "{text}"
        );
        assert_eq!(call.input, json!(text), "{text}");
    }
}

/// TS: should invoke repairTool when provided and use its result / should pass instructions to repairToolCall
#[tokio::test]
async fn repair_runs_once_and_the_replacement_is_fully_revalidated() {
    let calls = Arc::new(AtomicUsize::new(0));
    let repair_calls = Arc::clone(&calls);
    let repair = ToolCallRepair::new(move |context| {
        repair_calls.fetch_add(1, Ordering::SeqCst);
        async move {
            assert!(matches!(context.error, AiMuxError::InvalidToolInput { .. }));
            assert_eq!(context.instructions.as_deref(), Some("Use metric units"));
            assert_eq!(context.system, context.instructions);
            Ok(Some(RawToolCall {
                tool_name: "weather".into(),
                input: r#"{"city":"Singapore","days":3}"#.into(),
                ..context.tool_call
            }))
        }
    });

    let call = parse_tool_call(
        raw("weather", r#"{"city":"Singapore"#),
        Some(&[weather_tool()]),
        Some(&repair),
        &[],
        Some("Use metric units"),
    )
    .await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(call.input, json!({ "city": "Singapore", "days": 3 }));
    assert_eq!(call.invalid, None);
}

#[tokio::test]
async fn an_invalid_repair_is_not_repaired_again() {
    let calls = Arc::new(AtomicUsize::new(0));
    let repair_calls = Arc::clone(&calls);
    let repair = ToolCallRepair::new(move |context| {
        repair_calls.fetch_add(1, Ordering::SeqCst);
        async move {
            Ok(Some(RawToolCall {
                input: r#"{"city":7}"#.into(),
                ..context.tool_call
            }))
        }
    });

    let call = parse_tool_call(
        raw("weather", "{"),
        Some(&[weather_tool()]),
        Some(&repair),
        &[],
        None,
    )
    .await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // The invalid call is built from the original call (ai@7.0.127
    // parse-tool-call.ts:112-133) and carries the repaired call's own
    // validation error, not wrapped: only a failing repair function is
    // wrapped in ToolCallRepairError (:90-96, :103-110).
    assert_eq!(call.input, json!("{"));
    assert_eq!(call.invalid, Some(true));
    assert!(matches!(
        call.error,
        Some(AiMuxError::InvalidToolInput { ref tool_input, .. }) if tool_input == r#"{"city":7}"#
    ));
}

/// The host-reply path shares the post-repair contract: a `repaired` reply
/// that still fails validation leaves the original call invalid with the
/// reply's own error, not a `ToolCallRepair` error (ai@7.0.127
/// parse-tool-call.ts:103-133).
#[tokio::test]
async fn a_repaired_reply_that_fails_validation_is_not_wrapped() {
    let tools = [weather_tool()];
    let invalid = parse_tool_call(raw("weather", "{"), Some(&tools), None, &[], None).await;

    let call = apply_tool_call_repair(
        &invalid,
        Some(&tools),
        ToolCallRepairReply::Repaired {
            tool_call: raw("weather", r#"{"city":7}"#),
        },
    )
    .unwrap();

    assert_eq!(call.input, json!("{"));
    assert_eq!(call.invalid, Some(true));
    assert!(matches!(
        call.error,
        Some(AiMuxError::InvalidToolInput { ref tool_input, .. }) if tool_input == r#"{"city":7}"#
    ));
}

/// A struct implementing `RepairToolCall` is used as the hook, like a closure.
#[tokio::test]
async fn repair_hook_can_be_a_trait_implementation() {
    struct FixInput {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl RepairToolCall for FixInput {
        async fn repair(
            &self,
            context: ToolCallRepairContext,
        ) -> Result<Option<RawToolCall>, AiMuxError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some(RawToolCall {
                input: r#"{"city":"Singapore","days":3}"#.into(),
                ..context.tool_call
            }))
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let repair = ToolCallRepair::from(FixInput {
        calls: Arc::clone(&calls),
    });

    let call = parse_tool_call(
        raw("weather", r#"{"city":"Singapore"#),
        Some(&[weather_tool()]),
        Some(&repair),
        &[],
        None,
    )
    .await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(call.input, json!({ "city": "Singapore", "days": 3 }));
    assert_eq!(call.invalid, None);
}

/// TS: should throw NoSuchToolError when tools is null
#[tokio::test]
async fn missing_tools_bypasses_repair_like_ai_sdk() {
    let calls = Arc::new(AtomicUsize::new(0));
    let repair_calls = Arc::clone(&calls);
    let repair = ToolCallRepair::new(move |_| {
        repair_calls.fetch_add(1, Ordering::SeqCst);
        async { Ok(None) }
    });

    let call = parse_tool_call(raw("weather", "{}"), None, Some(&repair), &[], None).await;

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        call.error,
        Some(AiMuxError::NoSuchTool {
            available_tools: None,
            ..
        })
    ));
}

/// TS: should re-throw error if tool call repair returns null
#[tokio::test]
async fn repair_returning_none_keeps_the_original_failure() {
    let repair = ToolCallRepair::new(|_| async { Ok(None) });
    let original_input = "{";
    let call = parse_tool_call(
        raw("weather", original_input),
        Some(&[weather_tool()]),
        Some(&repair),
        &[],
        None,
    )
    .await;

    assert_eq!(call.input, json!(original_input));
    assert!(matches!(
        call.error,
        Some(AiMuxError::InvalidToolInput { tool_input, .. })
            if tool_input == original_input
    ));
}

/// TS: should throw ToolCallRepairError if repairToolCall throws
#[tokio::test]
async fn repair_failure_keeps_both_typed_errors() {
    let repair = ToolCallRepair::new(|context| async move {
        assert_eq!(context.input_schema("weather"), weather_tool_schema());
        Err(AiMuxError::other("repair model failed"))
    });

    let call = parse_tool_call(
        raw("weather", "{"),
        Some(&[weather_tool()]),
        Some(&repair),
        &[],
        None,
    )
    .await;

    assert!(matches!(
        call.error,
        Some(AiMuxError::ToolCallRepair { original_error, cause })
            if matches!(*original_error, AiMuxError::InvalidToolInput { .. })
                && matches!(*cause, AiMuxError::Other { .. })
    ));
}

/// TS: should successfully parse a valid provider-executed dynamic tool call
#[tokio::test]
async fn provider_executed_dynamic_calls_do_not_require_a_local_tool() {
    let mut tool_call = raw("provider_search", r#"{"query":"rust"}"#);
    tool_call.provider_executed = Some(true);
    tool_call.dynamic = Some(true);

    let call = parse_tool_call(tool_call, None, None, &[], None).await;

    assert_eq!(call.input, json!({ "query": "rust" }));
    assert_eq!(call.invalid, None);
}

fn weather_tool_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "city": { "type": "string" },
            "days": { "type": "integer" }
        },
        "required": ["city"],
        "additionalProperties": false
    })
}
