//! Azure's subset of the OpenAI provider tools (`azure-openai-tools.ts`).

use aimux_core::AiMuxError;
use aimux_core::tool::ProviderTool;

use crate::openai::internal::{
    CodeInterpreterArgs, FileSearchArgs, ImageGenerationArgs, WebSearchArgs, WebSearchPreviewArgs,
    code_interpreter, file_search, image_generation, web_search, web_search_preview,
};

/// Azure's subset of the provider tools, the AI SDK's `azureOpenaiTools`
/// (@ai-sdk/azure 4.0.84 src/azure-openai-tools.ts), reachable as
/// `azure.tools` (azure-openai-provider.ts:434).
#[derive(Debug, Clone, Copy, Default)]
pub struct AzureOpenAITools;

impl AzureOpenAITools {
    /// [`code_interpreter`].
    ///
    /// # Errors
    /// Returns an error when arguments do not match the provider tool schema.
    pub fn code_interpreter(&self, args: CodeInterpreterArgs) -> Result<ProviderTool, AiMuxError> {
        code_interpreter(args)
    }

    /// [`file_search`].
    ///
    /// # Errors
    /// Returns an error when arguments do not match the provider tool schema.
    pub fn file_search(&self, args: FileSearchArgs) -> Result<ProviderTool, AiMuxError> {
        file_search(args)
    }

    /// [`image_generation`].
    ///
    /// # Errors
    /// Returns an error when arguments do not match the provider tool schema.
    pub fn image_generation(&self, args: ImageGenerationArgs) -> Result<ProviderTool, AiMuxError> {
        image_generation(args)
    }

    /// [`web_search`].
    ///
    /// # Errors
    /// Returns an error when arguments do not match the provider tool schema.
    pub fn web_search(&self, args: WebSearchArgs) -> Result<ProviderTool, AiMuxError> {
        web_search(args)
    }

    /// [`web_search_preview`].
    ///
    /// # Errors
    /// Returns an error when arguments do not match the provider tool schema.
    pub fn web_search_preview(
        &self,
        args: WebSearchPreviewArgs,
    ) -> Result<ProviderTool, AiMuxError> {
        web_search_preview(args)
    }
}
