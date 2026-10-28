//! `createToolNameMapping` from `@ai-sdk/provider-utils`
//! (`create-tool-name-mapping.ts`).
//!
//! A [`ProviderTool`](aimux_core::tool::ProviderTool) carries both an `id`
//! (`anthropic.web_search_20250305`) and a `name`, the name the caller uses
//! for it in this call. The provider's wire format uses its own fixed name
//! (`web_search`), so the two differ whenever the caller renames a provider
//! tool: prompts map the caller's names to the provider's, responses map them
//! back.

use std::collections::HashMap;

use aimux_core::tool::Tool;

/// Maps between custom tool names and provider tool names.
#[derive(Debug, Default, Clone)]
pub struct ToolNameMapping {
    custom_tool_name_to_provider_tool_name: HashMap<String, String>,
    provider_tool_name_to_custom_tool_name: HashMap<String, String>,
}

impl ToolNameMapping {
    /// Maps a custom tool name (used by the client) to the provider's tool
    /// name. A name without a mapping is returned as given.
    #[must_use]
    pub fn to_provider_tool_name<'a>(&'a self, custom_tool_name: &'a str) -> &'a str {
        self.custom_tool_name_to_provider_tool_name
            .get(custom_tool_name)
            .map_or(custom_tool_name, String::as_str)
    }

    /// Maps a provider tool name to the custom tool name used by the client.
    /// A name without a mapping is returned as given.
    #[must_use]
    pub fn to_custom_tool_name<'a>(&'a self, provider_tool_name: &'a str) -> &'a str {
        self.provider_tool_name_to_custom_tool_name
            .get(provider_tool_name)
            .map_or(provider_tool_name, String::as_str)
    }
}

/// Builds the mapping for the provider tools of one call.
///
/// `tools` are the tools passed to the language model; `provider_tool_names`
/// maps provider tool ids (`provider.tool`) to the provider tool names.
/// Function tools and provider tools whose id is not listed are not mapped.
#[must_use]
pub fn create_tool_name_mapping(
    tools: Option<&[Tool]>,
    provider_tool_names: &[(&str, &str)],
) -> ToolNameMapping {
    let mut mapping = ToolNameMapping::default();
    for tool in tools.unwrap_or_default() {
        if let Tool::Provider(tool) = tool
            && let Some((_, provider_tool_name)) =
                provider_tool_names.iter().find(|(id, _)| tool.id == *id)
        {
            mapping
                .custom_tool_name_to_provider_tool_name
                .insert(tool.name.clone(), (*provider_tool_name).to_owned());
            mapping
                .provider_tool_name_to_custom_tool_name
                .insert((*provider_tool_name).to_owned(), tool.name.clone());
        }
    }
    mapping
}
