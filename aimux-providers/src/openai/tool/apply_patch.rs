//! The apply patch provider tool and its arguments (tool/apply-patch.ts).

use aimux_core::tool::ProviderTool;

use crate::openai::tools::openai_id;

/// Create the `apply_patch` provider tool.
#[must_use]
pub fn apply_patch() -> ProviderTool {
    ProviderTool {
        id: openai_id("apply_patch"),
        name: "apply_patch".into(),
        args: serde_json::Map::new(),
        supports_deferred_results: None,
    }
}
