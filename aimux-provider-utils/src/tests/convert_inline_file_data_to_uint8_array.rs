//! Rust translation of
//! `packages/provider-utils/src/convert-inline-file-data-to-uint8-array.test.ts`
//! (@ai-sdk/provider-utils). `UploadFileData` has no stream variant, so the
//! case that rejects stream data has no Rust counterpart.

use aimux_core::files_model::UploadFileData;
use aimux_core::shared::FileBytes;

use crate::convert_inline_file_data_to_uint8_array;

/// upstream: @ai-sdk/provider-utils src/convert-inline-file-data-to-uint8-array.test.ts › convertInlineFileDataToUint8Array › converts text data to UTF-8 bytes
#[test]
fn converts_text_data_to_utf_8_bytes() {
    assert_eq!(
        convert_inline_file_data_to_uint8_array(&UploadFileData::Text { text: "abc".into() })
            .unwrap(),
        b"abc"
    );
}

/// upstream: @ai-sdk/provider-utils src/convert-inline-file-data-to-uint8-array.test.ts › convertInlineFileDataToUint8Array › returns Uint8Array data as-is
#[test]
fn returns_uint8_array_data_as_is() {
    let bytes = vec![1, 2, 3];
    assert_eq!(
        convert_inline_file_data_to_uint8_array(&UploadFileData::Data {
            data: FileBytes::Binary(bytes.clone()),
        })
        .unwrap(),
        bytes
    );
}

/// upstream: @ai-sdk/provider-utils src/convert-inline-file-data-to-uint8-array.test.ts › convertInlineFileDataToUint8Array › decodes base64 string data
#[test]
fn decodes_base64_string_data() {
    assert_eq!(
        convert_inline_file_data_to_uint8_array(&UploadFileData::Data {
            data: FileBytes::Base64("YWJj".into()),
        })
        .unwrap(),
        b"abc"
    );
}
