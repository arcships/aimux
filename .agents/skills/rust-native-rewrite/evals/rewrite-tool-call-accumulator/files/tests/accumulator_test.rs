use std::sync::{Arc, Mutex};

use tool_call_accumulator::{Accumulator, Delta, StreamPart};

fn delta(index: i64, id: Option<&str>, name: Option<&str>, arguments: Option<&str>) -> Delta {
    Delta {
        index,
        id: id.map(str::to_string),
        name: name.map(str::to_string),
        arguments: arguments.map(str::to_string),
    }
}

fn json(parts: &[StreamPart]) -> String {
    serde_json::to_string(parts).unwrap()
}

fn new_accumulator() -> (Accumulator, Arc<Mutex<Vec<String>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    (Accumulator::new(Arc::clone(&log)), log)
}

#[test]
fn single_call_in_one_delta() {
    let (mut acc, log) = new_accumulator();
    let parts = acc
        .push(
            &delta(
                0,
                Some("call_1"),
                Some("weather"),
                Some(r#"{"city":"Paris"}"#),
            ),
            true,
        )
        .unwrap();
    assert_eq!(
        json(&parts),
        r#"[{"type":"tool-input-start","id":"call_1","toolName":"weather"},{"type":"tool-input-delta","id":"call_1","delta":"{\"city\":\"Paris\"}"},{"type":"tool-call","id":"call_1","toolName":"weather","input":"{\"city\":\"Paris\"}"}]"#
    );
    assert_eq!(
        *log.lock().unwrap(),
        ["tool-input-start", "tool-input-delta", "tool-call"]
    );
}

#[test]
fn arguments_split_across_deltas() {
    let (mut acc, _) = new_accumulator();
    acc.push(
        &delta(0, Some("call_1"), Some("weather"), Some(r#"{"ci"#)),
        true,
    )
    .unwrap();
    let parts = acc
        .push(&delta(0, None, None, Some(r#"ty":"Paris"}"#)), true)
        .unwrap();
    assert_eq!(
        json(&parts),
        r#"[{"type":"tool-input-delta","id":"call_1","delta":"ty\":\"Paris\"}"},{"type":"tool-call","id":"call_1","toolName":"weather","input":"{\"city\":\"Paris\"}"}]"#
    );
}

#[test]
fn without_deltas() {
    let (mut acc, _) = new_accumulator();
    let first = acc
        .push(
            &delta(0, Some("call_1"), Some("weather"), Some(r#"{"ci"#)),
            false,
        )
        .unwrap();
    let second = acc
        .push(&delta(0, None, None, Some(r#"ty":"Paris"}"#)), false)
        .unwrap();
    assert_eq!(
        json(&first),
        r#"[{"type":"tool-input-start","id":"call_1","toolName":"weather"}]"#
    );
    assert_eq!(
        json(&second),
        r#"[{"type":"tool-call","id":"call_1","toolName":"weather","input":"{\"city\":\"Paris\"}"}]"#
    );
}

#[test]
fn index_minus_one_appends_to_latest_call() {
    let (mut acc, _) = new_accumulator();
    acc.push(&delta(0, Some("call_1"), Some("a"), None), true)
        .unwrap();
    acc.push(&delta(1, Some("call_2"), Some("b"), None), true)
        .unwrap();
    let parts = acc.push(&delta(-1, None, None, Some("{}")), true).unwrap();
    assert_eq!(parts[0].id.as_deref(), Some("call_2"));
}

#[test]
fn missing_id_is_an_error() {
    let (mut acc, _) = new_accumulator();
    let err = acc
        .push(&delta(0, None, Some("weather"), None), true)
        .unwrap_err();
    assert_eq!(err.to_string(), "Expected 'id' to be a string.");
}

#[test]
fn missing_name_is_an_error() {
    let (mut acc, _) = new_accumulator();
    let err = acc
        .push(&delta(0, Some("call_1"), None, None), true)
        .unwrap_err();
    assert_eq!(err.to_string(), "Expected 'function.name' to be a string.");
}

#[test]
fn deltas_after_completion_are_ignored() {
    let (mut acc, _) = new_accumulator();
    acc.push(&delta(0, Some("call_1"), Some("a"), Some("{}")), true)
        .unwrap();
    let parts = acc.push(&delta(0, None, None, Some("x")), true).unwrap();
    assert!(parts.is_empty());
}

#[test]
fn finish_flushes_open_calls() {
    let (mut acc, log) = new_accumulator();
    acc.push(&delta(0, Some("call_1"), Some("a"), Some(r#"{"x":"#)), true)
        .unwrap();
    acc.push(&delta(1, Some("call_2"), Some("b"), None), true)
        .unwrap();
    let parts = acc.finish(&Some("tool-calls".to_string()));
    assert_eq!(
        json(&parts),
        r#"[{"type":"tool-call","id":"call_1","toolName":"a","input":"{\"x\":"},{"type":"tool-call","id":"call_2","toolName":"b","input":"{}"},{"type":"finish","finishReason":"tool-calls"}]"#
    );
    assert_eq!(
        log.lock().unwrap().last().map(String::as_str),
        Some("finish")
    );
}

#[test]
fn finish_without_reason() {
    let (mut acc, _) = new_accumulator();
    assert_eq!(json(&acc.finish(&None)), r#"[{"type":"finish"}]"#);
}

#[test]
fn nothing_after_finish() {
    let (mut acc, _) = new_accumulator();
    acc.finish(&None);
    assert!(acc.finish(&None).is_empty());
    assert!(
        acc.push(&delta(0, Some("c"), Some("a"), Some("{}")), true)
            .unwrap()
            .is_empty()
    );
}
