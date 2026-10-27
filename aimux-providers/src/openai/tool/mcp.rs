//! The MCP provider tool and its arguments (tool/mcp.ts).

use std::collections::BTreeMap;

use aimux_core::AiMuxError;
use aimux_core::tool::ProviderTool;
use serde::{Deserialize, Serialize};

use crate::openai::tools::provider_tool;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpAllowedTools {
    Names(Vec<String>),
    Filter {
        #[serde(rename = "readOnly", default, skip_serializing_if = "Option::is_none")]
        read_only: Option<bool>,
        #[serde(rename = "toolNames", default, skip_serializing_if = "Option::is_none")]
        tool_names: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpApprovalPolicy {
    Always,
    Never,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpApprovalFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_names: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpRequireApproval {
    Policy(McpApprovalPolicy),
    Filter {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        never: Option<McpApprovalFilter>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpArgs {
    pub server_label: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<McpAllowedTools>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_approval: Option<McpRequireApproval>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_description: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
}

/// Configure the `mcp` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn mcp(args: McpArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("mcp", args)
}
