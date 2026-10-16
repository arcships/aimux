//! `streaming-tool-call-tracker.test.ts` from `@ai-sdk/provider-utils`, case
//! for case, and the T3 differential fixture recorded from the same tracker,
//! run through the chat streams that use [`ChatToolCalls`](super::ChatToolCalls).
//!
//! A case drives a stream's `Transformer` synchronously: every
//! `tool_calls[i]` delta is one chunk, `flush` ends the stream, and only the
//! tool parts are kept. Which stream runs a case follows the tracker options
//! upstream tested:
//!
//! - no `type` check (the tracker default): Mistral, whose `index` is
//!   optional and whose ids come from an injected generator;
//! - `'function'` when present (`'if-present'`): OpenAI;
//! - `'function'` required (`'required'`): Groq;
//! - `extractMetadata`: OpenAI-compatible, whose thought signature is the
//!   metadata of the call a delta starts.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::provider_namespace;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::RawToolCall;
use aimux_core::types::ProviderMetadata;
use aimux_provider_utils::{TransformStreamController, Transformer};

use super::Controller;

type GenerateId = Arc<dyn Fn() -> String + Send + Sync>;

const MISSING_NAME: &str = "Expected 'function.name' to be a string.";
const INVALID_TYPE: &str = "Expected 'function' type.";

/// The tracker's four events, projected out of [`StreamPart`] so the
/// assertions read like the upstream suite.
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

/// The tool part `part` is, if it is one. Any other part a chat stream emits
/// around tool calls (stream start, response metadata, finish) is dropped;
/// an error part means the case's chunk did not fit the vendor's schema.
fn project(part: StreamPart) -> Option<Part> {
    Some(match part {
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
        StreamPart::Error { error } => panic!("the stream rejected a chunk: {error:?}"),
        part @ (StreamPart::ToolInputStart { .. }
        | StreamPart::ToolInputDelta { .. }
        | StreamPart::ToolInputEnd { .. }
        | StreamPart::ToolCall(_)) => panic!("unexpected tool part: {part:?}"),
        _ => return None,
    })
}

/// A chat stream with its chunk type fixed, so the vendors can share one
/// harness.
trait ChatStream {
    fn chunk(&mut self, chunk: Value, controller: &mut Controller);
    fn end(self: Box<Self>, controller: &mut Controller);
}

impl<T> ChatStream for T
where
    T: Transformer<Input = Result<Value, AiMuxError>, Output = StreamPart>,
{
    fn chunk(&mut self, chunk: Value, controller: &mut Controller) {
        self.transform(Ok(chunk), controller);
    }

    fn end(self: Box<Self>, controller: &mut Controller) {
        (*self).flush(controller);
    }
}

/// A chat stream plus the tool parts it has emitted so far (the TS
/// `createCollector`).
struct Harness {
    /// `None` once flushed.
    stream: Option<Box<dyn ChatStream>>,
    parts: Vec<Part>,
}

impl Harness {
    /// The tracker defaults: no `type` check; the default generator's ids
    /// are `tool-call`, `tool-call-1`, …
    fn new() -> Self {
        Self::with_ids(|| "tool-call".to_string())
    }

    /// No `type` check, ids from `generate_id`.
    fn with_ids(generate_id: impl Fn() -> String + Send + Sync + 'static) -> Self {
        Self::of(crate::mistral::chat_test_stream(Arc::new(generate_id)))
    }

    /// `type` must be `'function'` when present.
    fn openai() -> Self {
        Self::of(crate::openai::model::chat_test_stream(Arc::new(|| {
            "tool-call".to_string()
        })))
    }

    /// `type` must be `'function'` on the delta that starts a call.
    fn groq() -> Self {
        Self::of(crate::groq::chat_test_stream(Arc::new(|| {
            "tool-call".to_string()
        })))
    }

    /// Thought signatures become the metadata of the call; the provider's
    /// metadata key is `google`.
    fn compatible() -> Self {
        Self::of(crate::openai_compatible::chat::chat_test_stream(
            "google",
            Arc::new(|| "tool-call".to_string()),
        ))
    }

    fn of(stream: impl ChatStream + 'static) -> Self {
        Self {
            stream: Some(Box::new(stream)),
            parts: Vec::new(),
        }
    }

    /// Feed one `tool_calls[i]` entry as a chunk. An `Err` is the message of
    /// the `InvalidResponseData` error the stream ended with. Every delta
    /// gets a fresh controller, so, like the upstream tracker, the stream
    /// can be fed again after an error.
    fn delta(&mut self, tool_call: Value) -> Result<(), String> {
        let mut controller = TransformStreamController::new();
        let stream = self.stream.as_mut().expect("delta after flush");
        stream.chunk(
            json!({ "choices": [{ "delta": { "tool_calls": [tool_call] } }] }),
            &mut controller,
        );
        self.collect(&mut controller)
    }

    fn flush(&mut self) {
        let mut controller = TransformStreamController::new();
        let stream = self.stream.take().expect("flushed twice");
        stream.end(&mut controller);
        // The OpenAI-compatible stream reports a missing finish reason as an
        // error part; it is not a tool part.
        let parts: Vec<StreamPart> = controller.drain().filter_map(Result::ok).collect();
        self.parts.extend(
            parts
                .into_iter()
                .filter(|part| !matches!(part, StreamPart::Error { .. }))
                .filter_map(project),
        );
    }

    fn collect(&mut self, controller: &mut Controller) -> Result<(), String> {
        let mut result = Ok(());
        for item in controller.drain() {
            match item {
                Ok(part) => self.parts.extend(project(part)),
                Err(AiMuxError::InvalidResponseData(message)) => result = Err(message),
                Err(error) => panic!("unexpected stream error: {error:?}"),
            }
        }
        assert_eq!(result.is_err(), controller.is_closed());
        result
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

/// A `tool_calls[i]` entry with an explicit `function` object (as every TS
/// test passes one). `None` leaves a field out.
fn delta(
    index: Option<usize>,
    id: Option<&str>,
    ty: Option<&str>,
    name: Option<&str>,
    arguments: Option<&str>,
) -> Value {
    let mut function = serde_json::Map::new();
    if let Some(name) = name {
        function.insert("name".into(), json!(name));
    }
    if let Some(arguments) = arguments {
        function.insert("arguments".into(), json!(arguments));
    }
    let mut tool_call = json!({ "function": function });
    if let Some(index) = index {
        tool_call["index"] = json!(index);
    }
    if let Some(id) = id {
        tool_call["id"] = json!(id);
    }
    if let Some(ty) = ty {
        tool_call["type"] = json!(ty);
    }
    tool_call
}

/// Full call start: index, id, `type: 'function'`, name and arguments.
fn start(index: usize, id: &str, name: &str, arguments: &str) -> Value {
    delta(
        Some(index),
        Some(id),
        Some("function"),
        Some(name),
        Some(arguments),
    )
}

/// Continuation carrying only arguments (plus an optional index).
fn cont(index: Option<usize>, arguments: &str) -> Value {
    delta(index, None, None, None, Some(arguments))
}

/// `tool_call` carrying a Gemini thought signature (`extra_content`).
fn signed(mut tool_call: Value, signature: &str) -> Value {
    tool_call["extra_content"] = json!({ "google": { "thought_signature": signature } });
    tool_call
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

/// Deterministic id generator over `ids` (the last one repeats) and a call
/// counter.
fn counting_generator(
    ids: &'static [&'static str],
) -> (
    impl Fn() -> String + Send + Sync + 'static,
    Arc<AtomicUsize>,
) {
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

    /// Added in 5.0.55: `it.each` "repeat their ids" / "omit their ids"
    /// (streaming-tool-call-tracker.test.ts:248-302).
    #[test]
    fn keeps_same_name_calls_with_empty_opening_arguments_distinct() {
        for (first_continuation_id, second_continuation_id) in
            [(Some("call_1"), Some("call_2")), (None, None)]
        {
            let mut h = Harness::new();

            h.delta(start(0, "call_1", "get_weather", "")).unwrap();
            h.delta(delta(
                Some(0),
                first_continuation_id,
                None,
                None,
                Some("{\"city\":\"Berlin\"}"),
            ))
            .unwrap();
            h.delta(start(0, "call_2", "get_weather", "")).unwrap();
            h.delta(delta(
                Some(0),
                second_continuation_id,
                None,
                None,
                Some("{\"city\":\"Paris\"}"),
            ))
            .unwrap();
            h.flush();

            assert_eq!(
                h.tool_calls(),
                vec![
                    tc("call_1", "get_weather", "{\"city\":\"Berlin\"}"),
                    tc("call_2", "get_weather", "{\"city\":\"Paris\"}"),
                ],
                "continuation ids {first_continuation_id:?}, {second_continuation_id:?}"
            );
        }
    }

    /// Added in 5.0.55: `it.each([undefined, null, '', ' \n'])`
    /// (streaming-tool-call-tracker.test.ts:304-341). Rust has one `None`
    /// for the wire's absent and `null` arguments.
    #[test]
    fn retains_an_incomplete_call_when_its_id_changes_on_a_named_empty_fragment() {
        for arguments in [None, Some(""), Some(" \n")] {
            let mut h = Harness::new();

            h.delta(delta(
                Some(0),
                Some("call_1"),
                None,
                Some("get_weather"),
                Some("{\"city\":"),
            ))
            .unwrap();
            h.delta(delta(
                Some(0),
                Some("alias"),
                None,
                Some("get_weather"),
                arguments,
            ))
            .unwrap();
            h.delta(delta(
                Some(0),
                Some("alias"),
                None,
                None,
                Some("\"Berlin\"}"),
            ))
            .unwrap();
            h.flush();

            let input = format!("{{\"city\":{}\"Berlin\"}}", arguments.unwrap_or(""));
            assert_eq!(
                h.tool_calls(),
                vec![tc("call_1", "get_weather", &input)],
                "arguments {arguments:?}"
            );
            let starts: Vec<&Part> = h
                .parts
                .iter()
                .filter(|part| matches!(part, Part::ToolInputStart { .. }))
                .collect();
            assert_eq!(
                starts,
                vec![&input_start("call_1", "get_weather")],
                "arguments {arguments:?}"
            );
        }
    }

    /// Added in 5.0.55 (streaming-tool-call-tracker.test.ts:343-381).
    #[test]
    fn uses_an_unlabeled_continuation_for_the_only_call_without_complete_structured_arguments() {
        let mut h = Harness::new();

        h.delta(start(0, "call_1", "get_weather", "")).unwrap();
        h.delta(cont(None, "{\"city\":\"Berlin\"}")).unwrap();
        h.delta(start(1, "call_2", "get_weather", "")).unwrap();
        h.delta(cont(None, "{\"city\":\"Paris\"}")).unwrap();
        h.flush();

        assert_eq!(
            h.tool_calls(),
            vec![
                tc("call_1", "get_weather", "{\"city\":\"Berlin\"}"),
                tc("call_2", "get_weather", "{\"city\":\"Paris\"}"),
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
        let mut h = Harness::with_ids(|| "generated-id".to_string());

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

        assert_eq!(result, Err(MISSING_NAME.to_string()));
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
        let mut h = Harness::with_ids(generate);

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
            let mut h = Harness::with_ids(|| "generated-id".to_string());

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
            let mut h = Harness::with_ids(|| "generated-id".to_string());

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
        let mut h = Harness::with_ids(generate);

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
        let mut h = Harness::with_ids(|| "generated-id".to_string());

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
        let mut h = Harness::with_ids(generate);

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
        let mut h = Harness::with_ids(generate);

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
        let mut h = Harness::with_ids(|| "generated-id".to_string());

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
        let mut h = Harness::groq();

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

        assert_eq!(result, Err(MISSING_NAME.to_string()));
    }
}

mod type_validation {
    use super::*;

    fn custom_type() -> Value {
        delta(
            Some(0),
            Some("call_1"),
            Some("custom"),
            Some("fn"),
            Some(""),
        )
    }

    fn no_type() -> Value {
        delta(Some(0), Some("call_1"), None, Some("fn"), Some(""))
    }

    #[test]
    fn does_not_validate_type_with_none() {
        let mut h = Harness::new();

        assert_eq!(h.delta(custom_type()), Ok(()));
    }

    #[test]
    fn validates_type_when_present_with_if_present() {
        let mut h = Harness::openai();

        assert_eq!(h.delta(custom_type()), Err(INVALID_TYPE.to_string()));

        // A missing type is accepted.
        assert_eq!(h.delta(no_type()), Ok(()));
    }

    #[test]
    fn requires_function_type_with_required() {
        let mut h = Harness::groq();

        assert_eq!(h.delta(no_type()), Err(INVALID_TYPE.to_string()));

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
}

mod metadata {
    use super::*;

    // The upstream `extractMetadata` / `buildToolCallProviderMetadata` hooks
    // are the OpenAI-compatible stream's thought signatures here: the
    // signature of the delta that starts a call is the call's metadata.

    #[test]
    fn includes_the_starting_delta_metadata_in_the_tool_call() {
        let mut h = Harness::compatible();

        h.delta(signed(start(0, "call_1", "fn", "{\"incomplete"), "sig123"))
            .unwrap();
        h.flush();

        assert_eq!(
            h.parts.last(),
            Some(&Part::ToolCall {
                tool_call_id: "call_1".into(),
                tool_name: "fn".into(),
                input: "{\"incomplete".into(),
                provider_metadata: Some(
                    provider_namespace("google", json!({ "thoughtSignature": "sig123" })).unwrap(),
                ),
            })
        );
    }

    #[test]
    fn ignores_metadata_on_continuations() {
        let mut h = Harness::compatible();

        h.delta(start(0, "call_1", "fn", "{")).unwrap();
        h.delta(signed(cont(Some(0), "}"), "sig123")).unwrap();
        h.flush();

        assert_eq!(h.parts.last(), Some(&tool_call("call_1", "fn", "{}")));
    }
}

mod generate_id {
    use super::*;

    #[test]
    fn keeps_the_wire_id_when_present() {
        let mut h = Harness::with_ids(|| "custom-id".to_string());

        h.delta(start(0, "call_1", "fn", "{\"key\": \"val"))
            .unwrap();
        h.clear();

        h.flush();

        let ids: Vec<String> = h.tool_calls().into_iter().map(|c| c.0).collect();
        assert_eq!(ids, vec!["call_1"]);
    }
}

/// The T3 differential fixture:
/// `fixtures/aisdk/provider-utils/t3-unattributable-deltas.json` holds the
/// deltas of each case plus what the `StreamingToolCallTracker` of the
/// `@ai-sdk/provider-utils` version pinned in `fixtures/aisdk/VERSIONS.json`
/// (5.0.55) emitted for them with the case's `typeValidation`
/// (`'if-present'`: openai chat; `'required'`: groq; `'none'`:
/// openai-compatible), recorded by `scripts/gen-t3-tracker-fixture.mjs`.
/// Each case asserts that a chat stream with the same `type` rule matches
/// per delta, on the error, and on flush.
mod t3_unattributable_delta_parity {
    use super::*;

    const FIXTURE: &str =
        include_str!("../../../fixtures/aisdk/provider-utils/t3-unattributable-deltas.json");

    fn to_aisdk(part: &Part) -> Value {
        match part {
            Part::ToolInputStart { id, tool_name } => {
                json!({"type": "tool-input-start", "id": id, "toolName": tool_name})
            }
            Part::ToolInputDelta { id, delta } => {
                json!({"type": "tool-input-delta", "id": id, "delta": delta})
            }
            Part::ToolInputEnd { id } => json!({"type": "tool-input-end", "id": id}),
            Part::ToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            } => json!({
                "type": "tool-call", "toolCallId": tool_call_id, "toolName": tool_name, "input": input
            }),
        }
    }

    /// The stream that applies the case's `typeValidation` to its deltas.
    ///
    /// The vendor schemas are stricter than the tracker: OpenAI and Groq
    /// require `index`, and Groq rejects a `type` other than `'function'`
    /// while parsing. A case those schemas would reject runs on a stream
    /// whose rule gives the same verdict for its deltas: Mistral (no check)
    /// for `'if-present'` deltas without an index, none of which has a
    /// non-`'function'` type; OpenAI for `'required'` deltas with another
    /// `type`, which both rules reject.
    fn harness(case: &Value, generate_id: impl Fn() -> String + Send + Sync + 'static) -> Harness {
        let deltas = case["deltas"].as_array().unwrap();
        let all_indexed = deltas.iter().all(|d| d.get("index").is_some());
        let other_type = deltas
            .iter()
            .any(|d| d.get("type").is_some_and(|t| t != "function"));
        let generate_id: GenerateId = Arc::new(generate_id);
        let stream: Box<dyn ChatStream> = match case["typeValidation"].as_str().unwrap() {
            "if-present" if all_indexed => {
                Box::new(crate::openai::model::chat_test_stream(generate_id))
            }
            "if-present" => {
                assert!(!other_type);
                Box::new(crate::mistral::chat_test_stream(generate_id))
            }
            "required" if other_type => {
                Box::new(crate::openai::model::chat_test_stream(generate_id))
            }
            "required" => Box::new(crate::groq::chat_test_stream(generate_id)),
            "none" => Box::new(crate::mistral::chat_test_stream(generate_id)),
            other => panic!("unknown typeValidation {other}"),
        };
        Harness {
            stream: Some(stream),
            parts: Vec::new(),
        }
    }

    fn run_case(case: &Value) {
        let name = case["name"].as_str().unwrap();
        // Ids are generated `gen-0`, `gen-1`, ... like the recording script.
        let next_id = AtomicUsize::new(0);
        let mut h = harness(case, move || {
            format!("gen-{}", next_id.fetch_add(1, Ordering::SeqCst))
        });
        let mut steps = Vec::new();
        let mut error = Value::Null;
        for tool_call in case["deltas"].as_array().unwrap() {
            h.clear();
            let result = h.delta(tool_call.clone());
            steps.push(json!({"parts": h.parts.iter().map(to_aisdk).collect::<Vec<_>>()}));
            if let Err(message) = result {
                error = json!({"name": "AI_InvalidResponseDataError", "message": message});
                break;
            }
        }
        let flush = if error.is_null() {
            h.clear();
            h.flush();
            Value::Array(h.parts.iter().map(to_aisdk).collect())
        } else {
            Value::Null
        };
        let actual = json!({"steps": steps, "error": error, "flush": flush});
        assert_eq!(actual, case["aisdk"], "{name} ({})", case["tracker"]);
    }

    fn case(name: &str) -> Value {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("missing fixture case {name}"))
            .clone()
    }

    /// Every recorded case, including the typeValidation `required` (groq)
    /// and `none` (openai-compatible) ones and the interleaved ones.
    #[test]
    fn every_fixture_case_matches_the_aisdk_tracker() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            run_case(case);
        }
    }

    /// streaming-tool-call-tracker.ts:144-146, :305-308.
    #[test]
    fn ambiguous_delta_across_open_calls_is_dropped() {
        run_case(&case("ambiguous_delta_across_open_calls_is_dropped"));
    }

    /// streaming-tool-call-tracker.ts:153-155.
    #[test]
    fn blank_name_matching_no_call_is_dropped() {
        run_case(&case("blank_name_matching_no_call_is_dropped"));
    }

    /// streaming-tool-call-tracker.ts:305-308, :384-389.
    #[test]
    fn nameless_delta_without_open_call_is_invalid_response_data() {
        run_case(&case(
            "nameless_delta_without_open_call_is_invalid_response_data",
        ));
    }

    /// streaming-tool-call-tracker.ts:375-381.
    #[test]
    fn non_function_type_is_invalid_response_data() {
        run_case(&case("non_function_type_is_invalid_response_data"));
    }

    /// The case 5.0.55 changed: an index-only continuation after the index
    /// is reused goes to the only call there without complete structured
    /// arguments (streaming-tool-call-tracker.ts:291-298, :345-347); 5.0.53
    /// dropped it.
    #[test]
    fn index_only_continuation_goes_to_the_only_incomplete_call() {
        run_case(&case("index_reuse_with_new_id"));
    }
}
