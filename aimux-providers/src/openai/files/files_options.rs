//! Provider options of OpenAI Files uploads (openai-files-options.ts).

use aimux_core::shared::SharedProviderOptions;

/// OpenAI provider-specific file upload options.
///
/// Aligned with TS `OpenAIFilesOptions`.
#[derive(Debug, Clone, Default)]
pub(crate) struct OpenAIFilesOptions {
    /// The purpose of the file (defaults to `"assistants"`).
    pub(crate) purpose: Option<String>,
    /// Seconds after which the file expires.
    pub(crate) expires_after: Option<u64>,
}

/// Parse provider options for the OpenAI files provider.
pub(crate) fn parse_openai_files_options(
    provider_options: Option<&SharedProviderOptions>,
) -> OpenAIFilesOptions {
    let mut opts = OpenAIFilesOptions::default();
    if let Some(po) = provider_options
        && let Some(openai) = po.get("openai")
    {
        if let Some(purpose) = openai.get("purpose").and_then(|v| v.as_str()) {
            opts.purpose = Some(purpose.to_string());
        }
        if let Some(expires_after) = openai
            .get("expiresAfter")
            .and_then(serde_json::Value::as_u64)
        {
            opts.expires_after = Some(expires_after);
        }
    }
    opts
}
