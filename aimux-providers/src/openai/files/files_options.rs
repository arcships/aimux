//! Provider options of OpenAI Files uploads (openai-files-options.ts).

use aimux_core::shared::SharedProviderOptions;

/// `openaiFilesOptionsSchema`. Both keys are `.optional()`: absent is
/// `None`, `null` fails.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenAIFilesOptions {
    /// The purpose of the file (defaults to `"assistants"`).
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(crate) purpose: Option<String>,
    /// Seconds after which the file expires: `z.number()`, sent as written.
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(crate) expires_after: Option<serde_json::Number>,
}

/// `parseProviderOptions({ provider: 'openai', providerOptions, schema:
/// openaiFilesOptionsSchema })` (openai-files.ts:94-98); all `None` without
/// an `openai` namespace.
///
/// # Errors
///
/// `InvalidArgument` "invalid openai provider options" when the namespace
/// does not match the schema, with the serde error as its cause.
pub(crate) fn parse_openai_files_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<OpenAIFilesOptions, aimux_core::AiMuxError> {
    Ok(
        aimux_provider_utils::parse_provider_options("openai", provider_options)?
            .unwrap_or_default(),
    )
}
