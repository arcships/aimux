//! Tool-argument vectors (`fixtures/v4/tool-input-vectors.json`), checked
//! against the AI SDK's `parseToolCall` (`ai@7.0.127
//! src/generate-text/parse-tool-call.ts`).
//!
//! For each vector:
//! - the provider DTO (`RawToolCall`) survives Rust -> JSON -> Rust with its
//!   argument text byte for byte;
//! - core parsing gives the value `JSON.parse` gives, or an invalid call whose
//!   error and input keep the original text (`:112-133`), never `{}` and
//!   never a partial-JSON repair;
//! - the core DTO (`ToolCall`) survives Rust -> JSON -> Rust with its value
//!   equal under JSON semantics.

use aimux_core::error::AiMuxError;
use aimux_core::parse_tool_call::parse_tool_call;
use aimux_core::tool::{FunctionTool, RawToolCall, Tool, ToolCall};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("../../fixtures/v4/tool-input-vectors.json");

struct Vector {
    row: u64,
    provider_input: String,
    valid: bool,
    input: Option<Value>,
}

fn vectors() -> Vec<Vector> {
    let fixture: Value = serde_json::from_str(FIXTURE).expect("fixture is JSON");
    fixture["vectors"]
        .as_array()
        .expect("vectors array")
        .iter()
        .map(|v| Vector {
            row: v["row"].as_u64().expect("row"),
            provider_input: v["providerInput"]
                .as_str()
                .expect("providerInput")
                .to_owned(),
            valid: v["expect"]["valid"].as_bool().expect("expect.valid"),
            input: v["expect"].get("input").cloned(),
        })
        .collect()
}

/// A provider-executed dynamic call: core parses it with no tool set, so only
/// JSON parsing (no schema) decides validity, as in
/// `parseProviderExecutedDynamicToolCall` (`parse-tool-call.ts:199-224`).
fn raw(input: &str) -> RawToolCall {
    RawToolCall {
        tool_call_id: "call-1".into(),
        tool_name: "lookup".into(),
        input: input.into(),
        provider_executed: Some(true),
        dynamic: Some(true),
        provider_metadata: None,
    }
}

fn error_text(call: &ToolCall) -> Option<&str> {
    match call.error.as_ref()? {
        AiMuxError::InvalidToolInput { tool_input, .. } => Some(tool_input),
        _ => None,
    }
}

/// JSON equality as JavaScript sees it: numbers compare by value, so the
/// `1e2` that `serde_json` keeps as a float equals JSON.parse's `100`.
fn json_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| json_eq(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_some_and(|b| json_eq(a, b)))
        }
        _ => a == b,
    }
}

async fn check(vector: &Vector) {
    let label = format!("row {} {:?}", vector.row, vector.provider_input);

    // Provider DTO: byte-identical argument text across the binding wire.
    let wire = serde_json::to_string(&raw(&vector.provider_input)).unwrap();
    let back: RawToolCall = serde_json::from_str(&wire).unwrap();
    assert_eq!(
        back.input.as_bytes(),
        vector.provider_input.as_bytes(),
        "{label}: provider text"
    );

    // Core parsing.
    let call = parse_tool_call(back, None, None, &[], None).await;
    if vector.valid {
        assert_eq!(call.invalid, None, "{label}: expected a valid call");
        let expected = vector.input.as_ref().expect("valid vectors carry input");
        assert!(
            json_eq(&call.input, expected),
            "{label}: parsed value {} != {expected}",
            call.input
        );
    } else {
        assert_eq!(
            call.invalid,
            Some(true),
            "{label}: expected an invalid call"
        );
        assert_eq!(
            error_text(&call),
            Some(vector.provider_input.as_str()),
            "{label}: error keeps the original text"
        );
        assert_eq!(
            call.input,
            Value::String(vector.provider_input.clone()),
            "{label}: input is the verbatim text"
        );
    }

    // Core DTO: JSON-semantic equality across the binding wire. A bare JSON
    // string stays a string (no second JSON.parse).
    let core_back: ToolCall = serde_json::from_str(&serde_json::to_string(&call).unwrap()).unwrap();
    assert!(
        json_eq(&core_back.input, &call.input),
        "{label}: core value"
    );
    assert_eq!(
        core_back.invalid, call.invalid,
        "{label}: core invalid flag"
    );
}

#[tokio::test]
async fn tool_input_vectors_match_the_ai_sdk_parse() {
    let vectors = vectors();
    assert_eq!(vectors.len(), 8);
    for vector in &vectors {
        check(vector).await;
    }
}

/// An error on a duplicate-key call keeps the original text, duplicate keys
/// included: `InvalidToolInputError.toolInput` is `toolCall.input`
/// (`parse-tool-call.ts:258-263`).
#[tokio::test]
async fn duplicate_key_text_survives_in_the_error() {
    let text = r#"{"a":1,"a":2}"#;
    let tools = vec![Tool::Function(FunctionTool::new(
        "lookup",
        json!({"type": "object", "properties": {"a": {"type": "string"}}}),
    ))];
    let mut call = raw(text);
    call.provider_executed = None;
    call.dynamic = None;
    let parsed = parse_tool_call(call, Some(&tools), None, &[], None).await;
    assert_eq!(parsed.invalid, Some(true));
    assert_eq!(error_text(&parsed), Some(text));
}
