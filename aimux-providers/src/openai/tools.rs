//! The native Responses provider tools as one value (openai-tools.ts). The
//! factories and their arguments live in `tool/`, one file per upstream tool,
//! and are re-exported here.

use aimux_core::tool::ProviderTool;
use aimux_core::types::NamespacedKind;
use aimux_core::{AiMuxError, ErrorCause};
use serde::Serialize;

pub use super::tool::apply_patch::apply_patch;
pub use super::tool::code_interpreter::{
    CodeInterpreterArgs, CodeInterpreterContainer, code_interpreter,
};
pub use super::tool::computer::{
    ComputerButton, ComputerPoint, OpenAIComputerAction, OpenAIComputerSafetyCheck, computer,
};
pub use super::tool::custom::{CustomFormat, CustomToolArgs, GrammarSyntax, custom_tool};
pub use super::tool::file_search::{
    ComparisonOperator, CompoundOperator, FileSearchArgs, FileSearchFilter, FileSearchFilterValue,
    FileSearchRanking, file_search,
};
pub use super::tool::image_generation::{
    ImageAction, ImageBackground, ImageGenerationArgs, ImageModeration, ImageOutputFormat,
    ImageQuality, InputFidelity, InputImageMask, image_generation,
};
pub use super::tool::local_shell::local_shell;
pub use super::tool::mcp::{
    McpAllowedTools, McpApprovalFilter, McpApprovalPolicy, McpArgs, McpRequireApproval, mcp,
};
pub use super::tool::programmatic_tool_calling::programmatic_tool_calling;
pub use super::tool::shell::{
    LocalShellSkill, ShellArgs, ShellDomainSecret, ShellEnvironment, ShellMemoryLimit,
    ShellNetworkPolicy, ShellSkill, ShellSkillMediaType, ShellSkillSource, shell,
};
pub use super::tool::tool_search::{ToolSearchArgs, ToolSearchExecution, tool_search};
pub use super::tool::web_search::{
    SearchContextSize, UserLocation, UserLocationType, WebSearchArgs, WebSearchFilters, web_search,
};
pub use super::tool::web_search_preview::{WebSearchPreviewArgs, web_search_preview};

/// The `openai.<kind>` id of a provider tool.
pub(super) fn openai_id(kind: &str) -> NamespacedKind {
    NamespacedKind::new(format!("openai.{kind}")).expect("openai.<kind> is namespaced")
}

pub(super) fn provider_tool(kind: &str, args: impl Serialize) -> Result<ProviderTool, AiMuxError> {
    let args = serde_json::to_value(args).map_err(|error| AiMuxError::InvalidArgument {
        message: error.to_string(),
        cause: Some(ErrorCause::source(&error)),
    })?;
    crate::openai::responses::tool_args::validate_tool_args(kind, &args)?;
    Ok(ProviderTool {
        id: openai_id(kind),
        name: kind.to_owned(),
        args: args.as_object().cloned().ok_or_else(|| {
            AiMuxError::invalid_argument("provider tool arguments must be an object")
        })?,
        supports_deferred_results: None,
    })
}

/// The provider tools as one value, the AI SDK's `openaiTools`
/// (@ai-sdk/openai 4.0.83 src/openai-tools.ts:15), reachable on the provider
/// as `openai.tools` (openai-provider.ts:387): `openai().tools.web_search(…)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenAITools;

impl OpenAITools {
    /// [`web_search`](fn@web_search).
    ///
    /// # Errors
    /// Returns an error when arguments do not match the provider tool schema.
    pub fn web_search(&self, args: WebSearchArgs) -> Result<ProviderTool, AiMuxError> {
        web_search(args)
    }

    /// [`web_search_preview`](fn@web_search_preview).
    ///
    /// # Errors
    /// Returns an error when arguments do not match the provider tool schema.
    pub fn web_search_preview(
        &self,
        args: WebSearchPreviewArgs,
    ) -> Result<ProviderTool, AiMuxError> {
        web_search_preview(args)
    }

    /// [`file_search`](fn@file_search).
    ///
    /// # Errors
    /// Returns an error when arguments do not match the provider tool schema.
    pub fn file_search(&self, args: FileSearchArgs) -> Result<ProviderTool, AiMuxError> {
        file_search(args)
    }

    /// [`code_interpreter`](fn@code_interpreter).
    ///
    /// # Errors
    /// Returns an error when arguments do not match the provider tool schema.
    pub fn code_interpreter(&self, args: CodeInterpreterArgs) -> Result<ProviderTool, AiMuxError> {
        code_interpreter(args)
    }

    /// [`image_generation`](fn@image_generation).
    ///
    /// # Errors
    /// Returns an error when arguments do not match the provider tool schema.
    pub fn image_generation(&self, args: ImageGenerationArgs) -> Result<ProviderTool, AiMuxError> {
        image_generation(args)
    }

    /// [`custom_tool`](fn@custom_tool).
    ///
    /// # Errors
    /// Returns an error when arguments do not match the provider tool schema.
    pub fn custom_tool(&self, args: CustomToolArgs) -> Result<ProviderTool, AiMuxError> {
        custom_tool(args)
    }

    /// [`mcp`](fn@mcp).
    ///
    /// # Errors
    /// Returns an error when arguments do not match the provider tool schema.
    pub fn mcp(&self, args: McpArgs) -> Result<ProviderTool, AiMuxError> {
        mcp(args)
    }

    /// [`shell`](fn@shell).
    ///
    /// # Errors
    /// Returns an error when arguments do not match the provider tool schema.
    pub fn shell(&self, args: ShellArgs) -> Result<ProviderTool, AiMuxError> {
        shell(args)
    }

    /// [`tool_search`](fn@tool_search).
    ///
    /// # Errors
    /// Returns an error when arguments do not match the provider tool schema.
    pub fn tool_search(&self, args: ToolSearchArgs) -> Result<ProviderTool, AiMuxError> {
        tool_search(args)
    }

    /// [`apply_patch`](fn@apply_patch).
    #[must_use]
    pub fn apply_patch(&self) -> ProviderTool {
        apply_patch()
    }

    /// [`computer`](fn@computer).
    #[must_use]
    pub fn computer(&self) -> ProviderTool {
        computer()
    }

    /// [`local_shell`](fn@local_shell).
    #[must_use]
    pub fn local_shell(&self) -> ProviderTool {
        local_shell()
    }

    /// [`programmatic_tool_calling`](fn@programmatic_tool_calling).
    #[must_use]
    pub fn programmatic_tool_calling(&self) -> ProviderTool {
        programmatic_tool_calling()
    }
}
