//! Recordings written by master (aimux 0.5, recording schema 2) load in this
//! version; what it cannot replay from them fails at replay time, saying why.
//!
//! Fixtures, both written by master's own `JsonlRecorder`:
//! - `fixtures/master_language_recordings.jsonl`: two OpenAI chat calls
//!   recorded on 2026-10-13 by master a8e3a996 (`OpenAIProvider` chat model
//!   `gpt-4o` against a local mock server, explicit key): "What is the capital
//!   of France?" with temperature 0, then "Capital of Italy?" with the
//!   instructions "Answer in one word.", which master records as a system
//!   message whose content is a part array. Written by a scratch test that is
//!   not part of the repository; the file is unchanged.
//! - `fixtures/master_decision_recordings.jsonl`: master's
//!   `aimux-providers/tests/fixtures/jev_systemone_live.jsonl` (two live Jev
//!   decisions, 2026-10-06), copied byte for byte.

use aimux_core::decision_model::{DecideOptions, decide};
use aimux_core::error::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, generate_text};
use aimux_core::recording::{OutcomeStatus, RECORDING_SCHEMA, Recording, RecordingOperation};
use aimux_core::replay::{
    MockDecisionReplayModel, MockReplayModel, replay_decision_with_model, replay_with_model,
};

const LANGUAGE: &str = include_str!("fixtures/master_language_recordings.jsonl");
const DECISION: &str = include_str!("fixtures/master_decision_recordings.jsonl");

fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn parse(lines: &str) -> Vec<Recording> {
    lines
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Every field master writes loads: its configuration snapshot is ignored,
/// the identity this version adds (`provider_id`) is empty, and the wire
/// exchange and outcome are kept.
#[test]
fn master_language_recordings_load() {
    assert_eq!(RECORDING_SCHEMA, 2);
    let recordings = parse(LANGUAGE);
    assert_eq!(recordings.len(), 2);
    for rec in &recordings {
        assert_eq!(rec.schema, 2);
        assert_eq!(rec.input.operation, RecordingOperation::LanguageModel);
        assert_eq!(rec.provider.provider_id, "");
        assert_eq!(rec.provider.provider, "openai");
        assert_eq!(rec.provider.model_id, "gpt-4o");
        assert!(rec.complete && rec.transport_closed);
        assert_eq!(rec.outcome.status, OutcomeStatus::Success);
        assert_eq!(rec.exchanges.len(), 1);
        let request = &rec.exchanges[0].request;
        assert!(request.url.ends_with("/chat/completions"));
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name == "Authorization" && value == "[REDACTED]")
        );
    }

    // The first prompt (one user text message) reads; the second has a
    // system message in master's part-array form, so it loads as empty.
    assert_eq!(recordings[0].input.prompt.len(), 1);
    assert!(recordings[0].input.unreadable_prompt().is_none());
    assert!(recordings[1].input.prompt.is_empty());
    assert!(recordings[1].input.unreadable_prompt().is_some());

    // Round trip: written back as schema 2 with this version's fields.
    let written = serde_json::to_value(&recordings[0]).unwrap();
    assert_eq!(written["schema"], 2);
    assert!(written["provider"].get("base_url").is_none());
    serde_json::from_value::<Recording>(written).unwrap();

    let mock = MockReplayModel::from_jsonl(&fixture("master_language_recordings.jsonl")).unwrap();
    assert_eq!(mock.recordings().len(), 2);
}

/// Offline replay of a master recording whose prompt this version reads
/// returns the recorded response.
#[tokio::test]
async fn a_master_recording_replays_offline() {
    let mock = MockReplayModel::from_jsonl(&fixture("master_language_recordings.jsonl")).unwrap();
    let result = generate_text(
        &mock,
        "What is the capital of France?",
        GenerateTextOptions {
            temperature: Some(0.0),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(result.text(), "Paris.");
}

/// What this version cannot replay from a master recording fails when it is
/// replayed, not when it is loaded, and the error says the recording is from
/// aimux 0.5 or earlier.
#[tokio::test]
async fn what_cannot_be_replayed_fails_at_replay_time() {
    let recordings = parse(LANGUAGE);
    let mock = MockReplayModel::from_jsonl(&fixture("master_language_recordings.jsonl")).unwrap();

    // The prompt with a system message never matches offline.
    let err = generate_text(
        &mock,
        "Capital of Italy?",
        GenerateTextOptions {
            instructions: Some("Answer in one word.".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("aimux 0.5 or earlier"), "{err}");

    // Request replay refuses the unreadable prompt ...
    let err = replay_with_model(&recordings[1], &mock, None)
        .await
        .unwrap_err();
    assert!(matches!(err, AiMuxError::InvalidArgument { .. }), "{err}");
    assert!(err.to_string().contains("recorded prompt"), "{err}");

    // ... and master's call options (`tool_choice: "auto"`), naming the writer.
    let err = replay_with_model(&recordings[0], &mock, None)
        .await
        .unwrap_err();
    assert!(matches!(err, AiMuxError::JsonParse { .. }), "{err}");
    assert!(err.to_string().contains("aimux 0.5 or earlier"), "{err}");
}

/// Master wrote every timeout key, an unset one as `null`
/// (`"step_ms": null`). Such a key carries nothing: request replay drops it
/// before the timeout object, which rejects unknown keys, is read.
#[tokio::test]
async fn null_timeout_keys_as_master_wrote_them_still_replay() {
    let mut recording = parse(LANGUAGE).remove(0);
    let mock = MockReplayModel::new("openai", "gpt-4o", vec![recording.clone()]);
    // The first master call with this version's keys, and the timeout object
    // as master wrote one with nothing set.
    recording.input.options = serde_json::json!({
        "prompt": recording.input.prompt,
        "temperature": 0.0,
        "toolChoice": { "type": "auto" },
        "timeout": {
            "total_ms": null,
            "step_ms": null,
            "first_chunk_ms": null,
            "chunk_ms": null,
        },
    });
    let result = replay_with_model(&recording, &mock, None).await.unwrap();
    assert_eq!(result.text(), "Paris.");
}

/// Master's decision recordings load. Their options and results use the
/// pre-V4 decision shapes, so replaying them fails at replay time and says
/// they were written by aimux 0.5 or earlier.
#[tokio::test]
async fn master_decision_recordings_load_and_explain_at_replay() {
    let recordings = parse(DECISION);
    assert_eq!(recordings.len(), 2);
    for rec in &recordings {
        assert_eq!(rec.schema, 2);
        assert_eq!(rec.input.operation, RecordingOperation::Decision);
        assert_eq!(rec.provider.provider_id, "");
        assert_eq!(rec.provider.provider, "jev");
        assert!(rec.outcome.decision_result.is_some());
        assert_eq!(rec.exchanges[0].response.as_ref().unwrap().status, 200);
    }

    let mock = MockDecisionReplayModel::new("jev", "jev-latest", recordings.clone()).unwrap();
    let err = replay_decision_with_model(&recordings[0], &mock)
        .await
        .unwrap_err();
    assert!(matches!(err, AiMuxError::JsonParse { .. }), "{err}");
    assert!(err.to_string().contains("aimux 0.5 or earlier"), "{err}");

    // The same call in this version's shape (the V4 Jev fixture) finds no
    // master recording to match, and the error says why.
    let v4: Recording = serde_json::from_str(
        include_str!("../../aimux-providers/tests/fixtures/jev_systemone_live.jsonl")
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    let options: DecideOptions = serde_json::from_value(v4.input.options).unwrap();
    let err = decide(&mock, options).await.unwrap_err();
    assert!(err.to_string().contains("2 recording(s)"), "{err}");
    assert!(err.to_string().contains("aimux 0.5 or earlier"), "{err}");
}
