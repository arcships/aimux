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

use crate::bedrock::event_stream::{EventStreamMessage, decode_stream, encode_message};

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
