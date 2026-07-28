//! The `Files` trait — the provider-facing interface for file management.
//!
//! Aligned with Vercel AI SDK `FilesV4`
//! (`reference/ai/packages/provider/src/files/v4/`).
//!
//! Unlike the other model traits, the files interface uses `upload_file`
//! (no `do_` prefix), matching the TS spec.

use async_trait::async_trait;

use crate::error::AiMuxError;
use crate::shared::{
    FileBytes, SharedProviderMetadata, SharedProviderOptions, SharedProviderReference, Warning,
};

/// File data accepted by [`Files::upload_file`].
///
/// The V4 spec restricts upload input to the `data` and `text` variants of
/// `SharedV4FileData` (URLs and provider references are not valid uploads), so
/// this is a dedicated two-variant enum rather than the full [`crate::shared::FileData`].
#[derive(Debug, Clone)]
pub enum UploadFileData {
    /// Raw bytes (`Uint8Array`) or a base64-encoded string.
    Data { data: FileBytes },
    /// Inline text content (UTF-8).
    Text { text: String },
}

/// Options passed to [`Files::upload_file`].
///
/// Aligned with V4 `FilesV4UploadFileCallOptions`.
#[derive(Debug, Clone)]
pub struct UploadFileCallOptions {
    /// The file data (raw bytes/base64 or inline text).
    pub data: UploadFileData,

    /// The IANA media type of the file, e.g. `"application/pdf"`.
    pub media_type: String,

    /// The filename of the file.
    pub filename: Option<String>,

    /// Additional provider-specific options, keyed by provider name.
    pub provider_options: Option<SharedProviderOptions>,
}

impl UploadFileCallOptions {
    /// Create options with the given data and media type.
    pub fn new(data: UploadFileData, media_type: impl Into<String>) -> Self {
        Self {
            data,
            media_type: media_type.into(),
            filename: None,
            provider_options: None,
        }
    }
}

/// Result of [`Files::upload_file`].
///
/// Aligned with V4 `FilesV4UploadFileResult`.
#[derive(Debug, Clone)]
pub struct UploadFileResult {
    /// A provider reference mapping provider names to provider-specific file
    /// identifiers.
    pub provider_reference: SharedProviderReference,

    /// The IANA media type of the uploaded file, if available from the
    /// provider.
    pub media_type: Option<String>,

    /// The filename of the uploaded file, if available from the provider.
    pub filename: Option<String>,

    /// Additional provider-specific metadata, keyed by provider name.
    pub provider_metadata: Option<SharedProviderMetadata>,

    /// Warnings from the provider.
    pub warnings: Vec<Warning>,
}

/// The unified file-management interface (provider-facing).
///
/// Aligned with V4 `FilesV4`. Named `Files` (dropping the `V4` suffix) to
/// match the established convention of [`crate::language_model::LanguageModel`]
/// et al.
#[async_trait]
pub trait Files: Send + Sync {
    /// Specification version (always `"v4"`).
    fn specification_version(&self) -> &'static str {
        "v4"
    }

    /// Provider ID.
    fn provider(&self) -> &str;

    /// Upload a file to the provider and return a provider reference that can
    /// be used in subsequent API calls.
    ///
    /// Note: this method has **no** `do_` prefix, matching the TS `FilesV4`
    /// spec.
    async fn upload_file(
        &self,
        options: &UploadFileCallOptions,
    ) -> Result<UploadFileResult, AiMuxError>;
}
