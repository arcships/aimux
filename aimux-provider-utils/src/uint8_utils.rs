//! Base64 conversion as the AI SDK does it (`uint8-utils.ts`).

use std::borrow::Cow;

use aimux_core::image_model::ImageFileData;
use aimux_core::shared::FileBytes;
use aimux_core::video_model::VideoFileData;
use base64::Engine as _;

/// Base64 text that `atob` does not accept (`convertBase64ToUint8Array`).
/// The message is the one Node's `atob` throws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("The string to be decoded is not correctly encoded.")]
pub struct InvalidBase64Error;

/// Inline data that is either base64 text or bytes, the `string | Uint8Array`
/// that `convertToBase64` takes.
#[derive(Clone, Copy, Debug)]
pub enum Base64OrBytes<'a> {
    /// Base64 text, passed through unchanged.
    Base64(&'a str),
    /// Bytes, encoded.
    Bytes(&'a [u8]),
}

impl<'a> From<&'a str> for Base64OrBytes<'a> {
    fn from(value: &'a str) -> Self {
        Self::Base64(value)
    }
}

impl<'a> From<&'a [u8]> for Base64OrBytes<'a> {
    fn from(value: &'a [u8]) -> Self {
        Self::Bytes(value)
    }
}

impl<'a> From<&'a FileBytes> for Base64OrBytes<'a> {
    fn from(value: &'a FileBytes) -> Self {
        match value {
            FileBytes::Base64(text) => Self::Base64(text),
            FileBytes::Binary(bytes) => Self::Bytes(bytes),
        }
    }
}

impl<'a> From<&'a ImageFileData> for Base64OrBytes<'a> {
    fn from(value: &'a ImageFileData) -> Self {
        match value {
            ImageFileData::Base64(text) => Self::Base64(text),
            ImageFileData::Binary(bytes) => Self::Bytes(bytes),
        }
    }
}

impl<'a> From<&'a VideoFileData> for Base64OrBytes<'a> {
    fn from(value: &'a VideoFileData) -> Self {
        match value {
            VideoFileData::Base64(text) => Self::Base64(text),
            VideoFileData::Binary(bytes) => Self::Bytes(bytes),
        }
    }
}

/// Decode base64 as `convertBase64ToUint8Array` does: `-` and `_` are read
/// as `+` and `/`, then `atob`'s forgiving decode (WHATWG "forgiving-base64
/// decode") drops ASCII whitespace, strips up to two `=` when the length is a
/// multiple of four, and accepts unpadded input and noncanonical trailing
/// bits.
///
/// # Errors
///
/// Returns [`InvalidBase64Error`] for text `atob` rejects: a character
/// outside the alphabet, `=` anywhere but the end of a padded block, or a
/// length that leaves one character over.
pub fn convert_base64_to_uint8_array(
    base64: impl AsRef<str>,
) -> Result<Vec<u8>, InvalidBase64Error> {
    forgiving_decode(base64.as_ref().encode_utf16())
}

/// Decode the UTF-16 code units of base64 text the way
/// [`convert_base64_to_uint8_array`] does.
pub(crate) fn forgiving_decode(
    chars: impl Iterator<Item = u16>,
) -> Result<Vec<u8>, InvalidBase64Error> {
    let mut chars = chars
        .filter(|&c| !matches!(c, 0x09 | 0x0a | 0x0c | 0x0d | 0x20))
        .collect::<Vec<_>>();
    if chars.len().is_multiple_of(4) {
        for _ in 0..2 {
            if chars.last() == Some(&0x3d) {
                chars.pop();
            }
        }
    }
    if chars.len() % 4 == 1 {
        return Err(InvalidBase64Error);
    }
    let mut bytes = Vec::with_capacity(chars.len() * 3 / 4);
    let mut value = 0u32;
    let mut bits = 0;
    for c in chars {
        let digit = match c {
            0x41..=0x5a => c - 0x41,
            0x61..=0x7a => c - 0x61 + 26,
            0x30..=0x39 => c - 0x30 + 52,
            0x2b | 0x2d => 62,
            0x2f | 0x5f => 63,
            _ => return Err(InvalidBase64Error),
        };
        value = (value << 6) | u32::from(digit);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((value >> bits) as u8);
            value &= (1 << bits) - 1;
        }
    }
    Ok(bytes)
}

/// Encode bytes as `convertUint8ArrayToBase64` does (`btoa`: the standard
/// alphabet with padding).
#[must_use]
pub fn convert_uint8_array_to_base64(bytes: impl AsRef<[u8]>) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Base64 text passes through and bytes are encoded (`convertToBase64`).
#[must_use]
pub fn convert_to_base64<'a>(value: impl Into<Base64OrBytes<'a>>) -> Cow<'a, str> {
    match value.into() {
        Base64OrBytes::Base64(text) => Cow::Borrowed(text),
        Base64OrBytes::Bytes(bytes) => Cow::Owned(convert_uint8_array_to_base64(bytes)),
    }
}
