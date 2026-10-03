//! Port of `streaming-tool-call-argument-state.test.ts` from
//! `@ai-sdk/provider-utils`.

use aimux_provider_utils::{StreamingToolCallArgumentState, starts_with_structured_value};

#[test]
fn starts_with_structured_value_true() {
    for value in ["{}", "  {", "[]", "\n["] {
        assert!(starts_with_structured_value(Some(value)), "{value:?}");
    }
}

#[test]
fn starts_with_structured_value_false() {
    // TS covers `undefined` and `null` separately; Rust has one `None`.
    assert!(!starts_with_structured_value(None));
    for value in ["", "   ", "1", "\"value\""] {
        assert!(!starts_with_structured_value(Some(value)), "{value:?}");
    }
}

#[test]
fn tracks_a_structured_value_across_deltas() {
    let mut state = StreamingToolCallArgumentState::new("  {\"value\":");
    assert!(!state.has_complete_structured_value());

    state.append("1}");
    assert!(state.has_complete_structured_value());
}

#[test]
fn tracks_nested_objects_and_arrays() {
    let state = StreamingToolCallArgumentState::new("[{\"value\":{\"items\":[1,2]}}]");
    assert!(state.has_complete_structured_value());
}

#[test]
fn ignores_structural_characters_inside_strings() {
    let state = StreamingToolCallArgumentState::new("{\"value\":\"braces: } ] { [\"}");
    assert!(state.has_complete_structured_value());
}

#[test]
fn handles_escaped_quotes_across_deltas() {
    let mut state = StreamingToolCallArgumentState::new("{\"value\":\"escaped quote: \\\"");
    assert!(!state.has_complete_structured_value());

    state.append(" still in string\"}");
    assert!(state.has_complete_structured_value());
}

#[test]
fn can_begin_after_an_empty_or_whitespace_only_delta() {
    let mut state = StreamingToolCallArgumentState::new("  ");

    state.append("[");
    assert!(!state.has_complete_structured_value());

    state.append("]");
    assert!(state.has_complete_structured_value());
}

#[test]
fn does_not_treat_scalar_arguments_as_a_complete_structured_value() {
    let state = StreamingToolCallArgumentState::new("12");
    assert!(!state.has_complete_structured_value());
}

#[test]
fn does_not_recover_mismatched_structures_as_complete() {
    let mut state = StreamingToolCallArgumentState::new("{\"value\":]");
    state.append("}");
    assert!(!state.has_complete_structured_value());
}
