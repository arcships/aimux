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

/// `#[serde(default, deserialize_with = "crate::wire::present", skip_serializing_if =
/// "Option::is_none")]` for an `Option<Value>` that is an optional `unknown` or
/// `JSONValue` upstream.
///
/// The AI SDK's JSON omits `undefined` and keeps `null`, so the two differ: a
/// missing field is `None`, an explicit `null` is `Some(Value::Null)` (serde's
/// own `Option` reads both as `None`), and serializing writes each back.
///
/// # Errors
///
/// Returns the value's deserialization error.
pub fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
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

/// A value the AI SDK writes with a `type` tag wherever it stands, which aimux
/// otherwise holds untagged because a tagged union (`content`, the text
/// stream) writes the tag for it.
///
/// Outside such a union the tag must be written by the field:
/// `#[serde(with = "crate::wire::typed")]` for one value,
/// `#[serde(with = "crate::wire::typed_vec")]` for a list, and [`Tagged`]
/// for a value serialized on its own.
pub trait TypeTag {
    /// The value of the `type` field, e.g. `"tool-call"`.
    const TYPE: &'static str;
}

/// Serializes the borrowed value as a JSON object whose first field is
/// `"type": T::TYPE`, followed by the value's own fields, as the AI SDK's
/// objects list them (`{type: 'tool-call', toolCallId, ...}`).
pub struct Tagged<'a, T>(pub &'a T);

impl<T: serde::Serialize + TypeTag> serde::Serialize for Tagged<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{Error as _, SerializeMap as _};
        let serde_json::Value::Object(fields) =
            serde_json::to_value(self.0).map_err(S::Error::custom)?
        else {
            return Err(S::Error::custom(format!(
                "a `{}` value must serialize as a JSON object",
                T::TYPE
            )));
        };
        let mut map = serializer.serialize_map(Some(fields.len() + 1))?;
        map.serialize_entry("type", T::TYPE)?;
        for (key, value) in fields.iter().filter(|(key, _)| *key != "type") {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// Read a tagged value: the `type` field may be missing (JSON written before
/// aimux wrote the tag), but when present it must be `T::TYPE`.
fn untag<T>(value: serde_json::Value) -> Result<T, String>
where
    T: serde::de::DeserializeOwned + TypeTag,
{
    let serde_json::Value::Object(mut fields) = value else {
        return Err(format!("expected a `{}` object", T::TYPE));
    };
    match fields.remove("type") {
        None => {}
        Some(serde_json::Value::String(tag)) if tag == T::TYPE => {}
        Some(other) => {
            return Err(format!("expected type {:?}, found {other}", T::TYPE));
        }
    }
    serde_json::from_value(serde_json::Value::Object(fields)).map_err(|error| error.to_string())
}

/// `#[serde(with = "crate::wire::typed")]` for a [`TypeTag`] field: written
/// with its `type` tag, read with the tag optional (see [`Tagged`]).
pub mod typed {
    use super::{Deserialize, Deserializer, Serializer, Tagged, TypeTag};

    /// Write the value with its `type` tag first.
    ///
    /// # Errors
    ///
    /// Returns the serializer's error, or an error when the value is not a
    /// JSON object.
    pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: serde::Serialize + TypeTag,
        S: Serializer,
    {
        serde::Serialize::serialize(&Tagged(value), serializer)
    }

    /// Read the value; a `type` field must name `T::TYPE`.
    ///
    /// # Errors
    ///
    /// Fails when the `type` field names another type, or the value does not
    /// deserialize as `T`.
    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
    where
        T: serde::de::DeserializeOwned + TypeTag,
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        super::untag(value).map_err(serde::de::Error::custom)
    }
}

/// `#[serde(with = "crate::wire::typed_vec")]` for a `Vec` of [`TypeTag`]
/// values: each written with its `type` tag (see [`typed`]).
pub mod typed_vec {
    use super::{Deserialize, Deserializer, Serializer, Tagged, TypeTag};

    /// Write each value with its `type` tag first.
    ///
    /// # Errors
    ///
    /// Returns the serializer's error.
    pub fn serialize<T, S>(values: &[T], serializer: S) -> Result<S::Ok, S::Error>
    where
        T: serde::Serialize + TypeTag,
        S: Serializer,
    {
        serializer.collect_seq(values.iter().map(Tagged))
    }

    /// Read each value; a `type` field must name `T::TYPE`.
    ///
    /// # Errors
    ///
    /// Fails when an element's `type` names another type, or an element does
    /// not deserialize as `T`.
    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
    where
        T: serde::de::DeserializeOwned + TypeTag,
        D: Deserializer<'de>,
    {
        Vec::<serde_json::Value>::deserialize(deserializer)?
            .into_iter()
            .map(super::untag)
            .collect::<Result<_, _>>()
            .map_err(serde::de::Error::custom)
    }
}

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
