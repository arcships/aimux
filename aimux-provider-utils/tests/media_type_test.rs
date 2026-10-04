use aimux_core::{content::ContentPart, error::AiMuxError};
use aimux_provider_utils::{
    MAX_ID3_TAG_BYTES, MediaTypeData, detect_media_type, get_top_level_media_type,
    is_full_media_type, resolve_full_media_type,
};

#[test]
fn media_type_tables_and_resolution() {
    // Every upstream signature, with nonzero bytes in wildcard positions.
    // Ordered matching deliberately resolves QuickTime as MP4.
    let cases: &[(&str, &[u8], &str, &str, &str)] = &[
        (
            "image",
            &[0x47, 0x49, 0x46, 0x38, 0x37, 0x61],
            "R0lGODdh",
            "image/gif",
            "image/gif",
        ),
        (
            "image",
            &[0x47, 0x49, 0x46, 0x38, 0x39, 0x61],
            "R0lGODlh",
            "image/gif",
            "image/gif",
        ),
        (
            "image",
            &[0x89, 0x50, 0x4e, 0x47],
            "iVBORw==",
            "image/png",
            "image/png",
        ),
        ("image", &[0xff, 0xd8], "/9g=", "image/jpeg", "image/jpeg"),
        (
            "image",
            &[
                0x52, 0x49, 0x46, 0x46, 0xa5, 0xa5, 0xa5, 0xa5, 0x57, 0x45, 0x42, 0x50,
            ],
            "UklGRqWlpaVXRUJQ",
            "image/webp",
            "image/webp",
        ),
        (
            "image",
            &[0x42, 0x4d, 0xa5, 0xa5, 0xa5, 0xa5, 0x00, 0x00, 0x00, 0x00],
            "Qk2lpaWlAAAAAA==",
            "image/bmp",
            "image/bmp",
        ),
        (
            "image",
            &[0x49, 0x49, 0x2a, 0x00],
            "SUkqAA==",
            "image/tiff",
            "image/tiff",
        ),
        (
            "image",
            &[0x4d, 0x4d, 0x00, 0x2a],
            "TU0AKg==",
            "image/tiff",
            "image/tiff",
        ),
        (
            "image",
            &[
                0x00, 0x00, 0x00, 0xa5, 0x66, 0x74, 0x79, 0x70, 0x61, 0x76, 0x69, 0x66,
            ],
            "AAAApWZ0eXBhdmlm",
            "image/avif",
            "image/avif",
        ),
        (
            "image",
            &[
                0x00, 0x00, 0x00, 0xa5, 0x66, 0x74, 0x79, 0x70, 0x68, 0x65, 0x69, 0x63,
            ],
            "AAAApWZ0eXBoZWlj",
            "image/heic",
            "image/heic",
        ),
        (
            "application",
            &[0x25, 0x50, 0x44, 0x46],
            "JVBERg==",
            "application/pdf",
            "application/pdf",
        ),
        ("audio", &[0xff, 0xf0], "//A=", "audio/aac", "audio/aac"),
        ("audio", &[0xff, 0xf1], "//E=", "audio/aac", "audio/aac"),
        ("audio", &[0xff, 0xf8], "//g=", "audio/aac", "audio/aac"),
        ("audio", &[0xff, 0xf9], "//k=", "audio/aac", "audio/aac"),
        ("audio", &[0xff, 0xfb], "//s=", "audio/mpeg", "audio/mpeg"),
        ("audio", &[0xff, 0xfa], "//o=", "audio/mpeg", "audio/mpeg"),
        ("audio", &[0xff, 0xf3], "//M=", "audio/mpeg", "audio/mpeg"),
        ("audio", &[0xff, 0xf2], "//I=", "audio/mpeg", "audio/mpeg"),
        ("audio", &[0xff, 0xe3], "/+M=", "audio/mpeg", "audio/mpeg"),
        ("audio", &[0xff, 0xe2], "/+I=", "audio/mpeg", "audio/mpeg"),
        (
            "audio",
            &[
                0x52, 0x49, 0x46, 0x46, 0xa5, 0xa5, 0xa5, 0xa5, 0x57, 0x41, 0x56, 0x45,
            ],
            "UklGRqWlpaVXQVZF",
            "audio/wav",
            "audio/wav",
        ),
        (
            "audio",
            &[0x4f, 0x67, 0x67, 0x53],
            "T2dnUw==",
            "audio/ogg",
            "audio/ogg",
        ),
        (
            "audio",
            &[0x66, 0x4c, 0x61, 0x43],
            "ZkxhQw==",
            "audio/flac",
            "audio/flac",
        ),
        (
            "audio",
            &[0x40, 0x15, 0x00, 0x00],
            "QBUAAA==",
            "audio/aac",
            "audio/aac",
        ),
        (
            "audio",
            &[0x1a, 0x45, 0xdf, 0xa3],
            "GkXfow==",
            "audio/webm",
            "audio/webm",
        ),
        (
            "audio",
            &[0x00, 0x00, 0x00, 0xa5, 0x66, 0x74, 0x79, 0x70],
            "AAAApWZ0eXA=",
            "audio/mp4",
            "video/mp4",
        ),
        (
            "video",
            &[0x00, 0x00, 0x00, 0xa5, 0x66, 0x74, 0x79, 0x70],
            "AAAApWZ0eXA=",
            "video/mp4",
            "video/mp4",
        ),
        (
            "video",
            &[0x1a, 0x45, 0xdf, 0xa3],
            "GkXfow==",
            "video/webm",
            "audio/webm",
        ),
        (
            "video",
            &[0x00, 0x00, 0x00, 0x14, 0x66, 0x74, 0x79, 0x70, 0x71, 0x74],
            "AAAAFGZ0eXBxdA==",
            "video/mp4",
            "video/mp4",
        ),
        (
            "video",
            &[0x52, 0x49, 0x46, 0x46],
            "UklGRg==",
            "video/x-msvideo",
            "video/x-msvideo",
        ),
    ];
    assert_eq!(cases.len(), 31);
    for &(top, bytes, base64, scoped, generic) in cases {
        for data in [MediaTypeData::Bytes(bytes), MediaTypeData::Base64(base64)] {
            assert_eq!(detect_media_type(data, Some(top)).unwrap(), Some(scoped));
            assert_eq!(detect_media_type(data, None).unwrap(), Some(generic));
        }
        for declared in [top.to_string(), format!("{top}/*"), format!("{top}/")] {
            for part in [
                ContentPart::file(bytes.to_vec(), &declared),
                ContentPart::file_base64(base64, &declared),
            ] {
                assert_eq!(resolve_full_media_type(&part).unwrap(), scoped);
            }
        }
    }

    for (declared, top, full) in [
        ("image/png", "image", true),
        ("image/*", "image", false),
        ("image", "image", false),
        ("image/", "image", false),
        ("", "", false),
        ("/", "", false),
        ("/png", "", true),
        ("image/*/extra", "image", true),
    ] {
        assert_eq!(get_top_level_media_type(declared), top);
        assert_eq!(is_full_media_type(declared), full);
    }

    for bytes in [b"".as_slice(), b"unknown", b"GIF", b"BM", b"RIFF1234NOPE"] {
        assert_eq!(
            detect_media_type(MediaTypeData::Bytes(bytes), Some("image")).unwrap(),
            None
        );
        let part = ContentPart::file(bytes.to_vec(), "image/*");
        assert!(matches!(resolve_full_media_type(&part),
            Err(AiMuxError::UnsupportedFunctionality(message))
            if message == "file of media type \"image/*\" must specify subtype since it could not be auto-detected"
        ));
    }
    assert_eq!(
        detect_media_type(MediaTypeData::Base64("!"), Some("text")).unwrap(),
        None
    );
    assert!(detect_media_type(MediaTypeData::Base64("!"), None).is_err());
    for base64 in ["_9g=", "/9g", "/9h=", " /9g=\n"] {
        assert_eq!(
            detect_media_type(MediaTypeData::Base64(base64), Some("image")).unwrap(),
            Some("image/jpeg")
        );
    }
    for base64 in ["A", "=", "AAAA=", "AA=A", "AA===", "AAAAé"] {
        assert!(
            detect_media_type(MediaTypeData::Base64(base64), None).is_err(),
            "{base64}"
        );
    }

    for part in [
        ContentPart::file(vec![], "custom/type"),
        ContentPart::file_base64("!", "custom/type"),
        ContentPart::file_url("https://example.com/file", "custom/type"),
        ContentPart::file_reference("custom/type", serde_json::json!({"vendor": "file-id"})),
    ] {
        assert_eq!(resolve_full_media_type(&part).unwrap(), "custom/type");
    }
    for part in [
        ContentPart::file_url("https://example.com/file", "image"),
        ContentPart::file_reference("image", serde_json::json!({"vendor": "file-id"})),
    ] {
        assert!(matches!(resolve_full_media_type(&part),
            Err(AiMuxError::UnsupportedFunctionality(message))
            if message == "file of media type \"image\" must specify subtype since it is not passed as inline bytes"
        ));
    }
    assert_eq!(
        resolve_full_media_type(&ContentPart::image(vec![0xff, 0xd8], "image")).unwrap(),
        "image/jpeg"
    );

    // The bounded scan includes twelve bytes beyond the maximum total ID3 tag size.
    for (tag_size, expected) in [
        (0, Some("audio/mpeg")),
        (20, Some("audio/mpeg")),
        (MAX_ID3_TAG_BYTES - 10, Some("audio/mpeg")),
        (MAX_ID3_TAG_BYTES + 1, None),
    ] {
        let mut bytes = vec![0; tag_size + 12];
        bytes[..3].copy_from_slice(b"ID3");
        for i in 0..4 {
            bytes[6 + i] = ((tag_size >> (7 * (3 - i))) & 0x7f) as u8;
        }
        bytes[tag_size + 10..].copy_from_slice(&[0xff, 0xfb]);
        assert_eq!(
            detect_media_type(MediaTypeData::Bytes(&bytes), Some("audio")).unwrap(),
            expected
        );
        let encoded = encode_base64(&bytes);
        assert_eq!(
            detect_media_type(MediaTypeData::Base64(&encoded), Some("audio")).unwrap(),
            expected
        );
    }
}

fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    for chunk in bytes.chunks(3) {
        let bits = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for shift in [18, 12, 6, 0] {
            encoded.push(if shift / 6 < 3 - chunk.len() {
                '='
            } else {
                char::from(ALPHABET[((bits >> shift) & 63) as usize])
            });
        }
    }
    encoded
}
