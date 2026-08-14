//! Contract tests — validate wire format consistency across languages.
//!
//! These tests load shared JSON fixtures and verify that Rust serialization
//! matches the expected wire format. The same fixtures are used by Node/Python
//! tests to ensure cross-language consistency.

use aimux_core::generate::GenerateTextOptions;
use aimux_core::message::{ModelMessage, Role};
use aimux_core::options::{TimeoutConfiguration, ToolChoice};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReasonUnified, ReasoningEffort};
use serde_json::Value;

/// A single fixture from the shared JSON file.
#[derive(serde::Deserialize)]
struct Fixture {
    name: String,
    #[serde(rename = "type")]
    ty: String,
    json: String,
    #[allow(dead_code)]
    description: String,
}

fn load_fixtures() -> Vec<Fixture> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("contract-tests")
        .join("fixtures")
        .join("wire-format.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixtures at {path:?}: {e}"));
    serde_json::from_str(&content).expect("failed to parse fixtures JSON")
}

/// Assert that a Rust value serializes to the expected JSON.
fn assert_serialize<T: serde::Serialize>(value: &T, expected_json: &str, name: &str) {
    let actual =
        serde_json::to_string(value).unwrap_or_else(|e| panic!("{name}: serialize failed: {e}"));
    let actual_val: Value = serde_json::from_str(&actual)
        .unwrap_or_else(|e| panic!("{name}: actual is not valid JSON: {e}"));
    let expected_val: Value = serde_json::from_str(expected_json)
        .unwrap_or_else(|e| panic!("{name}: expected is not valid JSON: {e}"));
    assert_eq!(
        actual_val, expected_val,
        "fixture '{name}': serialization mismatch\n  expected: {expected_json}\n  actual:   {actual}"
    );
}

/// Assert that expected JSON deserializes to a Rust value that re-serializes
/// to the same JSON (round-trip).
#[allow(dead_code)]
fn assert_roundtrip<T: serde::Serialize + serde::de::DeserializeOwned>(
    expected_json: &str,
    name: &str,
    parse: impl Fn(&Value) -> Option<T>,
) {
    let expected_val: Value =
        serde_json::from_str(expected_json).expect("expected is not valid JSON");
    // We can't always construct the exact Rust value from the fixture,
    // so we verify round-trip: deserialize → re-serialize → compare.
    let roundtripped = match serde_json::from_str::<T>(expected_json) {
        Ok(v) => serde_json::to_string(&v).unwrap(),
        Err(_) => {
            // Some types need special handling — skip roundtrip for those
            return;
        }
    };
    let rt_val: Value = serde_json::from_str(&roundtripped).unwrap();
    assert_eq!(
        rt_val, expected_val,
        "fixture '{name}': round-trip mismatch"
    );
    // Suppress unused warning
    let _ = parse;
}

#[test]
fn tool_choice_wire_format() {
    assert_serialize(&ToolChoice::Auto, "\"auto\"", "tool_choice_auto");
    assert_serialize(&ToolChoice::None, "\"none\"", "tool_choice_none");
    assert_serialize(
        &ToolChoice::Required,
        "\"required\"",
        "tool_choice_required",
    );
    assert_serialize(
        &ToolChoice::Tool {
            tool_name: "get_weather".into(),
        },
        "{\"type\":\"tool\",\"toolName\":\"get_weather\"}",
        "tool_choice_tool",
    );
}

#[test]
fn role_wire_format() {
    assert_serialize(&Role::User, "\"user\"", "role_user");
    assert_serialize(&Role::System, "\"system\"", "role_system");
}

#[test]
fn finish_reason_unified_wire_format() {
    assert_serialize(
        &FinishReasonUnified::Stop,
        "\"stop\"",
        "finish_reason_unified_stop",
    );
}

#[test]
fn reasoning_effort_wire_format() {
    assert_serialize(&ReasoningEffort::High, "\"high\"", "reasoning_effort_high");
}

#[test]
fn generate_text_options_default_wire_format() {
    let opts = GenerateTextOptions::default();
    let json = serde_json::to_string(&opts).unwrap();
    let val: Value = serde_json::from_str(&json).unwrap();
    assert!(
        val.is_object(),
        "GenerateTextOptions should serialize to object"
    );
    // All fields should be null for default
    if let Value::Object(obj) = &val {
        for (_, v) in obj {
            assert!(v.is_null(), "default field should be null, got {v}");
        }
    }
}

/// RFC-0016 M2 true-case: `include_raw_chunks: Some(true)` round-trips on the
/// wire (companion of the `generate_text_options_include_raw_chunks_true`
/// fixture).
#[test]
fn generate_text_options_include_raw_chunks_true_wire_format() {
    let opts = GenerateTextOptions {
        include_raw_chunks: Some(true),
        ..Default::default()
    };
    let json = serde_json::to_string(&opts).unwrap();
    let val: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        val.get("include_raw_chunks"),
        Some(&Value::Bool(true)),
        "include_raw_chunks must serialize as true, got {json}"
    );
    // Round-trip: deserialize back to the same option.
    let back: GenerateTextOptions = serde_json::from_str(&json).unwrap();
    assert_eq!(back.include_raw_chunks, Some(true));
}

/// RFC-0024: an explicit `session_id` crosses the wire as snake_case and
/// round-trips (companion of the `generate_text_options_with_session_id`
/// fixture).
#[test]
fn generate_text_options_session_id_wire_format() {
    let opts = GenerateTextOptions {
        session_id: Some("sess-1".into()),
        ..Default::default()
    };
    let json = serde_json::to_string(&opts).unwrap();
    let val: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        val.get("session_id"),
        Some(&Value::String("sess-1".into())),
        "session_id must serialize as snake_case string, got {json}"
    );
    let back: GenerateTextOptions = serde_json::from_str(&json).unwrap();
    assert_eq!(back.session_id.as_deref(), Some("sess-1"));
}

/// Look up a fixture's expected JSON by name, failing loudly if it is gone.
///
/// This is the seam that makes Rust — the wire authority — actually consume the
/// shared fixture file instead of only hardcoding its own expectations. Without
/// it a fixture can drift away from Rust's real output and only the *other*
/// bindings go red, which points the blame at exactly the wrong side.
fn fixture_json(name: &str) -> String {
    load_fixtures()
        .into_iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("fixture '{name}' not found in wire-format.json"))
        .json
}

/// Every numeric field carries a non-null value, so each binding's declared
/// types are exercised rather than hidden behind `null`.
///
/// `top_k` is deliberately fractional. Rust is `Option<f64>` — matching the
/// upstream AI SDK, whose `topK?: number` carries no integer constraint — so a
/// binding that declares it as an integer cannot round-trip this fixture.
#[test]
fn generate_text_options_numeric_types_wire_format() {
    let opts = GenerateTextOptions {
        max_output_tokens: Some(256),
        temperature: Some(0.7),
        top_p: Some(0.95),
        top_k: Some(40.5),
        presence_penalty: Some(0.5),
        frequency_penalty: Some(-0.5),
        seed: Some(42),
        max_retries: Some(3),
        ..Default::default()
    };
    let expected = fixture_json("generate_text_options_numeric_types");
    assert_serialize(&opts, &expected, "generate_text_options_numeric_types");

    // Round-trip: the fractional top_k must survive as a fraction. An integer
    // field would truncate 40.5 to 40 and still "pass" a name-only check.
    let back: GenerateTextOptions = serde_json::from_str(&expected).unwrap();
    assert_eq!(back.top_k, Some(40.5), "top_k must round-trip as f64");
    assert_eq!(
        back.frequency_penalty,
        Some(-0.5),
        "penalties must stay signed"
    );
    assert_eq!(back.max_output_tokens, Some(256));
    assert_eq!(back.seed, Some(42));
    assert_eq!(back.max_retries, Some(3));
}

#[test]
fn stream_part_text_delta_wire_format() {
    let part = StreamPart::TextDelta {
        id: "tx1".into(),
        delta: "Hello".into(),
        provider_metadata: None,
    };
    let json = serde_json::to_string(&part).unwrap();
    let val: Value = serde_json::from_str(&json).unwrap();
    assert!(
        val.get("TextDelta").is_some(),
        "expected TextDelta variant, got {json}"
    );
}

#[test]
fn stream_part_stream_start_wire_format() {
    let part = StreamPart::StreamStart { warnings: vec![] };
    let json = serde_json::to_string(&part).unwrap();
    let val: Value = serde_json::from_str(&json).unwrap();
    assert!(
        val.get("StreamStart").is_some(),
        "expected StreamStart variant, got {json}"
    );
}

#[test]
fn stream_part_raw_wire_format() {
    let part = StreamPart::Raw {
        raw_value: serde_json::json!({ "id": "c1", "choices": [] }),
    };
    let json = serde_json::to_string(&part).unwrap();
    let val: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        val.get("Raw").and_then(|r| r.get("raw_value")),
        Some(&serde_json::json!({ "id": "c1", "choices": [] })),
        "expected Raw variant with raw_value, got {json}"
    );
}

#[test]
fn model_message_text_wire_format() {
    let msg = ModelMessage::user("Hello");
    assert_serialize(
        &msg,
        "{\"role\":\"user\",\"content\":\"Hello\"}",
        "model_message_text",
    );
}

/// Deserialize one fixture into its Rust type, re-serialize, and compare the
/// result against the fixture's own JSON.
fn assert_fixture_matches_rust<T>(fixture: &Fixture)
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let parsed: T = serde_json::from_str(&fixture.json).unwrap_or_else(|e| {
        panic!(
            "fixture '{}' ({}) does not deserialize into its Rust type: {e}\n  fixture: {}",
            fixture.name, fixture.ty, fixture.json
        )
    });
    let rust_json = serde_json::to_string(&parsed).unwrap();
    let actual: Value = serde_json::from_str(&rust_json).unwrap();
    let expected: Value = serde_json::from_str(&fixture.json).unwrap();
    assert_eq!(
        actual, expected,
        "fixture '{}' does not match Rust's own serialization\n  fixture: {}\n  rust:    {}",
        fixture.name, fixture.json, rust_json
    );
}

/// Every fixture must round-trip through the Rust type it claims to describe.
///
/// Rust is the wire authority: the other bindings validate themselves against
/// this file, so a fixture describing a shape Rust never emits sends four
/// bindings chasing a fiction. The predecessor of this test only checked that
/// the fixture's JSON *parsed*, which is why the `session_id: null` drift
/// survived — `session_id` is `skip_serializing_if = "Option::is_none"`, so
/// Rust omits the key entirely.
///
/// A fixture whose `type` has no arm here fails rather than being skipped:
/// silent skipping is how a net grows holes.
#[test]
fn all_fixtures_match_rust_serialization() {
    let fixtures = load_fixtures();
    assert!(!fixtures.is_empty(), "no fixtures loaded");

    for f in &fixtures {
        match f.ty.as_str() {
            "ToolChoice" => assert_fixture_matches_rust::<ToolChoice>(f),
            "StreamPart" => assert_fixture_matches_rust::<StreamPart>(f),
            "GenerateTextOptions" => assert_fixture_matches_rust::<GenerateTextOptions>(f),
            "TimeoutConfiguration" => assert_fixture_matches_rust::<TimeoutConfiguration>(f),
            "Role" => assert_fixture_matches_rust::<Role>(f),
            "ModelMessage" => assert_fixture_matches_rust::<ModelMessage>(f),
            "FinishReasonUnified" => assert_fixture_matches_rust::<FinishReasonUnified>(f),
            "ReasoningEffort" => assert_fixture_matches_rust::<ReasoningEffort>(f),
            other => panic!(
                "fixture '{}' declares type '{other}', which has no round-trip arm in \
                 all_fixtures_match_rust_serialization — wire it up so the fixture is \
                 actually pinned to Rust",
                f.name
            ),
        }
    }
}
