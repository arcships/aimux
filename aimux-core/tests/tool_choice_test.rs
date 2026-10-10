//! Ported from upstream `ai/src/prompt/prepare-tool-choice.test.ts`: the user
//! facing `toolChoice` values (`"auto" | "none" | "required" | { type: "tool",
//! toolName }`) and their wire shape.

use aimux_core::tool::ToolChoice;
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
