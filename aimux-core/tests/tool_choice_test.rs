//! Wire-format tests for the two tool choices.
//!
//! The user-level `ToolChoice` is the AI SDK's `toolChoice`
//! (`ai@7.0.127 src/types/language-model.ts:102-106`):
//! `"auto" | "none" | "required" | { "type": "tool", "toolName": "..." }`,
//! with cases ported from `ai/src/prompt/prepare-tool-choice.test.ts`.
//! A model call carries `LanguageModelV4ToolChoice`, an object union
//! (`@ai-sdk/provider@4.0.21
//! src/language-model/v4/language-model-v4-tool-choice.ts:1-5`), made by
//! `prepare_tool_choice` (`ai@7.0.127 src/prompt/prepare-tool-choice.ts:4-15`).
//! These tests lock the cross-language wire contract so refactors on the
//! Rust side cannot silently break the bindings.

use aimux_core::generate::GenerateTextOptions;
use aimux_core::tool::{LanguageModelV4ToolChoice, ToolChoice, prepare_tool_choice};
use serde_json::json;

fn assert_wire(choice: ToolChoice, wire: serde_json::Value) {
    assert_eq!(serde_json::to_value(&choice).unwrap(), wire);
    assert_eq!(serde_json::from_value::<ToolChoice>(wire).unwrap(), choice);
}

/// TS: returns auto when tool choice is not provided
#[test]
fn defaults_to_auto() {
    assert_eq!(ToolChoice::default(), ToolChoice::Auto);
}

/// TS: handles string tool choice: auto / none / required
#[test]
fn string_tool_choices() {
    assert_wire(ToolChoice::Auto, json!("auto"));
    assert_wire(ToolChoice::None, json!("none"));
    assert_wire(ToolChoice::Required, json!("required"));
}

/// TS: handles object tool choice
#[test]
fn object_tool_choice() {
    assert_wire(
        ToolChoice::Tool {
            tool_name: "tool2".into(),
        },
        json!({ "type": "tool", "toolName": "tool2" }),
    );
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
