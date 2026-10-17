//! Wire-format tests for the two tool choices.
//!
//! The user-level `ToolChoice` is the AI SDK's `toolChoice`
//! (`ai@7.0.127 src/types/language-model.ts:102-106`):
//! `"auto" | "none" | "required" | { "type": "tool", "toolName": "..." }`.
//! A model call carries `LanguageModelV4ToolChoice`, an object union
//! (`@ai-sdk/provider@4.0.21
//! src/language-model/v4/language-model-v4-tool-choice.ts:1-5`), made by
//! `prepare_tool_choice` (`ai@7.0.127 src/prompt/prepare-tool-choice.ts:4-15`).
//! These tests lock the cross-language wire contract so refactors on the
//! Rust side cannot silently break the bindings.

use aimux_core::generate::GenerateTextOptions;
use aimux_core::tool::{LanguageModelV4ToolChoice, ToolChoice, prepare_tool_choice};
use serde_json::json;

#[test]
fn serialize_unit_variants_as_bare_strings() {
    assert_eq!(
        serde_json::to_value(ToolChoice::Auto).unwrap(),
        json!("auto")
    );
    assert_eq!(
        serde_json::to_value(ToolChoice::None).unwrap(),
        json!("none")
    );
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

#[test]
fn v4_tool_choice_is_an_object_union() {
    let cases = [
        (LanguageModelV4ToolChoice::Auto, json!({ "type": "auto" })),
        (LanguageModelV4ToolChoice::None, json!({ "type": "none" })),
        (
            LanguageModelV4ToolChoice::Required,
            json!({ "type": "required" }),
        ),
        (
            LanguageModelV4ToolChoice::Tool {
                tool_name: "search".into(),
            },
            json!({ "type": "tool", "toolName": "search" }),
        ),
    ];
    for (choice, wire) in cases {
        assert_eq!(serde_json::to_value(&choice).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<LanguageModelV4ToolChoice>(wire).unwrap(),
            choice
        );
    }
}

#[test]
fn v4_tool_choice_rejects_the_user_level_string_form() {
    assert!(serde_json::from_value::<LanguageModelV4ToolChoice>(json!("auto")).is_err());
    assert!(
        serde_json::from_value::<LanguageModelV4ToolChoice>(json!({ "type": "tool" })).is_err()
    );
}

/// `prepareToolChoice`: `toolChoice == null ? {type:'auto'} : typeof
/// toolChoice === 'string' ? {type: toolChoice} : {type:'tool', toolName}`.
#[test]
fn prepare_tool_choice_follows_the_ai_sdk() {
    assert_eq!(prepare_tool_choice(None), LanguageModelV4ToolChoice::Auto);
    assert_eq!(
        prepare_tool_choice(Some(ToolChoice::Auto)),
        LanguageModelV4ToolChoice::Auto
    );
    assert_eq!(
        prepare_tool_choice(Some(ToolChoice::None)),
        LanguageModelV4ToolChoice::None
    );
    assert_eq!(
        prepare_tool_choice(Some(ToolChoice::Required)),
        LanguageModelV4ToolChoice::Required
    );
    assert_eq!(
        prepare_tool_choice(Some(ToolChoice::Tool {
            tool_name: "search".into()
        })),
        LanguageModelV4ToolChoice::Tool {
            tool_name: "search".into()
        }
    );
}

/// `generateText` sends `prepareToolChoice(...)` on every call, tools or
/// not (`ai@7.0.127 src/generate-text/generate-text.ts:953-955, :1046-1048`).
#[test]
fn a_call_without_a_choice_carries_auto_even_without_tools() {
    let options = GenerateTextOptions::default().into_call_options(vec![]);
    assert_eq!(options.tool_choice, Some(LanguageModelV4ToolChoice::Auto));

    let options = GenerateTextOptions {
        tool_choice: Some(ToolChoice::Required),
        ..Default::default()
    }
    .into_call_options(vec![]);
    assert_eq!(
        options.tool_choice,
        Some(LanguageModelV4ToolChoice::Required)
    );
}
