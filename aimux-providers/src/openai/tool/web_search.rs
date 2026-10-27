//! The web search provider tool and its arguments (tool/web-search.ts).

use aimux_core::AiMuxError;
use aimux_core::tool::ProviderTool;
use serde::{Deserialize, Serialize};

use crate::openai::tools::provider_tool;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchContextSize {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum UserLocationType {
    #[default]
    #[serde(rename = "approximate")]
    Approximate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserLocation {
    pub r#type: UserLocationType,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchFilters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_domains: Option<Vec<String>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_domains: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_web_access: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<WebSearchFilters>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<SearchContextSize>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_location: Option<UserLocation>,
}

/// Configure the `web_search` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn web_search(args: WebSearchArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("web_search", args)
}
