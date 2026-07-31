//! Rust translation of
//! `packages/provider-utils/src/streaming-tool-call-tracker.test.ts` (18 cases).
//!
//! The TS test collects emitted parts on a shared `controller.enqueue` sink and
//! resets it with `parts.length = 0` between assertions. The Rust equivalent
//! accumulates parts inside the tracker: inspect with `parts()` and reset with
//! `clear_parts()`.

use aimux_stream::{
    StreamingToolCallDelta, StreamingToolCallTracker, ToolCallStreamPart, TrackerError,
    TypeValidation,
};
use serde_json::{Value, json};

/// Convenience builder alias.
fn d() -> StreamingToolCallDelta {
    StreamingToolCallDelta::default()
}

/// A tracker with no metadata handling (`M = ()`), matching the TS tests that
/// don't exercise the metadata hooks. Pinning `M = ()` here lets type
/// inference resolve `ToolCallStreamPart::ToolCall { provider_metadata: None }`
/// in the assertions.
fn new_tracker() -> StreamingToolCallTracker<()> {
    StreamingToolCallTracker::<()>::new()
}

fn tool_call_id<M>(part: &ToolCallStreamPart<M>) -> &str {
    match part {
        ToolCallStreamPart::ToolCall { tool_call_id, .. } => tool_call_id,
        _ => panic!("expected tool-call part"),
    }
}

// == processDelta ==========================================================

#[test]
fn single_tool_call_accumulated_across_multiple_deltas() {
    // TS: "should handle a single tool call accumulated across multiple deltas"
    let mut tracker = new_tracker();

    // First delta: new tool call with id and name.
    tracker
        .process_delta(
            &d().index(0)
                .id("call_1")
                .tool_type("function")
                .function_name("get_weather")
                .arguments("{\"ci"),
        )
        .unwrap();

    assert_eq!(
        tracker.parts(),
        &[
            ToolCallStreamPart::ToolInputStart {
                id: "call_1".into(),
                tool_name: "get_weather".into(),
            },
            ToolCallStreamPart::ToolInputDelta {
                id: "call_1".into(),
                delta: "{\"ci".into(),
            },
        ]
    );

    tracker.clear_parts();

    // Second delta: more arguments.
    tracker
        .process_delta(&d().index(0).arguments("ty\": \"San"))
        .unwrap();

    assert_eq!(
        tracker.parts(),
        &[ToolCallStreamPart::ToolInputDelta {
            id: "call_1".into(),
            delta: "ty\": \"San".into(),
        }]
    );

    tracker.clear_parts();

    // Third delta: completes the JSON -- must not finalize before flush, since a
    // parsable buffer can still be the prefix of longer arguments.
    tracker
        .process_delta(&d().index(0).arguments(" Francisco\"}"))
        .unwrap();

    assert_eq!(
        tracker.parts(),
        &[ToolCallStreamPart::ToolInputDelta {
            id: "call_1".into(),
            delta: " Francisco\"}".into(),
        }]
    );

    tracker.clear_parts();

    tracker.flush();

    assert_eq!(
        tracker.parts(),
        &[
            ToolCallStreamPart::ToolInputEnd {
                id: "call_1".into(),
            },
            ToolCallStreamPart::ToolCall {
                tool_call_id: "call_1".into(),
                tool_name: "get_weather".into(),
                input: "{\"city\": \"San Francisco\"}".into(),
                provider_metadata: None,
            },
        ]
    );
}

#[test]
fn full_tool_call_in_single_chunk() {
    // TS: "should handle a full tool call in a single chunk"
    let mut tracker = new_tracker();

    tracker
        .process_delta(
            &d().index(0)
                .id("call_1")
                .tool_type("function")
                .function_name("get_weather")
                .arguments("{\"city\": \"London\"}"),
        )
        .unwrap();

    assert_eq!(
        tracker.parts(),
        &[
            ToolCallStreamPart::ToolInputStart {
                id: "call_1".into(),
                tool_name: "get_weather".into(),
            },
            ToolCallStreamPart::ToolInputDelta {
                id: "call_1".into(),
                delta: "{\"city\": \"London\"}".into(),
            },
        ]
    );

    tracker.clear_parts();
    tracker.flush();

    assert_eq!(
        tracker.parts(),
        &[
            ToolCallStreamPart::ToolInputEnd {
                id: "call_1".into(),
            },
            ToolCallStreamPart::ToolCall {
                tool_call_id: "call_1".into(),
                tool_name: "get_weather".into(),
                input: "{\"city\": \"London\"}".into(),
                provider_metadata: None,
            },
        ]
    );
}

#[test]
fn not_finalize_when_argument_prefix_is_parsable_json() {
    // TS: "should not finalize a tool call when its argument prefix is parsable JSON"
    let mut tracker = new_tracker();

    tracker
        .process_delta(
            &d().index(0)
                .id("call_1")
                .tool_type("function")
                .function_name("search")
                .arguments("{\"query\": \"test\"}"),
        )
        .unwrap();

    // The parsable prefix must not emit tool-input-end / tool-call.
    let types: Vec<&str> = tracker
        .parts()
        .iter()
        .map(|p| match p {
            ToolCallStreamPart::ToolInputStart { .. } => "tool-input-start",
            ToolCallStreamPart::ToolInputDelta { .. } => "tool-input-delta",
            ToolCallStreamPart::ToolInputEnd { .. } => "tool-input-end",
            ToolCallStreamPart::ToolCall { .. } => "tool-call",
        })
        .collect();
    assert_eq!(types, vec!["tool-input-start", "tool-input-delta"]);

    tracker
        .process_delta(&d().index(0).arguments(", \"limit\": 10}"))
        .unwrap();

    tracker.flush();

    assert_eq!(
        tracker.parts().last().unwrap(),
        &ToolCallStreamPart::ToolCall {
            tool_call_id: "call_1".into(),
            tool_name: "search".into(),
            input: "{\"query\": \"test\"}, \"limit\": 10}".into(),
            provider_metadata: None,
        }
    );

    let tool_call_count = tracker
        .parts()
        .iter()
        .filter(|p| matches!(p, ToolCallStreamPart::ToolCall { .. }))
        .count();
    assert_eq!(tool_call_count, 1);
}

#[test]
fn multiple_concurrent_tool_calls() {
    // TS: "should handle multiple concurrent tool calls"
    let mut tracker = new_tracker();

    tracker
        .process_delta(
            &d().index(0)
                .id("call_1")
                .tool_type("function")
                .function_name("get_weather")
                .arguments(""),
        )
        .unwrap();
    tracker
        .process_delta(
            &d().index(1)
                .id("call_2")
                .tool_type("function")
                .function_name("get_time")
                .arguments(""),
        )
        .unwrap();

    assert_eq!(
        tracker.parts(),
        &[
            ToolCallStreamPart::ToolInputStart {
                id: "call_1".into(),
                tool_name: "get_weather".into(),
            },
            ToolCallStreamPart::ToolInputStart {
                id: "call_2".into(),
                tool_name: "get_time".into(),
            },
        ]
    );
}

#[test]
fn skip_deltas_for_already_finished_tool_calls() {
    // TS: "should skip deltas for already-finished tool calls"
    let mut tracker = new_tracker();

    tracker
        .process_delta(
            &d().index(0)
                .id("call_1")
                .tool_type("function")
                .function_name("fn")
                .arguments("{}"),
        )
        .unwrap();

    // Finalize via flush.
    tracker.flush();
    tracker.clear_parts();

    // Late delta for the same tool call.
    tracker
        .process_delta(&d().index(0).arguments("extra"))
        .unwrap();

    assert!(tracker.parts().is_empty());
}

#[test]
fn skip_delta_emission_when_arguments_null() {
    // TS: "should skip delta emission when arguments are null".
    // `arguments: null` maps to `None` (omitted on the builder); behaviorally
    // identical to a `function` with no arguments field.
    let mut tracker = new_tracker();

    tracker
        .process_delta(
            &d().index(0)
                .id("call_1")
                .tool_type("function")
                .function_name("fn")
                .arguments(""),
        )
        .unwrap();

    tracker.clear_parts();

    // Delta with null arguments (no `.arguments(...)` call -> None).
    tracker.process_delta(&d().index(0)).unwrap();

    assert!(tracker.parts().is_empty());
}

#[test]
fn use_index_fallback_when_index_not_provided() {
    // TS: "should use index fallback when index is not provided"
    let mut tracker = new_tracker();

    tracker
        .process_delta(
            &d().id("call_1")
                .tool_type("function")
                .function_name("fn1")
                .arguments("{}"),
        )
        .unwrap();
    tracker
        .process_delta(
            &d().id("call_2")
                .tool_type("function")
                .function_name("fn2")
                .arguments("{}"),
        )
        .unwrap();

    let starts: Vec<_> = tracker
        .parts()
        .iter()
        .filter(|p| matches!(p, ToolCallStreamPart::ToolInputStart { .. }))
        .cloned()
        .collect();
    assert_eq!(
        starts,
        vec![
            ToolCallStreamPart::ToolInputStart {
                id: "call_1".into(),
                tool_name: "fn1".into(),
            },
            ToolCallStreamPart::ToolInputStart {
                id: "call_2".into(),
                tool_name: "fn2".into(),
            },
        ]
    );
}

#[test]
fn throw_when_id_missing() {
    // TS: "should throw when id is missing"
    let mut tracker = new_tracker();
    let err = tracker
        .process_delta(&d().index(0).tool_type("function").function_name("fn"))
        .unwrap_err();
    assert_eq!(err, TrackerError::MissingId);
    assert_eq!(err.to_string(), "Expected 'id' to be a string.");
}

#[test]
fn throw_when_function_name_missing() {
    // TS: "should throw when function.name is missing".
    // `function: {}` (TS) maps to `function: None` here; both yield a missing
    // name and the same error.
    let mut tracker = new_tracker();
    let err = tracker
        .process_delta(&d().index(0).id("call_1").tool_type("function"))
        .unwrap_err();
    assert_eq!(err, TrackerError::MissingFunctionName);
    assert_eq!(err.to_string(), "Expected 'function.name' to be a string.");
}

// == typeValidation ========================================================

#[test]
fn no_validate_type_with_type_validation_none() {
    // TS: "should not validate type with typeValidation: none"
    let mut tracker = new_tracker().with_type_validation(TypeValidation::None);
    // Should not throw even with a non-function type.
    let result = tracker.process_delta(
        &d().index(0)
            .id("call_1")
            .tool_type("custom")
            .function_name("fn")
            .arguments(""),
    );
    assert!(result.is_ok());
}

#[test]
fn validate_type_when_present_with_type_validation_if_present() {
    // TS: "should validate type when present with typeValidation: if-present"
    let mut tracker = new_tracker().with_type_validation(TypeValidation::IfPresent);

    // Should throw for a non-function type.
    let err = tracker
        .process_delta(
            &d().index(0)
                .id("call_1")
                .tool_type("custom")
                .function_name("fn")
                .arguments(""),
        )
        .unwrap_err();
    assert_eq!(err, TrackerError::InvalidType);
    assert_eq!(err.to_string(), "Expected 'function' type.");

    // Should not throw when type is null (absent). The first delta errored
    // before creating any tool call, so index 0 is still new.
    let result =
        tracker.process_delta(&d().index(0).id("call_1").function_name("fn").arguments(""));
    assert!(result.is_ok());
}

#[test]
fn require_function_type_with_type_validation_required() {
    // TS: "should require function type with typeValidation: required"
    let mut tracker = new_tracker().with_type_validation(TypeValidation::Required);

    // Should throw when type is null/undefined.
    let err = tracker
        .process_delta(&d().index(0).id("call_1").function_name("fn").arguments(""))
        .unwrap_err();
    assert_eq!(err, TrackerError::InvalidType);

    // Should not throw for 'function' type.
    let result = tracker.process_delta(
        &d().index(0)
            .id("call_1")
            .tool_type("function")
            .function_name("fn")
            .arguments(""),
    );
    assert!(result.is_ok());
}

// == flush =================================================================

#[test]
fn finalize_unfinished_tool_calls_on_flush() {
    // TS: "should finalize unfinished tool calls on flush"
    let mut tracker = new_tracker();

    // Start a tool call but don't complete it.
    tracker
        .process_delta(
            &d().index(0)
                .id("call_1")
                .tool_type("function")
                .function_name("fn")
                .arguments("{\"key\": \"val"),
        )
        .unwrap();

    tracker.clear_parts();

    // Flush should finalize.
    tracker.flush();

    assert_eq!(
        tracker.parts(),
        &[
            ToolCallStreamPart::ToolInputEnd {
                id: "call_1".into(),
            },
            ToolCallStreamPart::ToolCall {
                tool_call_id: "call_1".into(),
                tool_name: "fn".into(),
                input: "{\"key\": \"val".into(),
                provider_metadata: None,
            },
        ]
    );
}

#[test]
fn not_refinalize_already_finished_tool_calls() {
    // TS: "should not re-finalize already finished tool calls"
    let mut tracker = new_tracker();

    tracker
        .process_delta(
            &d().index(0)
                .id("call_1")
                .tool_type("function")
                .function_name("fn")
                .arguments("{}"),
        )
        .unwrap();

    // First flush finalizes the tool call.
    tracker.flush();
    tracker.clear_parts();

    tracker.flush();

    // No events should be emitted since the tool call was already finished.
    assert!(tracker.parts().is_empty());
}

// == metadata =============================================================

#[test]
fn extract_and_include_provider_metadata_in_tool_call_events() {
    // TS: "should extract and include provider metadata in tool-call events"
    let mut tracker = StreamingToolCallTracker::<Value>::new()
        .with_extract_metadata(|delta| {
            delta
                .extra
                .get("google")?
                .get("thought_signature")?
                .as_str()
                .map(|s| json!({ "thoughtSignature": s }))
        })
        .with_build_provider_metadata(|metadata| {
            metadata
                .as_ref()
                .and_then(|m| m.get("thoughtSignature"))
                .and_then(|s| s.as_str())
                .map(|s| json!({ "google": { "thoughtSignature": s } }))
        });

    tracker
        .process_delta(
            &d().index(0)
                .id("call_1")
                .tool_type("function")
                .function_name("fn")
                .arguments("{}")
                .extra(json!({ "google": { "thought_signature": "sig123" } })),
        )
        .unwrap();

    tracker.flush();

    let tool_call = tracker
        .parts()
        .iter()
        .find(|p| matches!(p, ToolCallStreamPart::ToolCall { .. }))
        .unwrap();
    assert_eq!(
        tool_call,
        &ToolCallStreamPart::ToolCall {
            tool_call_id: "call_1".into(),
            tool_name: "fn".into(),
            input: "{}".into(),
            provider_metadata: Some(json!({ "google": { "thoughtSignature": "sig123" } })),
        }
    );
}

#[test]
fn include_provider_metadata_for_unfinished_tool_calls_finalized_in_flush() {
    // TS: "should include provider metadata for unfinished tool calls finalized in flush"
    let mut tracker = StreamingToolCallTracker::<Value>::new()
        .with_extract_metadata(|_| Some(json!({ "custom": { "key": "value" } })))
        .with_build_provider_metadata(|metadata| {
            metadata
                .as_ref()
                .map(|m| json!({ "provider": (*m).clone() }))
        });

    tracker
        .process_delta(
            &d().index(0)
                .id("call_1")
                .tool_type("function")
                .function_name("fn")
                .arguments("{\"incomplete"),
        )
        .unwrap();

    tracker.clear_parts();

    tracker.flush();

    let tool_call = tracker
        .parts()
        .iter()
        .find(|p| matches!(p, ToolCallStreamPart::ToolCall { .. }))
        .unwrap();
    assert_eq!(
        tool_call,
        &ToolCallStreamPart::ToolCall {
            tool_call_id: "call_1".into(),
            tool_name: "fn".into(),
            input: "{\"incomplete".into(),
            provider_metadata: Some(json!({ "provider": { "custom": { "key": "value" } } })),
        }
    );
}

#[test]
fn not_include_provider_metadata_when_build_returns_none() {
    // TS: "should not include providerMetadata when buildToolCallProviderMetadata returns undefined"
    let mut tracker = StreamingToolCallTracker::<Value>::new()
        .with_extract_metadata(|_| None)
        .with_build_provider_metadata(|_| None);

    tracker
        .process_delta(
            &d().index(0)
                .id("call_1")
                .tool_type("function")
                .function_name("fn")
                .arguments("{}"),
        )
        .unwrap();

    tracker.flush();

    let tool_call = tracker
        .parts()
        .iter()
        .find(|p| matches!(p, ToolCallStreamPart::ToolCall { .. }))
        .unwrap();
    assert_eq!(
        tool_call,
        &ToolCallStreamPart::ToolCall {
            tool_call_id: "call_1".into(),
            tool_name: "fn".into(),
            input: "{}".into(),
            provider_metadata: None,
        }
    );
    // Mirrors the TS `not.toHaveProperty('providerMetadata')` assertion: the
    // metadata is explicitly `None`, never `Some`.
    assert!(matches!(
        tool_call,
        ToolCallStreamPart::ToolCall {
            provider_metadata: None,
            ..
        }
    ));
}

// == generateId ===========================================================

#[test]
fn use_custom_generate_id_for_tool_call_ids_when_id_missing_in_fallback() {
    // TS: "should use custom generateId for tool call IDs when id is missing in fallback".
    // The id is present (`call_1`), so the custom generator is NOT used; the
    // original id is kept (mirrors the TS `toolCall.id ?? generateId()` path).
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);

    let mut tracker = StreamingToolCallTracker::<()>::new().with_generate_id(|| {
        CALLS.fetch_add(1, Ordering::SeqCst);
        "custom-id".to_string()
    });

    tracker
        .process_delta(
            &d().index(0)
                .id("call_1")
                .tool_type("function")
                .function_name("fn")
                .arguments("{\"key\": \"val"),
        )
        .unwrap();

    tracker.clear_parts();
    tracker.flush();

    let tool_call = tracker
        .parts()
        .iter()
        .find(|p| matches!(p, ToolCallStreamPart::ToolCall { .. }))
        .unwrap();
    assert_eq!(tool_call_id(tool_call), "call_1");
    // The custom generator must not have been invoked.
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);
}

// == index bound (P1-10) ==================================================

#[test]
fn reject_tool_call_index_above_max_index() {
    // A remote `index` far above the cap must not resize `tool_calls` to a
    // huge vector; it returns `IndexOutOfRange`.
    let mut tracker = new_tracker().with_max_index(4);

    // index == max_index is accepted (boundary).
    tracker
        .process_delta(
            &d().index(4)
                .id("call_4")
                .tool_type("function")
                .function_name("fn")
                .arguments(""),
        )
        .unwrap();

    // index == max_index + 1 is rejected.
    let err = tracker
        .process_delta(
            &d().index(5)
                .id("call_5")
                .tool_type("function")
                .function_name("fn")
                .arguments(""),
        )
        .unwrap_err();
    assert_eq!(err, TrackerError::IndexOutOfRange);
    assert_eq!(err.to_string(), "Tool call index out of range");
}

#[test]
fn default_max_index_rejects_huge_index() {
    // The default cap (1024) rejects an absurd index that would otherwise
    // resize `tool_calls` to index+1 slots.
    let mut tracker = new_tracker();
    let err = tracker
        .process_delta(
            &d().index(1_000_000)
                .id("call_x")
                .tool_type("function")
                .function_name("fn")
                .arguments(""),
        )
        .unwrap_err();
    assert_eq!(err, TrackerError::IndexOutOfRange);
}
