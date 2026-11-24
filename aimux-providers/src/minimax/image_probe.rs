//! The pixel size of an inline frame image, read from its header.
//!
//! MiniMax rejects a first or last frame outside 256–5760 px per side or
//! outside a 0.4–2.5 aspect ratio. Checking that before the create request
//! turns a billable round trip into a local error. Only the header formats
//! that are cheap to read are probed (PNG, JPEG, GIF, WebP); for HEIC/HEIF or
//! an unreadable header the size is unknown and MiniMax validates it.

/// `(width, height)` of a PNG, JPEG, GIF or WebP image, when its header can
/// be read.
pub(super) fn image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    png(data)
        .or_else(|| jpeg(data))
        .or_else(|| gif(data))
        .or_else(|| webp(data))
}

fn be_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn be_u16(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes(
        data.get(at..at + 2)?.try_into().ok()?,
    )))
}

fn le_u16(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes(
        data.get(at..at + 2)?.try_into().ok()?,
    )))
}

fn le_u24(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at + 3)?;
    Some(u32::from(bytes[0]) | u32::from(bytes[1]) << 8 | u32::from(bytes[2]) << 16)
}

/// Signature, then the `IHDR` chunk: width and height at bytes 16 and 20.
fn png(data: &[u8]) -> Option<(u32, u32)> {
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") || data.get(12..16)? != b"IHDR" {
        return None;
    }
    Some((be_u32(data, 16)?, be_u32(data, 20)?))
}

/// Walk the marker segments to the first start-of-frame (SOF0–SOF15, except
/// DHT, JPG and DAC), whose payload holds height then width.
fn jpeg(data: &[u8]) -> Option<(u32, u32)> {
    if !data.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut at = 2;
    loop {
        // Fill bytes before a marker are allowed.
        while *data.get(at)? == 0xFF && *data.get(at + 1)? == 0xFF {
            at += 1;
        }
        if *data.get(at)? != 0xFF {
            return None;
        }
        let marker = *data.get(at + 1)?;
        // Standalone markers carry no length.
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            at += 2;
            continue;
        }
        let length = usize::try_from(be_u16(data, at + 2)?).ok()?;
        if length < 2 {
            return None;
        }
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            return Some((be_u16(data, at + 7)?, be_u16(data, at + 5)?));
        }
        at += 2 + length;
    }
}

/// `GIF87a` / `GIF89a`, then the logical screen width and height.
fn gif(data: &[u8]) -> Option<(u32, u32)> {
    if !(data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")) {
        return None;
    }
    Some((le_u16(data, 6)?, le_u16(data, 8)?))
}

/// `RIFF....WEBP` and its first chunk: lossy `VP8 `, lossless `VP8L` or
/// extended `VP8X`.
fn webp(data: &[u8]) -> Option<(u32, u32)> {
    if data.get(0..4)? != b"RIFF" || data.get(8..12)? != b"WEBP" {
        return None;
    }
    match data.get(12..16)? {
        b"VP8 " => {
            // Frame tag (3 bytes), start code 9d 01 2a, then 14-bit sizes.
            if data.get(23..26)? != [0x9D, 0x01, 0x2A] {
                return None;
            }
            Some((le_u16(data, 26)? & 0x3FFF, le_u16(data, 28)? & 0x3FFF))
        }
        b"VP8L" => {
            if *data.get(20)? != 0x2F {
                return None;
            }
            let bits = u32::from_le_bytes(data.get(21..25)?.try_into().ok()?);
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        b"VP8X" => Some((le_u24(data, 24)? + 1, le_u24(data, 27)? + 1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        data.extend_from_slice(&width.to_be_bytes());
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&[8, 6, 0, 0, 0]);
        data
    }

    #[test]
    fn reads_a_png_header() {
        assert_eq!(image_dimensions(&png_header(1280, 720)), Some((1280, 720)));
    }

    #[test]
    fn reads_a_jpeg_start_of_frame_after_other_segments() {
        let mut data = vec![0xFF, 0xD8];
        // APP0 with a 16-byte payload.
        data.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
        data.extend_from_slice(&[0; 14]);
        // DHT is not a frame header even though it is in the C0..CF range.
        data.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x04, 0x00, 0x00]);
        // SOF2: length, precision, height 600, width 800.
        data.extend_from_slice(&[0xFF, 0xC2, 0x00, 0x11, 0x08, 0x02, 0x58, 0x03, 0x20]);
        assert_eq!(image_dimensions(&data), Some((800, 600)));
    }

    #[test]
    fn reads_gif_and_webp_headers() {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[0x00, 0x04, 0x00, 0x03]);
        assert_eq!(image_dimensions(&gif), Some((1024, 768)));

        let mut vp8x = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
        vp8x.extend_from_slice(&[0xFF, 0x03, 0x00, 0xFF, 0x02, 0x00]);
        assert_eq!(image_dimensions(&vp8x), Some((1024, 768)));

        let mut vp8l = b"RIFF\0\0\0\0WEBPVP8L\0\0\0\0\x2f".to_vec();
        let bits: u32 = (640 - 1) | ((480 - 1) << 14);
        vp8l.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(image_dimensions(&vp8l), Some((640, 480)));

        let mut vp8 = b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0\0\0\0\x9d\x01\x2a".to_vec();
        vp8.extend_from_slice(&[0x80, 0x02, 0xE0, 0x01]);
        assert_eq!(image_dimensions(&vp8), Some((640, 480)));
    }

    #[test]
    fn an_unknown_or_truncated_header_has_no_size() {
        assert_eq!(image_dimensions(b"not an image"), None);
        assert_eq!(image_dimensions(&png_header(1, 1)[..18]), None);
        assert_eq!(image_dimensions(&[0xFF, 0xD8, 0xFF]), None);
    }
}
