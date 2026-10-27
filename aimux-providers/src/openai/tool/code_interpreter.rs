//! The code interpreter provider tool and its arguments (tool/code-interpreter.ts).

use aimux_core::AiMuxError;
use aimux_core::tool::ProviderTool;
use serde::{Deserialize, Serialize};

use crate::openai::tools::provider_tool;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CodeInterpreterContainer {
    Id(String),
    Files {
        #[serde(rename = "fileIds", default, skip_serializing_if = "Option::is_none")]
        file_ids: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeInterpreterArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<CodeInterpreterContainer>,
}

/// Configure the `code_interpreter` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn code_interpreter(args: CodeInterpreterArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("code_interpreter", args)
}
