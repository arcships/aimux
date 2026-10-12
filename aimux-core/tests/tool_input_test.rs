//! Ported from upstream `ai/src/generate-text/parse-tool-call.test.ts`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_core::error::AiMuxError;
use aimux_core::parse_tool_call::{ToolCallRepair, parse_tool_call};
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
        Err(AiMuxError::Other("repair model failed".into()))
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
                && matches!(*cause, AiMuxError::Other(_))
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
