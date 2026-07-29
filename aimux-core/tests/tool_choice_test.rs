//! Wire-format tests for `ToolChoice`.
//!
//! The serialized shape aligns with Vercel AI SDK `toolChoice`:
//! `"auto" | "none" | "required" | { "type": "tool", "toolName": "..." }`.
//! These tests lock the cross-language wire contract so refactors on the
//! Rust side cannot silently break the bindings.

use aimux_core::tool::ToolChoice;
use serde_json::json;

#[test]
fn serialize_unit_variants_as_bare_strings() {
    assert_eq!(serde_json::to_value(ToolChoice::Auto).unwrap(), json!("auto"));
    assert_eq!(serde_json::to_value(ToolChoice::None).unwrap(), json!("none"));
    assert_eq!(
        serde_json::to_value(ToolChoice::Required).unwrap(),
        json!("required")
    );
}

#[test]
fn serialize_tool_variant_as_tagged_object_with_camelcase_field() {
    let tc = ToolChoice::Tool {
        tool_name: "get_weather".into(),
    };
    assert_eq!(
        serde_json::to_value(tc).unwrap(),
        json!({ "type": "tool", "toolName": "get_weather" })
    );
}

#[test]
fn deserialize_bare_strings() {
    assert_eq!(
        serde_json::from_str::<ToolChoice>("\"auto\"").unwrap(),
        ToolChoice::Auto
    );
    assert_eq!(
        serde_json::from_str::<ToolChoice>("\"none\"").unwrap(),
        ToolChoice::None
    );
    assert_eq!(
        serde_json::from_str::<ToolChoice>("\"required\"").unwrap(),
        ToolChoice::Required
    );
}

#[test]
fn deserialize_tool_object() {
    let v = json!({ "type": "tool", "toolName": "get_weather" });
    assert_eq!(
        serde_json::from_value::<ToolChoice>(v).unwrap(),
        ToolChoice::Tool {
            tool_name: "get_weather".into()
        }
    );
}

#[test]
fn round_trip_all_variants() {
    let cases = [
        ToolChoice::Auto,
        ToolChoice::None,
        ToolChoice::Required,
        ToolChoice::Tool {
            tool_name: "search".into(),
        },
    ];
    for tc in cases {
        let j = serde_json::to_string(&tc).unwrap();
        let back: ToolChoice = serde_json::from_str(&j).unwrap();
        assert_eq!(back, tc, "round-trip failed for {j}");
    }
}

#[test]
fn reject_unknown_string() {
    assert!(serde_json::from_str::<ToolChoice>("\"always\"").is_err());
}

#[test]
fn reject_tool_object_missing_tool_name() {
    let v = json!({ "type": "tool" });
    assert!(serde_json::from_value::<ToolChoice>(v).is_err());
}

#[test]
fn reject_unknown_tool_type() {
    let v = json!({ "type": "function", "toolName": "x" });
    assert!(serde_json::from_value::<ToolChoice>(v).is_err());
}
