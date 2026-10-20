//! Ports of `@ai-sdk/amazon-bedrock` 5.0.100
//! `src/amazon-bedrock-event-stream-response-handler.test.ts`.
//!
//! `createAmazonBedrockEventStreamResponseHandler(chunkSchema)` decodes the
//! event-stream body and parses each frame's JSON as
//! `{ [eventType]: payload }`. In aimux these steps are split: the response
//! handler of `BedrockModel::do_stream` hands the body on as bytes,
//! `event_stream::decode_stream` decodes the frames, and the chat stream's
//! `Transformer` parses the JSON (emitting the wrapped value as the `raw`
//! part, and a parse failure as an `error` part). So the cases run through
//! `do_stream` of a directly built `BedrockModel` (a fixed endpoint over the
//! mock transport) with raw chunks on.
//!
//! Upstream mocks `EventStreamCodec.decode`, so its frames are dummy bytes
//! with only a valid length prefix. The port sends real frames (encoded with
//! `event_stream::encode_message`) carrying the decoded messages upstream's
//! mock returns.

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::json;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, TextPart, UserPart};
use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use aimux_provider_utils::HeaderMapOpt;

use crate::bedrock::BedrockModel;
use crate::bedrock::event_stream_decoder::encode_message;
use crate::shared::{Endpoint, EndpointConfig, ResolveEndpoint};
use crate::test_support::mock_fetch::{Canned, MockFetch};

/// A fixed base URL and no headers.
struct FixedEndpoint;

#[async_trait]
impl ResolveEndpoint for FixedEndpoint {
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        Ok(Endpoint {
            base_url: "https://bedrock-runtime.us-east-1.amazonaws.com".to_string(),
            headers: HeaderMapOpt::new(),
        })
    }
}

/// A `200` event-stream response with `body`.
fn event_stream_response(body: Vec<u8>) -> Canned {
    Canned {
        status: 200,
        headers: vec![(
            "content-type".to_string(),
            "application/vnd.amazon.eventstream".to_string(),
        )],
        body,
    }
}

/// Everything the stream yields when the response body is `body`.
async fn stream_parts(body: Vec<u8>) -> Vec<Result<StreamPart, AiMuxError>> {
    let mock = MockFetch::new(vec![event_stream_response(body)]);
    let model = BedrockModel::from_config(
        "anthropic.claude-3-haiku-20240307-v1:0".to_string(),
        EndpointConfig::new(
            "amazon-bedrock".to_string(),
            FixedEndpoint,
            Some(mock.transport()),
        ),
    );
    let mut options = CallOptions::new(vec![LanguageModelMessage::User {
        content: vec![UserPart::Text(TextPart {
            text: "Hello".into(),
            provider_options: None,
        })],
        provider_options: None,
    }]);
    options.include_raw_chunks = Some(true);
    let result = model.do_stream(&options).await.expect("the stream starts");
    result.stream.collect().await
}

/// A frame `{ ':message-type': 'event', ':event-type': 'chunk' }` with `body`.
fn chunk_frame(body: &str) -> Vec<u8> {
    encode_message("event", "chunk", body)
}

mod create_event_source_response_handler {
    use super::*;

    // Not ported (ts-only): "throws EmptyResponseBodyError when response body
    // is null". A `FetchResponse` always has a body stream (an empty one for
    // no content); there is no null body to reject.

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-event-stream-response-handler.test.ts › createEventSourceResponseHandler › successfully processes valid event stream data
    ///
    /// The `rawValue` of the parse result is the `raw` part. aimux has no
    /// schema-validated `value` to compare: it does not validate chunks
    /// against a schema.
    #[tokio::test]
    async fn successfully_processes_valid_event_stream_data() {
        let parts = stream_parts(chunk_frame(r#"{"content":"test message"}"#)).await;

        let raw_values: Vec<_> = parts
            .iter()
            .filter_map(|part| match part {
                Ok(StreamPart::Raw { raw_value }) => Some(raw_value.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            raw_values,
            vec![json!({ "chunk": { "content": "test message" } })],
            "{parts:?}"
        );
        assert!(parts.iter().all(Result::is_ok), "{parts:?}");
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-event-stream-response-handler.test.ts › createEventSourceResponseHandler › handles invalid JSON data
    ///
    /// The failed parse result (`success: false` with an `error`) is an
    /// `error` part.
    #[tokio::test]
    async fn handles_invalid_json_data() {
        let parts = stream_parts(chunk_frame("invalid json")).await;

        assert!(
            parts
                .iter()
                .any(|part| matches!(part, Ok(StreamPart::Error { .. }))),
            "{parts:?}"
        );
        assert!(
            !parts
                .iter()
                .any(|part| matches!(part, Ok(StreamPart::Raw { .. }))),
            "{parts:?}"
        );
    }

    // Not ported (sub-feature): "handles schema validation failures". The
    // response handler takes no chunk schema (upstream's `chunkSchema`
    // parameter): the chat stream reads the event payloads leniently, so a
    // payload that does not match a schema is not reported.

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-event-stream-response-handler.test.ts › createEventSourceResponseHandler › surfaces event stream decoding failures
    ///
    /// Upstream's mocked codec throws on `createFrame([13, 14])`. The real
    /// decoder rejects the same bytes (a six-byte frame is too short to be
    /// one), and the stream ends with that error.
    #[tokio::test]
    async fn surfaces_event_stream_decoding_failures() {
        let frame = vec![0, 0, 0, 6, 13, 14];

        let parts = stream_parts(frame).await;

        let last = parts.last().expect("the stream yields");
        let error = last
            .as_ref()
            .expect_err("the stream ends with the decode error");
        assert!(
            error
                .to_string()
                .contains("Invalid Amazon Bedrock event-stream frame"),
            "unexpected error: {error}"
        );
        assert!(
            !parts
                .iter()
                .any(|part| matches!(part, Ok(StreamPart::Raw { .. }))),
            "{parts:?}"
        );
    }
}
