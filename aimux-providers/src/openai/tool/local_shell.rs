//! The local shell provider tool and its arguments (tool/local-shell.ts).

use aimux_core::tool::ProviderTool;

use crate::openai::tools::openai_id;

/// Create the `local_shell` provider tool.
#[must_use]
pub fn local_shell() -> ProviderTool {
    ProviderTool {
        id: openai_id("local_shell"),
        name: "local_shell".into(),
        args: serde_json::Map::new(),
        supports_deferred_results: None,
    }
}
