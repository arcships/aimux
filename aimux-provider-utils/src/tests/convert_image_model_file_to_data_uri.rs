//! Rust translation of
//! `packages/provider-utils/src/convert-image-model-file-to-data-uri.test.ts`
//! (@ai-sdk/provider-utils).

use crate::convert_image_model_file_to_data_uri;
use aimux_core::image_model::{ImageFile, ImageFileData};

fn file(media_type: &str, data: ImageFileData) -> ImageFile {
    ImageFile::File {
        media_type: media_type.to_owned(),
        data,
    }
}

mod url_files {
    use super::*;

    /// upstream: @ai-sdk/provider-utils src/convert-image-model-file-to-data-uri.test.ts › convertImageModelFileToDataUri() › URL files › should return the URL as-is for URL type files
    #[test]
    fn should_return_the_url_as_is_for_url_type_files() {
        let result = convert_image_model_file_to_data_uri(&ImageFile::Url {
            url: "https://example.com/image.png".into(),
        });

        assert_eq!(result, "https://example.com/image.png");
    }

    /// upstream: @ai-sdk/provider-utils src/convert-image-model-file-to-data-uri.test.ts › convertImageModelFileToDataUri() › URL files › should handle URLs with query parameters
    #[test]
    fn should_handle_urls_with_query_parameters() {
        let result = convert_image_model_file_to_data_uri(&ImageFile::Url {
            url: "https://example.com/image.png?width=100&height=200".into(),
        });

        assert_eq!(result, "https://example.com/image.png?width=100&height=200");
    }
}

mod base64_string_files {
    use super::*;

    /// upstream: @ai-sdk/provider-utils src/convert-image-model-file-to-data-uri.test.ts › convertImageModelFileToDataUri() › base64 string files › should return a data URI for base64 string data
    #[test]
    fn should_return_a_data_uri_for_base64_string_data() {
        let result = convert_image_model_file_to_data_uri(&file(
            "image/png",
            ImageFileData::Base64(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==".into(),
            ),
        ));

        assert_eq!(
            result,
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
        );
    }

    /// upstream: @ai-sdk/provider-utils src/convert-image-model-file-to-data-uri.test.ts › convertImageModelFileToDataUri() › base64 string files › should handle different media types
    #[test]
    fn should_handle_different_media_types() {
        let result = convert_image_model_file_to_data_uri(&file(
            "image/jpeg",
            ImageFileData::Base64("base64data".into()),
        ));

        assert_eq!(result, "data:image/jpeg;base64,base64data");
    }
}

mod uint8_array_files {
    use super::*;

    /// upstream: @ai-sdk/provider-utils src/convert-image-model-file-to-data-uri.test.ts › convertImageModelFileToDataUri() › Uint8Array files › should convert Uint8Array to base64 and return a data URI
    #[test]
    fn should_convert_uint8_array_to_base64_and_return_a_data_uri() {
        // "Hello" in bytes
        let result = convert_image_model_file_to_data_uri(&file(
            "image/png",
            ImageFileData::Binary(vec![72, 101, 108, 108, 111]),
        ));

        assert_eq!(result, "data:image/png;base64,SGVsbG8=");
    }

    /// upstream: @ai-sdk/provider-utils src/convert-image-model-file-to-data-uri.test.ts › convertImageModelFileToDataUri() › Uint8Array files › should handle empty Uint8Array
    #[test]
    fn should_handle_empty_uint8_array() {
        let result =
            convert_image_model_file_to_data_uri(&file("image/png", ImageFileData::Binary(vec![])));

        assert_eq!(result, "data:image/png;base64,");
    }

    /// upstream: @ai-sdk/provider-utils src/convert-image-model-file-to-data-uri.test.ts › convertImageModelFileToDataUri() › Uint8Array files › should handle different media types with Uint8Array
    #[test]
    fn should_handle_different_media_types_with_uint8_array() {
        let result = convert_image_model_file_to_data_uri(&file(
            "image/webp",
            ImageFileData::Binary(vec![72, 101, 108, 108, 111]),
        ));

        assert_eq!(result, "data:image/webp;base64,SGVsbG8=");
    }
}
