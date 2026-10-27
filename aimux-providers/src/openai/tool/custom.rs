//! The custom provider tool and its arguments (tool/custom.ts).

use aimux_core::AiMuxError;
use aimux_core::tool::ProviderTool;
use serde::{Deserialize, Serialize};

use crate::openai::tools::provider_tool;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrammarSyntax {
    Regex,
    Lark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CustomFormat {
    Text,
    Grammar {
        syntax: GrammarSyntax,
        definition: String,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomToolArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#async: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<CustomFormat>,
}

/// Configure the `custom_tool` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn custom_tool(args: CustomToolArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("custom", args)
}
