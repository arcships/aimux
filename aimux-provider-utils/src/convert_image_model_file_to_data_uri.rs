//! An image model input file as a URL (`convert-image-model-file-to-data-uri.ts`).

use aimux_core::image_model::ImageFile;

use crate::uint8_utils::convert_to_base64;

/// A URL file is its URL; inline data becomes a `data:` URI with the file's
/// media type, bytes encoded to base64 (`convertImageModelFileToDataUri`).
#[must_use]
pub fn convert_image_model_file_to_data_uri(file: &ImageFile) -> String {
    match file {
        ImageFile::Url { url } => url.clone(),
        ImageFile::File { media_type, data } => {
            format!("data:{media_type};base64,{}", convert_to_base64(data))
        }
    }
}
