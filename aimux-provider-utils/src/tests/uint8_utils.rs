//! Rust translation of `packages/provider-utils/src/uint8-utils.test.ts`
//! (@ai-sdk/provider-utils). `uint8-utils.node.test.ts` checks the JS
//! encoder's heap use for a 14 MB array, which has no Rust counterpart.

use crate::{
    InvalidBase64Error, convert_base64_to_uint8_array, convert_to_base64,
    convert_uint8_array_to_base64,
};

fn create_bytes(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index % 256) as u8).collect()
}

mod convert_uint8_array_to_base64 {
    use super::*;

    /// upstream: @ai-sdk/provider-utils src/uint8-utils.test.ts › convertUint8ArrayToBase64 › converts a byte array to base64
    #[test]
    fn converts_a_byte_array_to_base64() {
        assert_eq!(
            convert_uint8_array_to_base64([72, 101, 108, 108, 111]),
            "SGVsbG8="
        );
    }

    /// upstream: @ai-sdk/provider-utils src/uint8-utils.test.ts › convertUint8ArrayToBase64 › handles an empty array
    #[test]
    fn handles_an_empty_array() {
        assert_eq!(convert_uint8_array_to_base64([0u8; 0]), "");
    }

    /// upstream: @ai-sdk/provider-utils src/uint8-utils.test.ts › convertUint8ArrayToBase64 › round-trips arrays larger than a single conversion chunk
    #[test]
    fn round_trips_arrays_larger_than_a_single_conversion_chunk() {
        let bytes = create_bytes(100_000);

        let result = convert_base64_to_uint8_array(convert_uint8_array_to_base64(&bytes)).unwrap();

        assert_eq!(result.len(), bytes.len());
        assert_eq!(result, bytes);
    }
}

mod convert_base64_to_uint8_array {
    use super::*;

    /// upstream: @ai-sdk/provider-utils src/uint8-utils.test.ts › convertBase64ToUint8Array › converts base64 to a byte array
    #[test]
    fn converts_base64_to_a_byte_array() {
        assert_eq!(
            convert_base64_to_uint8_array("SGVsbG8=").unwrap(),
            [72, 101, 108, 108, 111]
        );
    }

    /// upstream: @ai-sdk/provider-utils src/uint8-utils.test.ts › convertBase64ToUint8Array › supports base64url characters
    #[test]
    fn supports_base64url_characters() {
        assert_eq!(convert_base64_to_uint8_array("-_8=").unwrap(), [251, 255]);
    }

    /// upstream: @ai-sdk/provider-utils src/uint8-utils.test.ts › convertBase64ToUint8Array › handles an empty string
    #[test]
    fn handles_an_empty_string() {
        assert_eq!(convert_base64_to_uint8_array("").unwrap(), Vec::<u8>::new());
    }

    // Extra coverage: `atob`'s forgiving decode, which the TS function runs
    // after replacing the base64url characters.
    #[test]
    fn accepts_whitespace_missing_padding_and_trailing_bits() {
        assert_eq!(
            convert_base64_to_uint8_array(" SGVs\nbG8 ").unwrap(),
            b"Hello"
        );
        assert_eq!(convert_base64_to_uint8_array("SGVsbG9=").unwrap(), b"Hello");
    }

    #[test]
    fn rejects_what_atob_rejects() {
        for text in ["SGVsbG8!", "S", "SGV=sbG8", "SGVsbG8==="] {
            assert_eq!(
                convert_base64_to_uint8_array(text),
                Err(InvalidBase64Error),
                "{text}"
            );
        }
        assert_eq!(
            InvalidBase64Error.to_string(),
            "The string to be decoded is not correctly encoded."
        );
    }
}

mod convert_to_base64 {
    use super::*;

    /// upstream: @ai-sdk/provider-utils src/uint8-utils.test.ts › convertToBase64 › returns base64 strings unchanged
    #[test]
    fn returns_base64_strings_unchanged() {
        assert_eq!(convert_to_base64("SGVsbG8="), "SGVsbG8=");
    }

    /// upstream: @ai-sdk/provider-utils src/uint8-utils.test.ts › convertToBase64 › converts byte arrays to base64
    #[test]
    fn converts_byte_arrays_to_base64() {
        assert_eq!(
            convert_to_base64(&[72u8, 101, 108, 108, 111][..]),
            "SGVsbG8="
        );
    }
}
