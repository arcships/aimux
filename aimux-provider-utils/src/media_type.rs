//! Media type detection and resolution using the provider-utils signatures.

use std::borrow::Cow;

use aimux_core::{content::ContentPart, error::AiMuxError};

/// Inline data accepted by the media type detector.
#[derive(Clone, Copy, Debug)]
pub enum MediaTypeData<'a> {
    Bytes(&'a [u8]),
    Base64(&'a str),
}

/// Largest ID3v2 tag (including its ten-byte header) scanned for an audio frame.
pub const MAX_ID3_TAG_BYTES: usize = 128 * 1024;
const DEFAULT_SNIFF_BYTES: usize = 18;
const MAX_SIGNATURE_BYTES: usize = 12;

// -1 marks a variable byte. Order is significant for overlapping containers.
const SIGNATURES: &[(&str, &[i16])] = &[
    ("image/gif", &[0x47, 0x49, 0x46, 0x38, 0x37, 0x61]),
    ("image/gif", &[0x47, 0x49, 0x46, 0x38, 0x39, 0x61]),
    ("image/png", &[0x89, 0x50, 0x4e, 0x47]),
    ("image/jpeg", &[0xff, 0xd8]),
    (
        "image/webp",
        &[
            0x52, 0x49, 0x46, 0x46, -1, -1, -1, -1, 0x57, 0x45, 0x42, 0x50,
        ],
    ),
    ("image/bmp", &[0x42, 0x4d, -1, -1, -1, -1, 0, 0, 0, 0]),
    ("image/tiff", &[0x49, 0x49, 0x2a, 0]),
    ("image/tiff", &[0x4d, 0x4d, 0, 0x2a]),
    (
        "image/avif",
        &[0, 0, 0, -1, 0x66, 0x74, 0x79, 0x70, 0x61, 0x76, 0x69, 0x66],
    ),
    (
        "image/heic",
        &[0, 0, 0, -1, 0x66, 0x74, 0x79, 0x70, 0x68, 0x65, 0x69, 0x63],
    ),
    ("application/pdf", &[0x25, 0x50, 0x44, 0x46]),
    ("audio/aac", &[0xff, 0xf0]),
    ("audio/aac", &[0xff, 0xf1]),
    ("audio/aac", &[0xff, 0xf8]),
    ("audio/aac", &[0xff, 0xf9]),
    ("audio/mpeg", &[0xff, 0xfb]),
    ("audio/mpeg", &[0xff, 0xfa]),
    ("audio/mpeg", &[0xff, 0xf3]),
    ("audio/mpeg", &[0xff, 0xf2]),
    ("audio/mpeg", &[0xff, 0xe3]),
    ("audio/mpeg", &[0xff, 0xe2]),
    (
        "audio/wav",
        &[
            0x52, 0x49, 0x46, 0x46, -1, -1, -1, -1, 0x57, 0x41, 0x56, 0x45,
        ],
    ),
    ("audio/ogg", &[0x4f, 0x67, 0x67, 0x53]),
    ("audio/flac", &[0x66, 0x4c, 0x61, 0x43]),
    ("audio/aac", &[0x40, 0x15, 0, 0]),
    ("audio/webm", &[0x1a, 0x45, 0xdf, 0xa3]),
    ("video/mp4", &[0, 0, 0, -1, 0x66, 0x74, 0x79, 0x70]),
    ("video/webm", &[0x1a, 0x45, 0xdf, 0xa3]),
    (
        "video/quicktime",
        &[0, 0, 0, 0x14, 0x66, 0x74, 0x79, 0x70, 0x71, 0x74],
    ),
    ("video/x-msvideo", &[0x52, 0x49, 0x46, 0x46]),
    ("audio/mp4", &[0, 0, 0, -1, 0x66, 0x74, 0x79, 0x70]),
];

fn decode_prefix(data: MediaTypeData<'_>, max_bytes: usize) -> Result<Cow<'_, [u8]>, AiMuxError> {
    let data = match data {
        MediaTypeData::Bytes(bytes) => {
            return Ok(Cow::Borrowed(&bytes[..bytes.len().min(max_bytes)]));
        }
        MediaTypeData::Base64(data) => data,
    };
    // Match atob: URL alphabet, ASCII whitespace, optional padding, and
    // noncanonical trailing bits are accepted; malformed input is an error.
    let invalid = || AiMuxError::InvalidArgument("invalid base64 data".into());
    let mut chars = data
        .encode_utf16()
        .take(max_bytes.div_ceil(3) * 4)
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
        return Err(invalid());
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
            _ => return Err(invalid()),
        };
        value = (value << 6) | u32::from(digit);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((value >> bits) as u8);
            value &= (1 << bits) - 1;
        }
    }
    bytes.truncate(max_bytes);
    Ok(Cow::Owned(bytes))
}

/// Detect a known media type, optionally restricted to its top-level segment.
///
/// # Errors
/// Returns an error if the inspected base64 prefix is malformed.
pub fn detect_media_type(
    data: MediaTypeData<'_>,
    top_level_type: Option<&str>,
) -> Result<Option<&'static str>, AiMuxError> {
    if top_level_type
        .is_some_and(|kind| !matches!(kind, "image" | "audio" | "video" | "application"))
    {
        return Ok(None);
    }
    let mut bytes = decode_prefix(data, DEFAULT_SNIFF_BYTES)?;
    let mut offset = 0;
    if bytes.len() > 10 && bytes.starts_with(b"ID3") {
        bytes = decode_prefix(data, MAX_ID3_TAG_BYTES + MAX_SIGNATURE_BYTES)?;
        offset = 10
            + ((usize::from(bytes[6] & 0x7f) << 21)
                | (usize::from(bytes[7] & 0x7f) << 14)
                | (usize::from(bytes[8] & 0x7f) << 7)
                | usize::from(bytes[9] & 0x7f));
    }
    let bytes = &bytes[offset.min(bytes.len())..];
    Ok(SIGNATURES.iter().find_map(|&(media_type, prefix)| {
        let eligible = match top_level_type {
            Some(kind) => get_top_level_media_type(media_type) == kind,
            None => media_type != "audio/mp4",
        };
        (eligible
            && bytes.len() >= prefix.len()
            && prefix
                .iter()
                .zip(bytes)
                .all(|(&expected, &actual)| expected == -1 || expected == i16::from(actual)))
        .then_some(media_type)
    }))
}

/// Return the segment preceding the first slash.
#[must_use]
pub fn get_top_level_media_type(media_type: &str) -> &str {
    media_type
        .split_once('/')
        .map_or(media_type, |(kind, _)| kind)
}

/// Whether a media type has a nonempty subtype other than `*`.
#[must_use]
pub fn is_full_media_type(media_type: &str) -> bool {
    media_type
        .split_once('/')
        .is_some_and(|(_, subtype)| !subtype.is_empty() && subtype != "*")
}

/// Resolve an inline file's partial media type, or preserve its full type.
///
/// # Errors
/// Returns an error for nonfile parts, malformed base64, or partial media types
/// that cannot be detected or whose data is not inline.
pub fn resolve_full_media_type(part: &ContentPart) -> Result<String, AiMuxError> {
    let (media_type, data) = match part {
        ContentPart::Image {
            image, media_type, ..
        } => (media_type, Some(MediaTypeData::Bytes(image))),
        ContentPart::File {
            data, media_type, ..
        } => (media_type, Some(MediaTypeData::Bytes(data))),
        ContentPart::FileBase64 {
            data, media_type, ..
        } => (media_type, Some(MediaTypeData::Base64(data))),
        ContentPart::FileUrl { media_type, .. } | ContentPart::FileReference { media_type, .. } => {
            (media_type, None)
        }
        _ => {
            return Err(AiMuxError::InvalidArgument(
                "media type resolution requires a file part".into(),
            ));
        }
    };
    if is_full_media_type(media_type) {
        return Ok(media_type.clone());
    }
    let reason = if let Some(data) = data {
        if let Some(detected) = detect_media_type(data, Some(get_top_level_media_type(media_type)))?
        {
            return Ok(detected.into());
        }
        "it could not be auto-detected"
    } else {
        "it is not passed as inline bytes"
    };
    Err(AiMuxError::UnsupportedFunctionality(format!(
        "file of media type \"{media_type}\" must specify subtype since {reason}"
    )))
}
