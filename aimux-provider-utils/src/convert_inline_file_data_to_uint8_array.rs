//! Inline upload data as bytes (`convert-inline-file-data-to-uint8-array.ts`).

use aimux_core::error::{AiMuxError, ErrorCause};
use aimux_core::files_model::UploadFileData;
use aimux_core::shared::FileBytes;

use crate::uint8_utils::convert_base64_to_uint8_array;

/// Text is its UTF-8 bytes, bytes are returned as they are, and a base64
/// string is decoded (`convertInlineFileDataToUint8Array`). There is no
/// stream variant to reject.
///
/// # Errors
///
/// Returns `InvalidArgument` when a base64 string does not decode.
pub fn convert_inline_file_data_to_uint8_array(
    data: &UploadFileData,
) -> Result<Vec<u8>, AiMuxError> {
    match data {
        UploadFileData::Text { text } => Ok(text.as_bytes().to_vec()),
        UploadFileData::Data {
            data: FileBytes::Binary(bytes),
        } => Ok(bytes.clone()),
        UploadFileData::Data {
            data: FileBytes::Base64(text),
        } => convert_base64_to_uint8_array(text).map_err(|error| AiMuxError::InvalidArgument {
            message: format!("invalid base64: {error}"),
            cause: Some(ErrorCause::source(&error)),
        }),
    }
}
