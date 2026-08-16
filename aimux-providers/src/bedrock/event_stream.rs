//! Minimal AWS Event Stream encoder/decoder.
//!
//! Amazon Bedrock's `converse-stream` endpoint returns responses in the AWS
//! event stream binary format (`application/vnd.amazon.eventstream`). Each
//! message is a framed binary record containing headers (notably
//! `:message-type` and `:event-type`) and a JSON payload.
//!
//! This module implements just enough of the format to decode Bedrock stream
//! events and to encode them in tests.
//!
//! Reference: <https://docs.aws.amazon.com/transcribe/latest/dg/event-stream.md>
//!            <https://github.com/smithy-lang/smithy-typescript/tree/main/packages/eventstream-codec>

/// CRC32 lookup table (IEEE polynomial 0xEDB88320).
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFFFFFFu32;
    for &byte in data {
        crc = (crc >> 8) ^ CRC32_TABLE[((crc ^ byte as u32) & 0xFF) as usize];
    }
    crc ^ 0xFFFFFFFF
}

/// A decoded event stream message.
#[derive(Debug, Clone)]
pub struct EventStreamMessage {
    /// The `:message-type` header value (typically `"event"`).
    pub message_type: String,
    /// The `:event-type` header value (e.g. `"contentBlockDelta"`).
    pub event_type: String,
    /// The message payload as a UTF-8 string (typically JSON).
    pub data: String,
}

/// Header value types in the AWS event stream format.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum HeaderValue {
    String(String),
    Bool(bool),
}

/// Encode a single event stream message.
///
/// Returns the binary frame ready to be sent on the wire.
#[must_use]
pub fn encode_message(message_type: &str, event_type: &str, payload: &str) -> Vec<u8> {
    // Headers: :message-type, :event-type, :content-type
    let headers = vec![
        (
            ":message-type",
            HeaderValue::String(message_type.to_string()),
        ),
        (":event-type", HeaderValue::String(event_type.to_string())),
        (
            ":content-type",
            HeaderValue::String("application/json".to_string()),
        ),
    ];

    let mut header_buf = Vec::new();

    for (name, value) in &headers {
        // name length (1 byte) + name + type (1 byte) + value
        header_buf.push(name.len() as u8);
        header_buf.extend_from_slice(name.as_bytes());
        match value {
            HeaderValue::String(s) => {
                header_buf.push(7); // type 7 = string
                header_buf.extend_from_slice(&(s.len() as u16).to_be_bytes());
                header_buf.extend_from_slice(s.as_bytes());
            }
            HeaderValue::Bool(b) => {
                header_buf.push(if *b { 0 } else { 1 }); // type 0/1 = bool true/false
            }
        }
    }

    let payload_bytes = payload.as_bytes();

    // Prelude: total_length(4) + headers_length(4) + prelude_crc(4)
    // Then headers, then payload, then message_crc(4)
    let total_length = (4 + 4 + 4 + header_buf.len() + payload_bytes.len() + 4) as u32;
    let headers_length = header_buf.len() as u32;

    // Prelude CRC: CRC32 of the first 8 bytes (total_length + headers_length)
    let mut prelude = Vec::new();
    prelude.extend_from_slice(&total_length.to_be_bytes());
    prelude.extend_from_slice(&headers_length.to_be_bytes());
    let prelude_crc = crc32(&prelude);

    let mut message = Vec::new();
    message.extend_from_slice(&total_length.to_be_bytes());
    message.extend_from_slice(&headers_length.to_be_bytes());
    message.extend_from_slice(&prelude_crc.to_be_bytes());
    message.extend_from_slice(&header_buf);
    message.extend_from_slice(payload_bytes);
    let msg_crc = crc32(&message);
    message.extend_from_slice(&msg_crc.to_be_bytes());

    message
}

/// Encode multiple messages into a single byte stream.
#[must_use]
pub fn encode_messages(messages: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut buf = Vec::new();
    for (msg_type, event_type, payload) in messages {
        buf.extend_from_slice(&encode_message(msg_type, event_type, payload));
    }
    buf
}

/// Decode all event stream messages from a byte buffer.
///
/// Returns a vector of decoded messages. Malformed frames — including those
/// that fail the prelude CRC or message CRC check — are skipped.
#[must_use]
pub fn decode_messages(data: &[u8]) -> Vec<EventStreamMessage> {
    let mut messages = Vec::new();
    let mut offset = 0;

    while offset + 12 <= data.len() {
        // Read prelude: total_length(4) + headers_length(4)
        let total_length = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;

        if total_length < 16 || offset + total_length > data.len() {
            break;
        }

        // Verify prelude CRC before trusting total_length/headers_length.
        // Prelude = first 8 bytes (total_length + headers_length); CRC at byte 8.
        let prelude_crc_stored = u32::from_be_bytes([
            data[offset + 8],
            data[offset + 9],
            data[offset + 10],
            data[offset + 11],
        ]);
        if crc32(&data[offset..offset + 8]) != prelude_crc_stored {
            break;
        }

        let headers_length = u32::from_be_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]) as usize;

        let header_start = offset + 12; // skip prelude (4+4+4)
        let header_end = header_start + headers_length;
        let payload_start = header_end;
        let payload_end = offset + total_length - 4; // last 4 bytes are CRC

        if header_end > data.len() || payload_end > data.len() {
            break;
        }

        // Parse headers
        let mut message_type = String::new();
        let mut event_type = String::new();
        let mut h_offset = header_start;

        while h_offset + 2 <= header_end {
            let name_len = data[h_offset] as usize;
            h_offset += 1;
            if h_offset + name_len > header_end {
                break;
            }
            let name = String::from_utf8_lossy(&data[h_offset..h_offset + name_len]).to_string();
            h_offset += name_len;
            if h_offset >= header_end {
                break;
            }
            let type_byte = data[h_offset];
            h_offset += 1;

            match type_byte {
                7 => {
                    // String: 2-byte length + value
                    if h_offset + 2 > header_end {
                        break;
                    }
                    let val_len = u16::from_be_bytes([data[h_offset], data[h_offset + 1]]) as usize;
                    h_offset += 2;
                    if h_offset + val_len > header_end {
                        break;
                    }
                    let val =
                        String::from_utf8_lossy(&data[h_offset..h_offset + val_len]).to_string();
                    h_offset += val_len;
                    match name.as_str() {
                        ":message-type" => message_type = val,
                        ":event-type" => event_type = val,
                        _ => {}
                    }
                }
                0 | 1 => {
                    // Bool true/false: no value bytes
                }
                _ => {
                    // Unknown type; skip remaining headers to avoid misparsing.
                    break;
                }
            }
        }

        // Extract payload
        let payload = if payload_end > payload_start {
            String::from_utf8_lossy(&data[payload_start..payload_end]).to_string()
        } else {
            String::new()
        };

        // Verify message CRC (over everything except the trailing 4 CRC bytes).
        let msg_crc_stored = u32::from_be_bytes([
            data[offset + total_length - 4],
            data[offset + total_length - 3],
            data[offset + total_length - 2],
            data[offset + total_length - 1],
        ]);
        if crc32(&data[offset..offset + total_length - 4]) != msg_crc_stored {
            break;
        }

        messages.push(EventStreamMessage {
            message_type,
            event_type,
            data: payload,
        });

        offset += total_length;
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_single_message() {
        let payload = r#"{"contentBlockIndex":0,"delta":{"text":"Hello"}}"#;
        let encoded = encode_message("event", "contentBlockDelta", payload);
        let decoded = decode_messages(&encoded);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].message_type, "event");
        assert_eq!(decoded[0].event_type, "contentBlockDelta");
        assert_eq!(decoded[0].data, payload);
    }

    #[test]
    fn round_trip_multiple_messages() {
        let messages = vec![
            ("event", "messageStart", r#"{"role":"assistant""#),
            (
                "event",
                "contentBlockDelta",
                r#"{"contentBlockIndex":0,"delta":{"text":"Hi"}}"#,
            ),
            ("event", "messageStop", r#"{"stopReason":"end_turn"}"#),
        ];
        let encoded = encode_messages(&messages);
        let decoded = decode_messages(&encoded);
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].event_type, "messageStart");
        assert_eq!(decoded[1].event_type, "contentBlockDelta");
        assert_eq!(decoded[2].event_type, "messageStop");
    }

    #[test]
    fn corrupt_prelude_crc_is_rejected() {
        let payload = r#"{"delta":{"text":"Hi"}}"#;
        let mut encoded = encode_message("event", "contentBlockDelta", payload);
        // Flip a byte in the prelude CRC (bytes 8..12).
        encoded[8] ^= 0xFF;
        let decoded = decode_messages(&encoded);
        assert_eq!(
            decoded.len(),
            0,
            "frame with bad prelude CRC must be dropped"
        );
    }

    #[test]
    fn corrupt_message_crc_is_rejected() {
        let payload = r#"{"delta":{"text":"Hi"}}"#;
        let mut encoded = encode_message("event", "contentBlockDelta", payload);
        // Flip the last byte (part of the trailing message CRC).
        let last = encoded.len() - 1;
        encoded[last] ^= 0xFF;
        let decoded = decode_messages(&encoded);
        assert_eq!(
            decoded.len(),
            0,
            "frame with bad message CRC must be dropped"
        );
    }

    #[test]
    fn corrupt_payload_is_rejected_by_crc() {
        let payload = r#"{"delta":{"text":"Hi"}}"#;
        let mut encoded = encode_message("event", "contentBlockDelta", payload);
        // Flip a byte in the payload region (middle of the frame).
        let mid = encoded.len() / 2;
        encoded[mid] ^= 0xFF;
        let decoded = decode_messages(&encoded);
        assert_eq!(
            decoded.len(),
            0,
            "frame with corrupted payload must be caught by message CRC"
        );
    }

    #[test]
    fn valid_frame_before_corrupt_one_is_kept() {
        // Two frames: first valid, second with a flipped payload byte.
        let good = encode_message("event", "messageStart", r#"{"role":"assistant"}"#);
        let mut bad = encode_message("event", "contentBlockDelta", r#"{"delta":{"text":"x"}}"#);
        let bad_mid = bad.len() / 2;
        bad[bad_mid] ^= 0xFF;
        let mut buf = good.clone();
        buf.extend_from_slice(&bad);
        let decoded = decode_messages(&buf);
        // First frame decodes; decoding stops (break) at the corrupt second frame.
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].event_type, "messageStart");
    }
}
