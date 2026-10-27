//! The programmatic tool calling provider tool and its arguments (tool/programmatic-tool-calling.ts).

use aimux_core::tool::ProviderTool;

use crate::openai::tools::openai_id;

/// Create the `programmatic_tool_calling` provider tool. Its results may
/// arrive in a later response than its call, so it supports deferred results
/// (@ai-sdk/openai 4.0.83 src/tool/programmatic-tool-calling.ts:55).
#[must_use]
pub fn programmatic_tool_calling() -> ProviderTool {
    ProviderTool {
        id: openai_id("programmatic_tool_calling"),
        name: "programmatic_tool_calling".into(),
        args: serde_json::Map::new(),
        supports_deferred_results: Some(true),
    }
}
