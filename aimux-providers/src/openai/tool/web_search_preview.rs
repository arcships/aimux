//! The web search preview provider tool and its arguments (tool/web-search-preview.ts).

use aimux_core::AiMuxError;
use aimux_core::tool::ProviderTool;
use serde::{Deserialize, Serialize};

use crate::openai::tool::web_search::{SearchContextSize, UserLocation};
use crate::openai::tools::provider_tool;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchPreviewArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<SearchContextSize>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_location: Option<UserLocation>,
}

/// Configure the `web_search_preview` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn web_search_preview(args: WebSearchPreviewArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("web_search_preview", args)
}
