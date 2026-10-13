//! Serde helpers for the public JSON, which is the AI SDK's JSON.
//!
//! The AI SDK's runtime values are JavaScript objects; its JSON is what
//! `JSON.stringify` and the SDK's own serializers make of them. The helpers
//! here give the Rust types the same JSON where serde's defaults differ.

use base64::Engine as _;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use serde::{Deserialize, Deserializer, Serializer};

/// Encode bytes as the AI SDK does (`convertUint8ArrayToBase64`,
/// `@ai-sdk/provider-utils` `uint8-utils.ts:12-23`: `btoa`, the standard
/// alphabet with padding).
#[must_use]
pub fn encode_base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Decode a base64 string as the AI SDK does (`convertBase64ToUint8Array`,
/// `uint8-utils.ts:6-10`): `-` and `_` are read as `+` and `/`, and `atob`'s
/// forgiving decode ignores ASCII whitespace, does not require padding and
/// drops trailing bits.
///
/// # Errors
///
/// Returns the decode error when the text is not base64.
pub fn decode_base64(text: &str) -> Result<Vec<u8>, base64::DecodeError> {
    const FORGIVING: GeneralPurpose = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    let normalized: String = text
        .chars()
        .filter(|c| !matches!(c, ' ' | '\t' | '\n' | '\x0c' | '\r'))
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .collect();
    FORGIVING.decode(normalized)
}

/// `#[serde(with = "crate::wire::base64_bytes")]` for a `Vec<u8>` field: a
/// `Uint8Array` in the AI SDK, a base64 string in its JSON.
pub mod base64_bytes {
    use super::{Deserialize, Deserializer, Serializer};

    /// Write the bytes as a base64 string.
    ///
    /// # Errors
    ///
    /// Returns the serializer's error.
    pub fn serialize<T, S>(bytes: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: AsRef<[u8]>,
        S: Serializer,
    {
        serializer.serialize_str(&super::encode_base64(bytes.as_ref()))
    }

    /// Read a base64 string into bytes.
    ///
    /// # Errors
    ///
    /// Fails when the value is not a string or not base64.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        super::decode_base64(&text)
            .map_err(|error| serde::de::Error::custom(format!("invalid base64 data: {error}")))
    }
}

/// Serde and TypeScript for a `Binary(Vec<u8>) | Base64(String)` enum: the AI
/// SDK's `Uint8Array | string`, a base64 string in its JSON either way.
///
/// The Rust value keeps the form the provider returned; serializing writes
/// `Binary` as base64 and `Base64` verbatim, and parsing yields `Base64`.
macro_rules! base64_wire_enum {
    ($($ty:ty),* $(,)?) => {$(
        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                match self {
                    Self::Base64(text) => serializer.serialize_str(text),
                    Self::Binary(bytes) => {
                        serializer.serialize_str(&$crate::wire::encode_base64(bytes))
                    }
                }
            }
        }

        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                <String as serde::Deserialize>::deserialize(deserializer).map(Self::Base64)
            }
        }
    )*};
}
pub(crate) use base64_wire_enum;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_btoa_and_atob() {
        // `btoa(String.fromCodePoint(0, 1, 2, 250, 255))` and the reverse.
        assert_eq!(encode_base64(&[0, 1, 2, 250, 255]), "AAEC+v8=");
        assert_eq!(decode_base64("AAEC+v8=").unwrap(), [0, 1, 2, 250, 255]);
        // URL-safe characters, missing padding and whitespace are accepted
        // (uint8-utils.ts:7, atob's forgiving decode).
        assert_eq!(decode_base64("AAEC-v8").unwrap(), [0, 1, 2, 250, 255]);
        assert_eq!(decode_base64(" AAEC\n+v8= ").unwrap(), [0, 1, 2, 250, 255]);
        assert!(decode_base64("not base64!").is_err());
    }
}
