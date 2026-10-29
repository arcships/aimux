//! The providerOptions of a file part (`deepseek-file-part-options.ts`).

use serde::Deserialize;

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

use super::chat_language_model_options::invalid;

/// The `providerOptions` of a file part.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeepSeekFilePartOptions {
    /// How DeepSeek processes an image sent as an `image_url` part
    /// (`low`, `high`, `original`, `auto`).
    pub image_detail: Option<String>,
    /// Send inline image data as a `file` part instead of an `image_url`.
    pub file_data: Option<bool>,
}

/// The options of a file part.
///
/// # Errors
///
/// `InvalidArgument` when `imageDetail` is not a known value or `fileData` is
/// not `true`.
pub(crate) fn parse_file_part_options(
    provider_options: Option<&SharedProviderOptions>,
    name: &str,
) -> Result<DeepSeekFilePartOptions, AiMuxError> {
    let options: DeepSeekFilePartOptions =
        aimux_provider_utils::parse_provider_options(name, provider_options)?.unwrap_or_default();
    if options.file_data == Some(false) {
        return Err(invalid(name, "fileData must be true"));
    }
    if options
        .image_detail
        .as_deref()
        .is_some_and(|detail| !["low", "high", "original", "auto"].contains(&detail))
    {
        return Err(invalid(
            name,
            "imageDetail must be low, high, original or auto",
        ));
    }
    Ok(options)
}
