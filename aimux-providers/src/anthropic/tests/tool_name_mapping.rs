//! The unit tests of `tool_name_mapping.rs` (previously inline there). The
//! mapping is aimux's form of `createToolNameMapping` from
//! `@ai-sdk/provider-utils`, so no upstream test file of this package covers it.

use crate::anthropic::tool_name_mapping::ToolNameMapping;
use aimux_core::tool::FunctionTool;
use aimux_core::tool::Tool;
use serde_json::json;

#[test]
fn unmapped_names_pass_through() {
    let mapping = ToolNameMapping::new(None);
    assert_eq!(mapping.to_custom_tool_name("web_search"), "web_search");
    assert_eq!(mapping.to_provider_tool_name("get_weather"), "get_weather");
}

#[test]
fn function_tools_are_ignored() {
    let tools = vec![Tool::Function(FunctionTool::new("web_search", json!({})))];
    let mapping = ToolNameMapping::new(Some(&tools));
    // A function tool named `web_search` must not hijack the provider name.
    assert_eq!(mapping.to_custom_tool_name("web_search"), "web_search");
}
