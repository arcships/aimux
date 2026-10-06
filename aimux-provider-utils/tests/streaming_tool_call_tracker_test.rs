//! Port of `streaming-tool-call-tracker.test.ts` from
//! `@ai-sdk/provider-utils`, case for case.
//!
//! Tracker parts are projected to a local `Part` enum so the assertions stay
//! byte-for-byte comparable with the upstream expectations.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_core::shared::provider_namespace;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::RawToolCall;
use aimux_core::types::ProviderMetadata;
use aimux_provider_utils::{
    StreamingToolCallDelta, StreamingToolCallFunction, StreamingToolCallTracker, TrackerError,
    TypeValidation,
};
use serde_json::{Value, json};

/// The TS tracker's four events, projected out of [`StreamPart`] so the
/// assertions below read like the upstream suite.
#[derive(Debug, Clone, PartialEq)]
#[allow(
    clippy::enum_variant_names,
    reason = "named after the upstream stream parts"
)]
enum Part {
    ToolInputStart {
        id: String,
        tool_name: String,
    },
    ToolInputDelta {
        id: String,
        delta: String,
    },
    ToolInputEnd {
        id: String,
    },
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        /// The raw argument text.
        input: String,
        provider_metadata: Option<ProviderMetadata>,
    },
}

fn project(part: StreamPart) -> Part {
    match part {
        StreamPart::ToolInputStart {
            id,
            tool_name,
            provider_executed: None,
            dynamic: None,
            title: None,
            provider_metadata: None,
        } => Part::ToolInputStart { id, tool_name },
        StreamPart::ToolInputDelta {
            id,
            delta,
            provider_metadata: None,
        } => Part::ToolInputDelta { id, delta },
        StreamPart::ToolInputEnd {
            id,
            provider_metadata: None,
        } => Part::ToolInputEnd { id },
        StreamPart::ToolCall(RawToolCall {
            tool_call_id,
            tool_name,
            input,
            provider_executed: None,
            dynamic: None,
            provider_metadata,
        }) => Part::ToolCall {
            tool_call_id,
            tool_name,
            input,
            provider_metadata,
        },
        other => panic!("tracker emitted an unexpected part: {other:?}"),
    }
}

/// Tracker plus the parts it has emitted so far (the TS `createCollector`).
struct Harness {
    tracker: StreamingToolCallTracker,
    parts: Vec<Part>,
}

impl Harness {
    fn new() -> Self {
        Self::with(StreamingToolCallTracker::new())
    }

    fn with(tracker: StreamingToolCallTracker) -> Self {
        Self {
            tracker,
            parts: Vec::new(),
        }
    }

    fn delta(&mut self, delta: StreamingToolCallDelta) -> Result<(), TrackerError> {
        let parts = self.tracker.process_delta(&delta)?;
        self.parts.extend(parts.into_iter().map(project));
        Ok(())
    }

    fn flush(&mut self) {
        let parts = self.tracker.flush();
        self.parts.extend(parts.into_iter().map(project));
    }

    fn clear(&mut self) {
        self.parts.clear();
    }

    /// `(id, name, input)` of every emitted `tool-call`, in order.
    fn tool_calls(&self) -> Vec<(String, String, String)> {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::ToolCall {
                    tool_call_id,
                    tool_name,
                    input,
                    ..
                } => Some((tool_call_id.clone(), tool_name.clone(), input.clone())),
                _ => None,
            })
            .collect()
    }
}

/// A delta with an explicit `function` object (as every TS test passes one).
fn delta(
    index: Option<usize>,
    id: Option<&str>,
    ty: Option<&str>,
    name: Option<&str>,
    arguments: Option<&str>,
) -> StreamingToolCallDelta {
    StreamingToolCallDelta {
        index,
        id: id.map(str::to_string),
        r#type: ty.map(str::to_string),
        function: Some(StreamingToolCallFunction {
            name: name.map(str::to_string),
            arguments: arguments.map(str::to_string),
        }),
        extra: Value::Null,
    }
}

/// Full call start: index, id, `type: 'function'`, name and arguments.
fn start(index: usize, id: &str, name: &str, arguments: &str) -> StreamingToolCallDelta {
    delta(
        Some(index),
        Some(id),
        Some("function"),
        Some(name),
        Some(arguments),
    )
}

/// Continuation carrying only arguments (plus an optional index).
fn cont(index: Option<usize>, arguments: &str) -> StreamingToolCallDelta {
    delta(index, None, None, None, Some(arguments))
}

fn tc(id: &str, name: &str, input: &str) -> (String, String, String) {
    (id.into(), name.into(), input.into())
}

fn input_start(id: &str, tool_name: &str) -> Part {
    Part::ToolInputStart {
        id: id.into(),
        tool_name: tool_name.into(),
    }
}

fn input_delta(id: &str, delta: &str) -> Part {
    Part::ToolInputDelta {
        id: id.into(),
        delta: delta.into(),
    }
}

fn input_end(id: &str) -> Part {
    Part::ToolInputEnd { id: id.into() }
}

fn tool_call(id: &str, name: &str, input: &str) -> Part {
    Part::ToolCall {
        tool_call_id: id.into(),
        tool_name: name.into(),
        input: input.into(),
        provider_metadata: None,
    }
}

/// Deterministic id generator: `prefix-1`, `prefix-2`, … and a call counter.
fn counting_generator(ids: &'static [&'static str]) -> (impl Fn() -> String, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    (
        move || {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            ids[n.min(ids.len() - 1)].to_string()
        },
        calls,
    )
}

mod process_delta {
    use super::*;

    #[test]
    fn single_tool_call_accumulated_across_multiple_deltas() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "get_weather", "{\"ci")).unwrap();
        assert_eq!(
            h.parts,
            vec![
                input_start("call_1", "get_weather"),
                input_delta("call_1", "{\"ci"),
            ]
        );
        h.clear();

        h.delta(cont(Some(0), "ty\": \"San")).unwrap();
        assert_eq!(h.parts, vec![input_delta("call_1", "ty\": \"San")]);
        h.clear();

        // Completing the JSON must not finalize before flush: a parsable
        // buffer can still be the prefix of longer arguments.
        h.delta(cont(Some(0), " Francisco\"}")).unwrap();
        assert_eq!(h.parts, vec![input_delta("call_1", " Francisco\"}")]);
        h.clear();

        h.flush();
        assert_eq!(
            h.parts,
            vec![
                input_end("call_1"),
                tool_call("call_1", "get_weather", "{\"city\": \"San Francisco\"}"),
            ]
        );
    }

    #[test]
    fn full_tool_call_in_a_single_chunk() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "get_weather", "{\"city\": \"London\"}"))
            .unwrap();
        assert_eq!(
            h.parts,
            vec![
                input_start("call_1", "get_weather"),
                input_delta("call_1", "{\"city\": \"London\"}"),
            ]
        );
        h.clear();

        h.flush();
        assert_eq!(
            h.parts,
            vec![
                input_end("call_1"),
                tool_call("call_1", "get_weather", "{\"city\": \"London\"}"),
            ]
        );
    }

    #[test]
    fn does_not_finalize_when_argument_prefix_is_parsable_json() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "search", "{\"query\": \"test\"}"))
            .unwrap();
        assert_eq!(
            h.parts,
            vec![
                input_start("call_1", "search"),
                input_delta("call_1", "{\"query\": \"test\"}"),
            ]
        );

        h.delta(cont(Some(0), ", \"limit\": 10}")).unwrap();
        h.flush();

        assert_eq!(
            h.parts.last(),
            Some(&tool_call(
                "call_1",
                "search",
                "{\"query\": \"test\"}, \"limit\": 10}"
            ))
        );
        assert_eq!(h.tool_calls().len(), 1);
    }

    #[test]
    fn multiple_concurrent_tool_calls() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "get_weather", "")).unwrap();
        h.delta(start(1, "call_2", "get_time", "")).unwrap();

        assert_eq!(
            h.parts,
            vec![
                input_start("call_1", "get_weather"),
                input_start("call_2", "get_time"),
            ]
        );
    }

    #[test]
    fn non_zero_and_non_contiguous_indexes() {
        let mut h = Harness::new();

        h.delta(start(1, "call_1", "fn1", "{\"value\":1}")).unwrap();
        h.delta(start(3, "call_2", "fn2", "{\"value\":2}")).unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![
                tc("call_1", "fn1", "{\"value\":1}"),
                tc("call_2", "fn2", "{\"value\":2}"),
            ]
        );
    }

    #[test]
    fn keeps_distinct_tool_calls_that_reuse_an_index() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "fn", "{\"value\":1}")).unwrap();
        h.delta(start(0, "call_2", "fn", "{\"value\":2}")).unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![
                tc("call_1", "fn", "{\"value\":1}"),
                tc("call_2", "fn", "{\"value\":2}"),
            ]
        );
    }

    #[test]
    fn continues_latest_call_when_index_is_omitted_after_starting_at() {
        for index in [None, Some(7)] {
            let mut h = Harness::new();

            h.delta(delta(
                index,
                Some("call_1"),
                Some("function"),
                Some("fn"),
                Some("{\"val"),
            ))
            .unwrap();
            h.delta(cont(None, "ue\":1}")).unwrap();
            h.flush();

            assert_eq!(
                h.tool_calls(),
                vec![tc("call_1", "fn", "{\"value\":1}")],
                "index {index:?}"
            );
        }
    }

    #[test]
    fn uses_the_index_when_continuation_ids_are_empty() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "fn", "{\"val")).unwrap();
        h.delta(delta(
            Some(0),
            Some(""),
            Some("function"),
            None,
            Some("ue\":1}"),
        ))
        .unwrap();
        h.flush();

        assert_eq!(h.tool_calls(), vec![tc("call_1", "fn", "{\"value\":1}")]);
    }

    #[test]
    fn skips_deltas_for_already_finished_tool_calls() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "fn", "{}")).unwrap();
        h.flush();
        h.clear();

        h.delta(cont(Some(0), "extra")).unwrap();
        assert_eq!(h.parts, vec![]);
    }

    #[test]
    fn skips_delta_emission_when_arguments_are_null() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "fn", "")).unwrap();
        h.clear();

        h.delta(delta(Some(0), None, None, None, None)).unwrap();
        assert_eq!(h.parts, vec![]);
    }

    #[test]
    fn uses_index_fallback_when_index_is_not_provided() {
        let mut h = Harness::new();

        h.delta(delta(
            None,
            Some("call_1"),
            Some("function"),
            Some("fn1"),
            Some("{}"),
        ))
        .unwrap();
        h.delta(delta(
            None,
            Some("call_2"),
            Some("function"),
            Some("fn2"),
            Some("{}"),
        ))
        .unwrap();

        let starts: Vec<&Part> = h
            .parts
            .iter()
            .filter(|p| matches!(p, Part::ToolInputStart { .. }))
            .collect();
        assert_eq!(
            starts,
            vec![&input_start("call_1", "fn1"), &input_start("call_2", "fn2")]
        );
    }

    #[test]
    fn generates_an_id_when_id_is_missing() {
        let mut h = Harness::with(
            StreamingToolCallTracker::new().with_generate_id(|| "generated-id".to_string()),
        );

        h.delta(delta(
            Some(0),
            None,
            Some("function"),
            Some("fn"),
            Some("{}"),
        ))
        .unwrap();
        h.flush();

        assert_eq!(h.tool_calls(), vec![tc("generated-id", "fn", "{}")]);
    }

    #[test]
    fn errors_when_function_name_is_missing() {
        // TS covers `name: undefined` and `name: null`; both are `None`.
        let mut h = Harness::new();

        let result = h.delta(delta(Some(0), Some("call_1"), Some("function"), None, None));

        assert_eq!(result, Err(TrackerError::MissingFunctionName));
        assert_eq!(
            TrackerError::MissingFunctionName.to_string(),
            "Expected 'function.name' to be a string."
        );
    }

    #[test]
    fn ignores_a_blank_function_name_without_preventing_prior_calls_from_finalizing() {
        for name in ["", "   "] {
            let mut h = Harness::new();

            h.delta(start(0, "call_1", "valid_tool", "{\"value\":1}"))
                .unwrap();
            h.delta(start(1, "call_2", name, "{\"value\":2}")).unwrap();
            h.flush();

            assert_eq!(
                h.tool_calls(),
                vec![tc("call_1", "valid_tool", "{\"value\":1}")],
                "name {name:?}"
            );
        }
    }

    #[test]
    fn retains_continuation_arguments_for_a_blank_name() {
        // (blank name, continuation id, continuation index)
        let cases = [
            ("", Some("call_1"), None), // blank name with a matching id
            ("   ", None, Some(0)),     // whitespace name, matching index
        ];
        for (name, id, index) in cases {
            let mut h = Harness::new();

            h.delta(start(0, "call_1", "read_file", "{\"pa")).unwrap();
            h.delta(delta(index, id, None, Some(name), Some("th\":\"a\"}")))
                .unwrap();
            h.flush();

            assert_eq!(
                h.tool_calls(),
                vec![tc("call_1", "read_file", "{\"path\":\"a\"}")],
                "name {name:?}"
            );
        }
    }

    #[test]
    fn keeps_id_less_calls_distinct_when_an_index_is_reused_and_type_is_omitted() {
        let (generate, _) = counting_generator(&["generated-1", "generated-2", "generated-3"]);
        let mut h = Harness::with(StreamingToolCallTracker::new().with_generate_id(generate));

        h.delta(delta(
            Some(0),
            None,
            None,
            Some("read_file"),
            Some("{\"path\":\"p0\"}"),
        ))
        .unwrap();
        h.delta(delta(
            Some(0),
            None,
            None,
            Some("write_file"),
            Some("{\"path\":\"p1\"}"),
        ))
        .unwrap();
        h.delta(delta(
            Some(0),
            None,
            None,
            Some("read_file"),
            Some("{\"path\":\"p2\"}"),
        ))
        .unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![
                tc("generated-1", "read_file", "{\"path\":\"p0\"}"),
                tc("generated-2", "write_file", "{\"path\":\"p1\"}"),
                tc("generated-3", "read_file", "{\"path\":\"p2\"}"),
            ]
        );
    }

    #[test]
    fn keeps_complete_same_name_calls_distinct_with_reused_index() {
        for id in [None, Some("dup")] {
            let mut h = Harness::with(
                StreamingToolCallTracker::new().with_generate_id(|| "generated-id".to_string()),
            );

            for value in ["{\"value\":1}", "{\"value\":2}"] {
                h.delta(delta(
                    Some(0),
                    id,
                    Some("function"),
                    Some("same_tool"),
                    Some(value),
                ))
                .unwrap();
            }
            h.flush();

            let calls = h.tool_calls();
            let inputs: Vec<&str> = calls.iter().map(|c| c.2.as_str()).collect();
            assert_eq!(inputs, vec!["{\"value\":1}", "{\"value\":2}"], "id {id:?}");
            let ids: std::collections::HashSet<&str> = calls.iter().map(|c| c.0.as_str()).collect();
            assert_eq!(ids.len(), 2, "id {id:?}");
        }
    }

    #[test]
    fn keeps_a_partial_same_name_call_distinct_with_reused_index() {
        for id in [None, Some("dup")] {
            let mut h = Harness::with(
                StreamingToolCallTracker::new().with_generate_id(|| "generated-id".to_string()),
            );

            for value in ["{\"value\":1}", "{\"value\":"] {
                h.delta(delta(
                    Some(0),
                    id,
                    Some("function"),
                    Some("same_tool"),
                    Some(value),
                ))
                .unwrap();
            }
            h.flush();

            let calls = h.tool_calls();
            let inputs: Vec<&str> = calls.iter().map(|c| c.2.as_str()).collect();
            assert_eq!(inputs, vec!["{\"value\":1}", "{\"value\":"], "id {id:?}");
            let ids: std::collections::HashSet<&str> = calls.iter().map(|c| c.0.as_str()).collect();
            assert_eq!(ids.len(), 2, "id {id:?}");
        }
    }

    #[test]
    fn keeps_interleaved_same_name_calls_with_distinct_ids_and_reused_index_separate() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "same_tool", "{\"value\":"))
            .unwrap();
        h.delta(start(0, "call_2", "same_tool", "{\"value\":2}"))
            .unwrap();
        h.delta(delta(Some(0), Some("call_1"), None, None, Some("1}")))
            .unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![
                tc("call_1", "same_tool", "{\"value\":1}"),
                tc("call_2", "same_tool", "{\"value\":2}"),
            ]
        );
    }

    #[test]
    fn ignores_an_index_only_continuation_after_the_index_is_reused() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "first", "{\"value\":1}"))
            .unwrap();
        h.delta(start(0, "call_2", "second", "{\"value\":2}"))
            .unwrap();
        h.delta(cont(Some(0), "{\"unattributed\":true}")).unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![
                tc("call_1", "first", "{\"value\":1}"),
                tc("call_2", "second", "{\"value\":2}"),
            ]
        );
    }

    #[test]
    fn uses_the_index_when_continuation_ids_are_blank() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "read_file", "{\"pa")).unwrap();
        h.delta(delta(Some(0), Some("   "), None, None, Some("th\":\"a\"}")))
            .unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![tc("call_1", "read_file", "{\"path\":\"a\"}")]
        );
    }

    #[test]
    fn generates_unique_ids_for_blank_and_repeated_ids() {
        let (generate, _) = counting_generator(&["generated-1", "generated-2"]);
        let mut h = Harness::with(StreamingToolCallTracker::new().with_generate_id(generate));

        h.delta(start(0, "", "read_file", "{}")).unwrap();
        h.delta(start(1, "dup", "read_file", "{}")).unwrap();
        h.delta(start(2, "dup", "write_file", "{}")).unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![
                tc("generated-1", "read_file", "{}"),
                tc("dup", "read_file", "{}"),
                tc("generated-2", "write_file", "{}"),
            ]
        );
    }

    #[test]
    fn keeps_same_name_calls_with_repeated_ids_and_distinct_indices_separate() {
        let mut h = Harness::with(
            StreamingToolCallTracker::new().with_generate_id(|| "generated-id".to_string()),
        );

        h.delta(start(0, "dup", "same_tool", "{\"value\":0}"))
            .unwrap();
        h.delta(start(1, "dup", "same_tool", "{\"value\":1}"))
            .unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![
                tc("dup", "same_tool", "{\"value\":0}"),
                tc("generated-id", "same_tool", "{\"value\":1}"),
            ]
        );
    }

    #[test]
    fn preserves_nonblank_ids_and_function_names_exactly() {
        let mut h = Harness::new();

        h.delta(start(0, " spaced ", " same_tool ", "{\"value\":"))
            .unwrap();
        h.delta(delta(Some(0), Some(" spaced "), None, None, Some("0}")))
            .unwrap();
        h.delta(start(1, "spaced", " same_tool ", "{\"value\":1}"))
            .unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![
                tc(" spaced ", " same_tool ", "{\"value\":0}"),
                tc("spaced", " same_tool ", "{\"value\":1}"),
            ]
        );
    }

    #[test]
    fn creates_bounded_unique_ids_when_generate_id_returns_duplicates() {
        let (generate, calls) = counting_generator(&["generated-id"]);
        let mut h = Harness::with(StreamingToolCallTracker::new().with_generate_id(generate));

        for (index, name) in [(0, "first"), (1, "second"), (2, "third")] {
            h.delta(delta(
                Some(index),
                None,
                Some("function"),
                Some(name),
                Some("{}"),
            ))
            .unwrap();
        }
        h.flush();

        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let ids: Vec<String> = h.tool_calls().into_iter().map(|c| c.0).collect();
        assert_eq!(
            ids,
            vec!["generated-id", "generated-id-1", "generated-id-2"]
        );
    }

    #[test]
    fn creates_usable_ids_when_generate_id_returns_blank_values() {
        let (generate, calls) = counting_generator(&["   "]);
        let mut h = Harness::with(StreamingToolCallTracker::new().with_generate_id(generate));

        for (index, name) in [(0, "first"), (1, "second")] {
            h.delta(delta(
                Some(index),
                None,
                Some("function"),
                Some(name),
                Some("{}"),
            ))
            .unwrap();
        }
        h.flush();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let ids: Vec<String> = h.tool_calls().into_iter().map(|c| c.0).collect();
        assert_eq!(ids, vec!["tool-call", "tool-call-1"]);
    }

    #[test]
    fn ignores_unattributable_deltas_when_multiple_calls_are_active() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "read_file", "{\"path\":\"a\"}"))
            .unwrap();
        h.delta(start(1, "call_2", "write_file", "{\"path\":\"b\"}"))
            .unwrap();
        h.delta(cont(None, "{\"unattributed\":true}")).unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![
                tc("call_1", "read_file", "{\"path\":\"a\"}"),
                tc("call_2", "write_file", "{\"path\":\"b\"}"),
            ]
        );
    }

    #[test]
    fn ignores_an_ambiguous_continuation_for_a_repeated_id() {
        let mut h = Harness::with(
            StreamingToolCallTracker::new().with_generate_id(|| "generated-id".to_string()),
        );

        h.delta(start(0, "dup", "read_file", "{\"path\":\"a\"}"))
            .unwrap();
        h.delta(start(1, "dup", "write_file", "{\"path\":\"b\"}"))
            .unwrap();
        h.delta(delta(
            None,
            Some("dup"),
            None,
            None,
            Some("{\"unattributed\":true}"),
        ))
        .unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![
                tc("dup", "read_file", "{\"path\":\"a\"}"),
                tc("generated-id", "write_file", "{\"path\":\"b\"}"),
            ]
        );
    }

    #[test]
    fn uses_a_matching_name_and_index_for_an_id_less_continuation() {
        let mut h = Harness::with(
            StreamingToolCallTracker::new().with_type_validation(TypeValidation::Required),
        );

        h.delta(start(0, "call_1", "read_file", "{\"pa")).unwrap();
        h.delta(delta(
            Some(0),
            None,
            None,
            Some("read_file"),
            Some("th\":\"a\"}"),
        ))
        .unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![tc("call_1", "read_file", "{\"path\":\"a\"}")]
        );
    }

    #[test]
    fn uses_the_index_when_a_continuation_has_an_unexpected_id() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "read_file", "{\"pa")).unwrap();
        h.delta(delta(
            Some(0),
            Some("unexpected"),
            None,
            None,
            Some("th\":\"a\"}"),
        ))
        .unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![tc("call_1", "read_file", "{\"path\":\"a\"}")]
        );
    }

    #[test]
    fn continues_a_call_when_its_id_changes_but_index_and_name_match() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "read_file", "{\"pa")).unwrap();
        h.delta(start(0, "unexpected", "read_file", "th\":\"a\"}"))
            .unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![tc("call_1", "read_file", "{\"path\":\"a\"}")]
        );
    }

    #[test]
    fn continues_a_call_when_all_labels_repeat_after_a_parsable_argument_prefix() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "calculate", "1")).unwrap();
        h.delta(start(0, "call_1", "calculate", "2")).unwrap();
        h.flush();

        assert_eq!(h.tool_calls(), vec![tc("call_1", "calculate", "12")]);
    }

    #[test]
    fn continues_a_structured_argument_when_repeated_labels_precede_a_nested_object() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "calculate", "{\"value\":"))
            .unwrap();
        h.delta(start(0, "call_1", "calculate", "{\"nested\":true}}"))
            .unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![tc("call_1", "calculate", "{\"value\":{\"nested\":true}}")]
        );
    }

    #[test]
    fn emits_tool_calls_in_index_order() {
        let mut h = Harness::new();

        h.delta(start(1, "call_1", "second", "{}")).unwrap();
        h.delta(start(0, "call_0", "first", "{}")).unwrap();
        h.flush();

        let names: Vec<String> = h.tool_calls().into_iter().map(|c| c.1).collect();
        assert_eq!(names, vec!["first", "second"]);
    }

    #[test]
    fn preserves_insertion_order_when_calls_mix_present_and_omitted_indices() {
        let mut h = Harness::new();

        h.delta(delta(
            None,
            Some("call_without_index"),
            Some("function"),
            Some("first"),
            Some("{}"),
        ))
        .unwrap();
        h.delta(start(0, "call_with_index", "second", "{}"))
            .unwrap();
        h.flush();

        let names: Vec<String> = h.tool_calls().into_iter().map(|c| c.1).collect();
        assert_eq!(names, vec!["first", "second"]);
    }

    #[test]
    fn errors_when_function_name_is_missing_from_a_new_call() {
        let mut h = Harness::new();

        // `function: {}` — a function object with neither name nor arguments.
        let result = h.delta(delta(Some(0), Some("call_1"), Some("function"), None, None));

        assert_eq!(result, Err(TrackerError::MissingFunctionName));
    }
}

mod type_validation {
    use super::*;

    fn custom_type() -> StreamingToolCallDelta {
        delta(
            Some(0),
            Some("call_1"),
            Some("custom"),
            Some("fn"),
            Some(""),
        )
    }

    fn no_type() -> StreamingToolCallDelta {
        delta(Some(0), Some("call_1"), None, Some("fn"), Some(""))
    }

    #[test]
    fn does_not_validate_type_with_none() {
        let mut h = Harness::with(
            StreamingToolCallTracker::new().with_type_validation(TypeValidation::None),
        );

        assert_eq!(h.delta(custom_type()), Ok(()));
    }

    #[test]
    fn validates_type_when_present_with_if_present() {
        let mut h = Harness::with(
            StreamingToolCallTracker::new().with_type_validation(TypeValidation::IfPresent),
        );

        assert_eq!(h.delta(custom_type()), Err(TrackerError::InvalidType));
        assert_eq!(
            TrackerError::InvalidType.to_string(),
            "Expected 'function' type."
        );

        // A missing type is accepted.
        assert_eq!(h.delta(no_type()), Ok(()));
    }

    #[test]
    fn requires_function_type_with_required() {
        let mut h = Harness::with(
            StreamingToolCallTracker::new().with_type_validation(TypeValidation::Required),
        );

        assert_eq!(h.delta(no_type()), Err(TrackerError::InvalidType));

        assert_eq!(h.delta(start(0, "call_1", "fn", "")), Ok(()));
    }
}

mod flush {
    use super::*;

    #[test]
    fn finalizes_unfinished_tool_calls() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "fn", "{\"key\": \"val"))
            .unwrap();
        h.clear();

        h.flush();

        assert_eq!(
            h.parts,
            vec![
                input_end("call_1"),
                tool_call("call_1", "fn", "{\"key\": \"val"),
            ]
        );
    }

    #[test]
    fn does_not_re_finalize_already_finished_tool_calls() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "fn", "{}")).unwrap();
        h.flush();
        h.clear();

        h.flush();

        assert_eq!(h.parts, vec![]);
    }
}

mod metadata {
    use super::*;

    fn google_tracker() -> StreamingToolCallTracker {
        StreamingToolCallTracker::new()
            .with_extract_metadata(|delta| {
                delta.extra["extra_content"]["google"]["thought_signature"]
                    .as_str()
                    .map(|sig| json!({ "thoughtSignature": sig }))
            })
            .with_build_provider_metadata(|metadata| {
                metadata.and_then(|m| m.get("thoughtSignature")).map(|sig| {
                    provider_namespace("google", json!({ "thoughtSignature": sig })).unwrap()
                })
            })
    }

    #[test]
    fn extracts_and_includes_provider_metadata_in_tool_call_parts() {
        let mut h = Harness::with(google_tracker());

        h.delta(
            start(0, "call_1", "fn", "{}")
                .extra(json!({ "extra_content": { "google": { "thought_signature": "sig123" } } })),
        )
        .unwrap();
        h.flush();

        let tool_call = h.parts.iter().find(|p| matches!(p, Part::ToolCall { .. }));
        assert_eq!(
            tool_call,
            Some(&Part::ToolCall {
                tool_call_id: "call_1".into(),
                tool_name: "fn".into(),
                input: "{}".into(),
                provider_metadata: Some(
                    provider_namespace("google", json!({ "thoughtSignature": "sig123" })).unwrap()
                ),
            })
        );
    }

    #[test]
    fn includes_provider_metadata_for_unfinished_tool_calls_finalized_in_flush() {
        let mut h = Harness::with(
            StreamingToolCallTracker::new()
                .with_extract_metadata(|_| Some(json!({ "custom": { "key": "value" } })))
                .with_build_provider_metadata(|metadata| {
                    metadata.map(|m| provider_namespace("provider", m.clone()).unwrap())
                }),
        );

        h.delta(start(0, "call_1", "fn", "{\"incomplete")).unwrap();
        h.clear();

        h.flush();

        assert_eq!(
            h.parts.last(),
            Some(&Part::ToolCall {
                tool_call_id: "call_1".into(),
                tool_name: "fn".into(),
                input: "{\"incomplete".into(),
                provider_metadata: Some(
                    provider_namespace("provider", json!({ "custom": { "key": "value" } }))
                        .unwrap()
                ),
            })
        );
    }

    #[test]
    fn omits_provider_metadata_when_the_builder_returns_none() {
        let mut h = Harness::with(
            StreamingToolCallTracker::new()
                .with_extract_metadata(|_| None)
                .with_build_provider_metadata(|_| None),
        );

        h.delta(start(0, "call_1", "fn", "{}")).unwrap();
        h.flush();

        let tool_call = h.parts.iter().find(|p| matches!(p, Part::ToolCall { .. }));
        assert_eq!(tool_call, Some(&super::tool_call("call_1", "fn", "{}")));
    }
}

mod generate_id {
    use super::*;

    #[test]
    fn keeps_the_wire_id_when_present() {
        let mut h = Harness::with(
            StreamingToolCallTracker::new().with_generate_id(|| "custom-id".to_string()),
        );

        h.delta(start(0, "call_1", "fn", "{\"key\": \"val"))
            .unwrap();
        h.clear();

        h.flush();

        let ids: Vec<String> = h.tool_calls().into_iter().map(|c| c.0).collect();
        assert_eq!(ids, vec!["call_1"]);
    }
}
