//! Ports of `@ai-sdk/amazon-bedrock` 5.0.100
//! `src/amazon-bedrock-event-stream-decoder.test.ts`.
//!
//! `createAmazonBedrockEventStreamDecoder(body, processEvent)` is
//! `event_stream::decode_stream(body)`: it yields each decoded frame (or the
//! decoding error) instead of handing it to a `processEvent` callback, so the
//! cases collect what it yields. The upstream "processEvent" that enqueues
//! `event.data` is a `map` over the yielded messages.

use bytes::Bytes;
use futures::StreamExt;
use futures::stream::BoxStream;

use aimux_core::error::AiMuxError;

use crate::bedrock::event_stream_decoder::{EventStreamMessage, decode_stream, encode_message};

/// `createEvent(data)`: one `event` frame of event type `chunk`.
fn create_event(data: &str) -> Vec<u8> {
    encode_message("event", "chunk", data)
}

/// `createStream(chunks)`: a body that delivers `chunks` and ends.
fn create_stream(chunks: Vec<Vec<u8>>) -> BoxStream<'static, Result<Bytes, AiMuxError>> {
    futures::stream::iter(chunks.into_iter().map(|chunk| Ok(Bytes::from(chunk)))).boxed()
}

/// Everything the decoder yields for `chunks`.
async fn decode_all(chunks: Vec<Vec<u8>>) -> Vec<Result<EventStreamMessage, AiMuxError>> {
    decode_stream(create_stream(chunks)).collect().await
}

/// The `data` of every decoded frame, failing on the first error.
fn data_of(items: Vec<Result<EventStreamMessage, AiMuxError>>) -> Vec<String> {
    items
        .into_iter()
        .map(|item| item.expect("the frame decodes").data)
        .collect()
}

mod create_amazon_bedrock_event_stream_decoder {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-event-stream-decoder.test.ts › createAmazonBedrockEventStreamDecoder › surfaces frame decode errors instead of stranding later frames
    #[tokio::test]
    async fn surfaces_frame_decode_errors_instead_of_stranding_later_frames() {
        let mut corrupted_frame = create_event("corrupted");
        let last = corrupted_frame.len() - 1;
        corrupted_frame[last] ^= 0xff;

        let items = decode_all(vec![corrupted_frame, create_event("later")]).await;

        // The stream rejects, and no frame reaches the event processing.
        assert!(
            items.iter().any(Result::is_err),
            "the decode error is surfaced: {items:?}"
        );
        assert!(
            items.iter().all(Result::is_err),
            "no frame is processed: {items:?}"
        );
    }

    // Not ported (sub-feature): "surfaces processEvent errors". `decode_stream`
    // takes no `processEvent` callback; its consumer is the chat stream's
    // `Transformer`, which reports problems as stream parts and has no
    // throwing callback whose error the decoder could surface.

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-event-stream-decoder.test.ts › createAmazonBedrockEventStreamDecoder › processes frames split across chunks
    #[tokio::test]
    async fn processes_frames_split_across_chunks() {
        let frame = create_event("data");
        let midpoint = frame.len() / 2;

        let items = decode_all(vec![frame[..midpoint].to_vec(), frame[midpoint..].to_vec()]).await;

        assert_eq!(data_of(items), vec!["data".to_string()]);
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-event-stream-decoder.test.ts › createAmazonBedrockEventStreamDecoder › rejects when EOF leaves an incomplete frame
    #[tokio::test]
    async fn rejects_when_eof_leaves_an_incomplete_frame() {
        let frame = create_event("data");
        let incomplete_frame = frame[..frame.len() - 1].to_vec();
        let incomplete_length = incomplete_frame.len();

        let items = decode_all(vec![incomplete_frame]).await;

        let error = items
            .into_iter()
            .find_map(Result::err)
            .expect("the stream rejects");
        let expected = format!(
            "Incomplete Amazon Bedrock event-stream frame: {incomplete_length} buffered bytes remain at end of stream."
        );
        assert!(
            error.to_string().contains(&expected),
            "unexpected error: {error}"
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-event-stream-decoder.test.ts › createAmazonBedrockEventStreamDecoder › allows EOF at a complete frame boundary
    #[tokio::test]
    async fn allows_eof_at_a_complete_frame_boundary() {
        let items = decode_all(vec![create_event("data")]).await;

        assert_eq!(data_of(items), vec!["data".to_string()]);
    }
}

// ── Frame length limit (#399; not upstream: the AI SDK decoder has no limit) ──

mod frame_length_limit {
    use std::time::Duration;

    use super::*;
    use crate::bedrock::event_stream_decoder::crc32;

    /// `AWS_EVENT_STREAM_MAX_MESSAGE_SIZE` in `aws-c-event-stream`.
    const AWS_MAX_FRAME: usize = 24 * 1024 * 1024;

    /// The 12-byte prelude of a frame that declares `total_length` bytes and
    /// no headers, with a valid prelude checksum.
    fn prelude(total_length: usize) -> Vec<u8> {
        let mut bytes = u32::try_from(total_length)
            .expect("the length fits the 4-byte field")
            .to_be_bytes()
            .to_vec();
        bytes.extend_from_slice(&0u32.to_be_bytes());
        let checksum = crc32(&bytes);
        bytes.extend_from_slice(&checksum.to_be_bytes());
        bytes
    }

    /// The first item decoded from `head` on a body that then stays open, or
    /// `None` when the decoder is still waiting for bytes after a second.
    async fn first_item_on_an_open_body(
        head: Vec<u8>,
    ) -> Option<Result<EventStreamMessage, AiMuxError>> {
        let body = futures::stream::iter(vec![Ok::<_, AiMuxError>(Bytes::from(head))])
            .chain(futures::stream::pending())
            .boxed();
        tokio::time::timeout(Duration::from_secs(1), decode_stream(body).next())
            .await
            .ok()
            .flatten()
    }

    fn assert_frame_length_error(item: Option<Result<EventStreamMessage, AiMuxError>>) {
        let error = item
            .expect("the decoder answers without waiting for the frame")
            .expect_err("the declared length is rejected");
        assert!(
            error
                .to_string()
                .contains("Invalid Amazon Bedrock event-stream frame length"),
            "unexpected error: {error}"
        );
    }

    /// The reproduction in #399: a 64 MiB length and then nothing.
    #[tokio::test]
    async fn rejects_a_64_mib_length_before_buffering_the_frame() {
        let declared = 64u32 * 1024 * 1024;

        let item = first_item_on_an_open_body(declared.to_be_bytes().to_vec()).await;

        assert_frame_length_error(item);
    }

    #[tokio::test]
    async fn rejects_a_length_one_byte_past_the_aws_maximum() {
        let item = first_item_on_an_open_body(prelude(AWS_MAX_FRAME + 1)).await;

        assert_frame_length_error(item);
    }

    /// Lengths up to the AWS maximum, those past 16 MiB included, are valid:
    /// the decoder waits for the rest of the frame, so a body that ends after
    /// the prelude fails as an incomplete frame, not on its length.
    #[tokio::test]
    async fn waits_for_the_rest_of_a_frame_up_to_the_aws_maximum() {
        for length in [16 * 1024 * 1024 + 1, AWS_MAX_FRAME] {
            let prelude = prelude(length);
            let buffered = prelude.len();

            let items = decode_all(vec![prelude]).await;

            let error = items
                .into_iter()
                .find_map(Result::err)
                .expect("the stream rejects");
            let expected = format!(
                "Incomplete Amazon Bedrock event-stream frame: {buffered} buffered bytes remain at end of stream."
            );
            assert!(
                error.to_string().contains(&expected),
                "length {length}: unexpected error: {error}"
            );
        }
    }
}

// ── Older aimux tests (previously the unit tests of `bedrock/event_stream.rs`) ──

mod frame_round_trip_and_crc {
    use crate::bedrock::event_stream_decoder::{decode_messages, encode_message, encode_messages};

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

    /// partial: `surfaces frame decode errors instead of stranding later frames`; full port in src/bedrock/tests/event_stream_decoder.rs.
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
