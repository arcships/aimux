//! The tool search provider tool and its arguments (tool/tool-search.ts).

use std::collections::BTreeMap;

use aimux_core::AiMuxError;
use aimux_core::tool::ProviderTool;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::openai::tools::provider_tool;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSearchExecution {
    Server,
    Client,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSearchArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ToolSearchExecution>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<BTreeMap<String, Value>>,
}

/// Configure the `tool_search` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn tool_search(args: ToolSearchArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("tool_search", args)
}
