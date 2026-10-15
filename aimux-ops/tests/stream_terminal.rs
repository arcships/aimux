//! RFC-0039 shared stream scenarios, at the EventStream -> StreamFrame boundary.
//!
//! The mock source emits already-lifted data events, not provider HTTP chunks.
//! `event` is Ok(Value), `terminalError` is Err(OpsError::AiMux), and `eof` is
//! source exhaustion. `serializationFailure` requires a failing serializer at
//! the native framing seam: a serde_json::Value cannot itself fail serialization.
//! Never substitute a host callback exception (CallbackFailure) for this fault.
//!
//! Run a real dispatch or binding driver with:
//! AIMUX_OPS_SCENARIO_DRIVER=/absolute/path/to/driver \
//!   cargo test -p aimux-ops --test stream_terminal run_shared_driver -- --ignored
//!
//! Driver protocol: one process per scenario, JSON on stdin, one JSON observation
//! on stdout, diagnostics on stderr. The input has no `expected` field. The driver
//! must install a scripted mock EventStream in the REAL framing/queue path and
//! execute `controls` in order. It must not implement a second terminal state
//! machine or derive frames from the script itself. Binding drivers must call
//! their native path, not reproduce native framing in the host language.
//!
//! `wait` parks the source until its named gate is released or cancellation.
//! `waitForSourceGate` and `waitForQueueFull` are instrumentation barriers, never
//! sleeps. `repeatEvent` lazily emits count copies. `read` consumes exactly count
//! frames; `drain` consumes to EOF, including another poll after end; `cancel`
//! signals the shared AbortSignal even after success. `closeReader` invokes the
//! real iterator-close hook, then `drainNative` waits for native cleanup without
//! delivering more frames to the closed consumer. `startProducer` starts the
//! bounded bridge without reading. Queue limits apply to data, not terminal.
//!
//! Observation shape is `expected` in the fixtures. `frames` records all native
//! callbacks/frames, `consumerFrames` only consumer deliveries. accepted means
//! successful preflight, NOT a callback frame. preflightError is null otherwise.
//! abortObserved records the real shared abort signal (null in expectations
//! leaves error/success cleanup signaling unspecified). queueFullObserved must
//! come from capacity instrumentation, and consumerClosed from the close hook.
//! sourceStarted records actual source startup; sourceEventsPolled counts Ok
//! data items pulled from the mock, including any polled but undelivered items.
//! Capture any returned post-acceptance error as a driver FAILURE, not another
//! envelope: accepted stream failures are delivered only by end.error.
//!
//! Errors match their stable kind/code/error.name subset; messages and variant
//! fields must still have the envelope shape. Other event data
//! is compared exactly. Extra native cancellation error detail is permitted.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Scenario {
    schema_version: u32,
    name: String,
    description: String,
    boundary: String,
    queue_limits: QueueLimits,
    script: Vec<SourceStep>,
    controls: Vec<Control>,
    fault: Option<Value>,
    expected: Observation,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QueueLimits {
    items: usize,
    bytes: usize,
    max_event_bytes: usize,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
enum SourceStep {
    Event { event: Value },
    RepeatEvent { count: usize, event: Value },
    Wait { gate: String },
    TerminalError { kind: String, message: String },
    SerializationFailure { message: String },
    Eof,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
enum Control {
    Read { count: usize },
    Drain,
    DrainNative,
    Cancel,
    CloseReader,
    StartProducer,
    WaitForQueueFull,
    WaitForSourceGate { gate: String },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Observation {
    accepted: bool,
    preflight_error: Option<Value>,
    frames: Vec<Value>,
    consumer_frames: Vec<Value>,
    abort_observed: Option<bool>,
    queue_full_observed: bool,
    consumer_closed: bool,
    source_started: bool,
    source_events_polled: usize,
}

fn scenarios() -> Vec<(PathBuf, Value, Scenario)> {
    let root = std::env::var_os("AIMUX_OPS_SCENARIOS").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../contract-tests/ops/scenarios"),
        PathBuf::from,
    );
    let mut paths: Vec<_> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("{}: {e}", root.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no scenarios in {}", root.display());
    paths
        .into_iter()
        .map(|path| {
            let raw: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let scenario = serde_json::from_value(raw.clone())
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            (path, raw, scenario)
        })
        .collect()
}

fn validate_envelope(error: &Value, partial: bool) {
    let object = error.as_object().expect("error envelope must be an object");
    assert!(matches!(
        error["kind"].as_str(),
        Some("aimux" | "recording" | "binding" | "protocol")
    ));
    assert!(
        error["code"]
            .as_i64()
            .is_some_and(|code| i32::try_from(code).is_ok())
    );
    assert!(error["error"]["name"].is_string());
    if !partial {
        assert!(
            error["message"].is_string(),
            "missing envelope message: {error}"
        );
    }
    assert!(!object.contains_key("retryable"));
    assert!(!object.contains_key("type"));
    if let Some(retry) = object.get("retryAfterMs") {
        assert!(retry.as_u64().is_some());
    }
}

fn validate_frames(frames: &[Value], terminal: bool, partial: bool) {
    let mut ends = 0;
    for (index, frame) in frames.iter().enumerate() {
        assert_eq!(
            frame["seq"],
            index.to_string(),
            "non-contiguous decimal seq"
        );
        match frame["type"].as_str() {
            Some("event") => {
                assert_eq!(ends, 0, "data after end");
                assert!(frame["event"].is_object());
                assert_eq!(frame.as_object().unwrap().len(), 3);
            }
            Some("end") => {
                ends += 1;
                assert_eq!(index + 1, frames.len(), "end must be last");
                assert!(matches!(
                    frame["outcome"].as_str(),
                    Some("success" | "error" | "cancelled")
                ));
                if frame["outcome"] == "error" {
                    assert!(frame.get("error").is_some(), "error end needs envelope");
                }
                if frame["outcome"] == "success" {
                    assert!(
                        frame.get("error").is_none(),
                        "success end cannot carry error"
                    );
                }
                if let Some(error) = frame.get("error") {
                    validate_envelope(error, partial);
                }
                assert_eq!(
                    frame.as_object().unwrap().len(),
                    if frame.get("error").is_some() { 4 } else { 3 }
                );
            }
            other => panic!("invalid callback frame type: {other:?}"),
        }
    }
    assert_eq!(
        ends,
        usize::from(terminal),
        "accepted native stream needs exactly one end"
    );
}

fn validate_observation(observed: &Observation, partial: bool) {
    assert_eq!(observed.accepted, observed.preflight_error.is_none());
    assert_eq!(observed.source_started, observed.accepted);
    if let Some(error) = &observed.preflight_error {
        validate_envelope(error, partial);
        assert!(observed.frames.is_empty());
        assert!(observed.consumer_frames.is_empty());
        assert_eq!(observed.abort_observed, Some(false));
        assert!(!observed.queue_full_observed);
        assert_eq!(observed.source_events_polled, 0);
    }
    validate_frames(&observed.frames, observed.accepted, partial);
    if observed.consumer_closed {
        assert!(
            observed.abort_observed == Some(true),
            "reader close must reach native abort"
        );
        assert_eq!(observed.frames.last().unwrap()["outcome"], "cancelled");
        assert!(observed.consumer_frames.len() < observed.frames.len());
        assert_eq!(
            observed.consumer_frames,
            observed.frames[..observed.consumer_frames.len()]
        );
        validate_frames(&observed.consumer_frames, false, partial);
    } else {
        assert_eq!(observed.consumer_frames, observed.frames);
    }
}

// Only error envelopes use subset matching; event DTOs remain exact values.
fn error_matches(actual: &Value, expected: &Value) {
    if let Some(expected) = expected.as_object() {
        for (key, value) in expected {
            let actual = actual
                .get(key)
                .unwrap_or_else(|| panic!("missing error field {key}"));
            error_matches(actual, value);
        }
    } else {
        assert_eq!(actual, expected);
    }
}

fn compare_frames(actual: &[Value], expected: &[Value]) {
    assert_eq!(actual.len(), expected.len(), "frame count");
    for (actual, expected) in actual.iter().zip(expected) {
        let mut actual = actual.clone();
        let mut expected = expected.clone();
        let actual_error = actual.as_object_mut().unwrap().remove("error");
        let expected_error = expected.as_object_mut().unwrap().remove("error");
        if let Some(expected) = expected_error {
            error_matches(&actual_error.expect("missing end.error"), &expected);
        } else if expected["outcome"] != "cancelled" {
            assert!(actual_error.is_none());
        }
        assert_eq!(actual, expected);
    }
}

fn compare(actual: &Observation, expected: &Observation) {
    validate_observation(actual, false);
    assert_eq!(actual.accepted, expected.accepted);
    assert_eq!(actual.source_started, expected.source_started);
    assert_eq!(actual.source_events_polled, expected.source_events_polled);
    match (&actual.preflight_error, &expected.preflight_error) {
        (Some(actual), Some(expected)) => error_matches(actual, expected),
        (None, None) => {}
        _ => panic!("preflight result mismatch"),
    }
    assert!(
        actual.abort_observed.is_some(),
        "driver must report actual abort state"
    );
    if let Some(expected) = expected.abort_observed {
        assert_eq!(actual.abort_observed, Some(expected));
    }
    assert_eq!(actual.queue_full_observed, expected.queue_full_observed);
    assert_eq!(actual.consumer_closed, expected.consumer_closed);
    compare_frames(&actual.frames, &expected.frames);
    compare_frames(&actual.consumer_frames, &expected.consumer_frames);
}

fn run_driver(executable: &Path, mut input: Value) -> Observation {
    input.as_object_mut().unwrap().remove("expected");
    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot start {}: {e}", executable.display()));
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let writer = thread::spawn(move || {
        serde_json::to_writer(&mut stdin, &input).unwrap();
        stdin.write_all(b"\n").unwrap();
    });
    let output = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    });
    let diagnostics = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.take(64 * 1024).read_to_end(&mut bytes).unwrap();
        bytes
    });
    let started = Instant::now();
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break (status, false);
        }
        if started.elapsed() > Duration::from_secs(10) {
            child.kill().unwrap();
            break (child.wait().unwrap(), true);
        }
        thread::sleep(Duration::from_millis(10));
    };
    let output = output.join().unwrap();
    let diagnostics = diagnostics.join().unwrap();
    assert!(
        !timed_out,
        "driver timed out: {}",
        String::from_utf8_lossy(&diagnostics)
    );
    writer.join().unwrap();
    assert!(
        status.success(),
        "driver failed ({status}): {}",
        String::from_utf8_lossy(&diagnostics)
    );
    assert!(
        output.len() <= 2 * 1024 * 1024,
        "oversized driver transcript"
    );
    serde_json::from_slice(&output).expect("driver must return one observation JSON object")
}

#[test]
fn shared_vectors_are_well_formed() {
    let mut names = BTreeSet::new();
    for (path, _, scenario) in scenarios() {
        assert_eq!(scenario.schema_version, 1);
        assert_eq!(scenario.boundary, "eventStream");
        assert_eq!(path.file_stem().unwrap(), scenario.name.as_str());
        assert!(names.insert(scenario.name));
        assert!(!scenario.description.is_empty());
        assert_eq!(scenario.queue_limits.items, 256);
        assert_eq!(scenario.queue_limits.bytes, 8 * 1024 * 1024);
        assert_eq!(scenario.queue_limits.max_event_bytes, 1024 * 1024);
        assert!(!scenario.script.is_empty());
        assert!(!scenario.controls.is_empty());
        for step in &scenario.script {
            match step {
                SourceStep::Event { event } => assert!(event["type"].is_string()),
                SourceStep::RepeatEvent { count, event } => {
                    assert!(*count > 0);
                    assert!(event["type"].is_string());
                }
                SourceStep::Wait { gate } => assert!(!gate.is_empty()),
                SourceStep::TerminalError { kind, message } => {
                    assert_eq!(kind, "apiCall");
                    assert!(!message.is_empty());
                }
                SourceStep::SerializationFailure { message } => assert!(!message.is_empty()),
                SourceStep::Eof => {}
            }
        }
        for control in &scenario.controls {
            match control {
                Control::Read { count } => assert!(*count > 0),
                Control::WaitForSourceGate { gate } => assert!(scenario.script.iter().any(
                    |step| matches!(step, SourceStep::Wait { gate: source } if source == gate)
                )),
                _ => {}
            }
        }
        assert_eq!(scenario.fault.is_some(), !scenario.expected.accepted);
        if scenario.expected.accepted {
            let mut events = Vec::new();
            for step in &scenario.script {
                match step {
                    SourceStep::Event { event } => events.push(event.clone()),
                    SourceStep::RepeatEvent { count, event } => {
                        events.extend(std::iter::repeat_n(event.clone(), *count));
                    }
                    SourceStep::TerminalError { .. } | SourceStep::SerializationFailure { .. } => {
                        break;
                    }
                    _ => {}
                }
            }
            let framed: Vec<_> = scenario
                .expected
                .frames
                .iter()
                .filter(|frame| frame["type"] == "event")
                .map(|frame| frame["event"].clone())
                .collect();
            assert_eq!(
                events, framed,
                "fixture must preserve all pre-terminal source events"
            );
            assert_eq!(scenario.expected.source_events_polled, events.len());
        }
        validate_observation(&scenario.expected, true);
    }
    assert_eq!(
        names,
        BTreeSet::from(
            [
                "accepted",
                "preflight-failure",
                "finish",
                "recoverable-error-part",
                "terminal-error",
                "cancel",
                "late-cancel-after-success",
                "reader-close",
                "duplicate-finish",
                "serialization-failure",
                "full-queue-cancel",
            ]
            .map(String::from)
        )
    );
}

#[test]
fn terminal_checker_rejects_regressions() {
    let valid = serde_json::json!([{"type":"end","seq":"0","outcome":"success"}]);
    validate_frames(valid.as_array().unwrap(), true, false);
    for invalid in [
        serde_json::json!([]),
        serde_json::json!([{"type":"end","seq":"0","outcome":"success"}, {"type":"end","seq":"1","outcome":"success"}]),
        serde_json::json!([{"type":"end","seq":"0","outcome":"success"}, {"type":"event","seq":"1","event":{}}]),
        serde_json::json!([{"type":"end","seq":0,"outcome":"success"}]),
        serde_json::json!([{"type":"end","seq":"0","outcome":"error"}]),
        serde_json::json!([{"type":"end","seq":"0","outcome":"success","error":{}}]),
    ] {
        assert!(
            std::panic::catch_unwind(|| validate_frames(invalid.as_array().unwrap(), true, false))
                .is_err()
        );
    }
}

#[test]
fn comparator_rejects_lost_data_and_wrong_outcomes() {
    for (_, _, scenario) in scenarios() {
        if !scenario.expected.accepted {
            continue;
        }
        let mut missing = scenario.expected.frames.clone();
        missing.pop();
        assert!(
            std::panic::catch_unwind(|| compare_frames(&missing, &scenario.expected.frames))
                .is_err()
        );
        let mut wrong = scenario.expected.frames.clone();
        let end = wrong.last_mut().unwrap();
        end["outcome"] = if end["outcome"] == "success" {
            "cancelled"
        } else {
            "success"
        }
        .into();
        assert!(
            std::panic::catch_unwind(|| compare_frames(&wrong, &scenario.expected.frames)).is_err()
        );
    }
}

#[test]
fn comparator_accepts_complete_envelopes_and_rejects_wrong_codes() {
    // Synthetic observations check the comparator, never the runtime.
    for (_, raw, scenario) in scenarios() {
        let mut raw = raw["expected"].clone();
        if raw["abortObserved"].is_null() {
            raw["abortObserved"] = false.into();
        }
        if let Some(error) = raw
            .get_mut("preflightError")
            .filter(|error| !error.is_null())
        {
            error["message"] = "driver-specific diagnostic".into();
        }
        for frames in ["frames", "consumerFrames"] {
            for frame in raw[frames].as_array_mut().unwrap() {
                if let Some(error) = frame.get_mut("error") {
                    error["message"] = "driver-specific diagnostic".into();
                }
            }
        }
        let actual = serde_json::from_value(raw.clone()).unwrap();
        compare(&actual, &scenario.expected);
        if !raw["preflightError"].is_null() {
            raw["preflightError"]["code"] = 399.into();
        } else if scenario
            .expected
            .frames
            .last()
            .is_some_and(|end| end.get("error").is_some())
        {
            for frames in ["frames", "consumerFrames"] {
                raw[frames].as_array_mut().unwrap().last_mut().unwrap()["error"]["code"] =
                    399.into();
            }
        } else {
            continue;
        }
        let wrong = serde_json::from_value(raw).unwrap();
        assert!(std::panic::catch_unwind(|| compare(&wrong, &scenario.expected)).is_err());
    }
}

#[test]
#[ignore = "requires a real dispatch or binding driver in AIMUX_OPS_SCENARIO_DRIVER"]
fn run_shared_driver() {
    let executable = std::env::var_os("AIMUX_OPS_SCENARIO_DRIVER")
        .expect("set AIMUX_OPS_SCENARIO_DRIVER to a real dispatch or binding driver");
    for (path, raw, scenario) in scenarios() {
        eprintln!("scenario: {}", path.display());
        let actual = run_driver(Path::new(&executable), raw);
        compare(&actual, &scenario.expected);
    }
}

#[path = "support/native_scenarios.rs"]
mod native_scenarios;

#[tokio::test]
async fn run_native_shared_vectors() {
    for (path, mut input, scenario) in scenarios() {
        input.as_object_mut().unwrap().remove("expected");
        eprintln!("native scenario: {}", path.display());
        let actual = tokio::time::timeout(Duration::from_secs(8), native_scenarios::observe(input))
            .await
            .expect("native scenario timed out");
        compare(&actual, &scenario.expected);
    }
}
