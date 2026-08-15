//! Bidirectional mapping between user-facing tool names and Anthropic's
//! provider tool names.
//!
//! A [`ProviderTool`](aimux_core::tool::ProviderTool) carries both an `id` (`anthropic.web_search_20250305`)
//! and a `name` — the name the caller uses to refer to that tool in this
//! request. Anthropic's wire format uses its own fixed name (`web_search`), so
//! the two differ whenever the caller renames a provider tool.
//!
//! Both directions are needed:
//! - **response side** — a `web_search_tool_result` block names the tool
//!   `web_search`; the caller expects to see their own name on the resulting
//!   `ToolResult`.
//! - **prompt side** — a replayed `ToolResult` carries the caller's name; the
//!   converter must recover the provider name to pick the right result block
//!   type (`web_search_tool_result` vs `code_execution_tool_result` vs …).
//!
//! Mirrors `createToolNameMapping` in
//! `reference/vercel-ai/provider-utils/src/create-tool-name-mapping.ts`, with
//! the Anthropic id→name table from `anthropic-language-model.ts`.

use std::collections::HashMap;

use aimux_core::tool::Tool;

/// Anthropic provider tool id → the tool name Anthropic uses on the wire.
///
/// Kept in sync with the `name` field each id produces in
/// [`prepare_tools`](super::prepare_tools).
fn provider_tool_name(id: &str) -> Option<&'static str> {
    Some(match id {
        "anthropic.code_execution_20250522"
        | "anthropic.code_execution_20250825"
        | "anthropic.code_execution_20260120" => "code_execution",
        "anthropic.computer_20241022"
        | "anthropic.computer_20250124"
        | "anthropic.computer_20251124" => "computer",
        "anthropic.text_editor_20241022" | "anthropic.text_editor_20250124" => "str_replace_editor",
        "anthropic.text_editor_20250429" | "anthropic.text_editor_20250728" => {
            "str_replace_based_edit_tool"
        }
        "anthropic.bash_20241022" | "anthropic.bash_20250124" => "bash",
        "anthropic.memory_20250818" => "memory",
        "anthropic.web_search_20250305" | "anthropic.web_search_20260209" => "web_search",
        "anthropic.web_fetch_20250910" | "anthropic.web_fetch_20260209" => "web_fetch",
        "anthropic.tool_search_regex_20251119" => "tool_search_tool_regex",
        "anthropic.tool_search_bm25_20251119" => "tool_search_tool_bm25",
        "anthropic.advisor_20260301" => "advisor",
        _ => return None,
    })
}

/// Bidirectional tool-name mapping for one model call.
///
/// Only provider tools that the caller actually passed are mapped; every other
/// name passes through unchanged.
#[derive(Debug, Default, Clone)]
pub struct ToolNameMapping {
    custom_to_provider: HashMap<String, String>,
    provider_to_custom: HashMap<String, String>,
}

impl ToolNameMapping {
    /// Build the mapping from the tools passed on this call.
    #[must_use]
    pub fn new(tools: Option<&[Tool]>) -> Self {
        let mut mapping = ToolNameMapping::default();
        for tool in tools.unwrap_or(&[]) {
            if let Tool::Provider(pt) = tool
                && let Some(provider_name) = provider_tool_name(&pt.id)
            {
                mapping
                    .custom_to_provider
                    .insert(pt.name.clone(), provider_name.to_string());
                mapping
                    .provider_to_custom
                    .insert(provider_name.to_string(), pt.name.clone());
            }
        }
        mapping
    }

    /// Caller's name → Anthropic's wire name. Unmapped names pass through.
    pub fn to_provider_tool_name<'a>(&'a self, custom_name: &'a str) -> &'a str {
        self.custom_to_provider
            .get(custom_name)
            .map(String::as_str)
            .unwrap_or(custom_name)
    }

    /// Anthropic's wire name → caller's name. Unmapped names pass through.
    pub fn to_custom_tool_name<'a>(&'a self, provider_name: &'a str) -> &'a str {
        self.provider_to_custom
            .get(provider_name)
            .map(String::as_str)
            .unwrap_or(provider_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimux_core::tool::{FunctionTool, ProviderTool};
    use serde_json::json;

    fn provider_tool(id: &str, name: &str) -> Tool {
        Tool::Provider(ProviderTool {
            id: id.to_string(),
            name: name.to_string(),
            args: json!({}),
        })
    }

    #[test]
    fn maps_renamed_provider_tool_both_ways() {
        let tools = vec![provider_tool("anthropic.web_search_20250305", "mySearch")];
        let mapping = ToolNameMapping::new(Some(&tools));

        assert_eq!(mapping.to_custom_tool_name("web_search"), "mySearch");
        assert_eq!(mapping.to_provider_tool_name("mySearch"), "web_search");
    }

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

    #[test]
    fn all_code_execution_versions_share_one_provider_name() {
        for id in [
            "anthropic.code_execution_20250522",
            "anthropic.code_execution_20250825",
            "anthropic.code_execution_20260120",
        ] {
            let tools = vec![provider_tool(id, "runCode")];
            let mapping = ToolNameMapping::new(Some(&tools));
            assert_eq!(
                mapping.to_custom_tool_name("code_execution"),
                "runCode",
                "{id} must map to the code_execution provider name"
            );
        }
    }

    /// Every provider tool id this crate can build a request for must also be
    /// mappable, and to the same name the request body uses. The two tables are
    /// written out separately, so without this they drift silently: a renamed
    /// tool would round-trip under the wrong name.
    #[test]
    fn every_known_provider_tool_id_maps_to_its_request_name() {
        use crate::anthropic::prepare_tools::prepare_provider_tool;
        use std::collections::BTreeSet;

        // Ids are recovered from the source rather than duplicated here, so a
        // newly supported tool cannot be added without this test seeing it.
        let src = include_str!("prepare_tools.rs");
        let ids: BTreeSet<&str> = src
            .match_indices("\"anthropic.")
            .filter_map(|(i, _)| {
                let rest = &src[i + 1..];
                rest.find('"').map(|end| &rest[..end])
            })
            .collect();
        assert!(
            ids.len() > 10,
            "expected to recover the id table, got {ids:?}"
        );

        for id in ids {
            let mut betas = BTreeSet::new();
            let Some(def) = prepare_provider_tool(id, &json!({}), &mut betas) else {
                continue;
            };
            let request_name = def["name"].as_str().expect("provider tool has a name");
            let tools = vec![provider_tool(id, "myCustomName")];
            let mapping = ToolNameMapping::new(Some(&tools));
            assert_eq!(
                mapping.to_custom_tool_name(request_name),
                "myCustomName",
                "{id} is sent as `{request_name}` but is not in the name mapping"
            );
        }
    }
}
