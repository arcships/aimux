//! Upstream `xai-transcription-model.test.ts`: the provider-information case,
//! the `doGenerate` cases and the `doStream` cases.
//!
//! `doGenerate` runs through a [`MockFetch`]. `doStream` runs through an
//! in-crate WebSocket double injected with the `web_socket` setting (the
//! `realtime` feature, on by default). It stands in for the upstream
//! `MockWebSocket`: it records the handshake (URL and headers) and every frame
//! the model sends, and the test feeds the server's events into it.
//!
//! Mapping notes:
//! - `specificationVersion`: `TranscriptionModel::specification_version()` is
//!   the trait's `"v4"`, which xAI keeps.
//! - The injected `_internal.currentDate` has no counterpart (aimux stamps
//!   `Timestamp::now()` and has no clock seam): each stamp is checked to fall
//!   within its call.
//! - The multipart boundary: undici's `----formdata-undici-N` is a JS-only
//!   format. Only the `multipart/form-data; boundary=` prefix is checked.
//! - The user agent carries the pinned `ai-sdk-xai/<version>` as a token. The
//!   upstream `VERSION` mock (`0.0.0-test`) has no counterpart, and the wire
//!   request also carries `ai-sdk-provider-utils/<version>`, which upstream's
//!   provider-level header check never sees.
//! - Per-call headers are `HashMap<String, String>`, so upstream's per-call
//!   `'X-Unset': undefined` is given at provider level, where `None` removes a
//!   header.
//! - `argument: 'providerOptions'`: aimux's `InvalidArgument` carries only a
//!   message, so the message is checked.

use std::collections::HashMap;
use std::sync::Arc;

use aimux_core::shared::SharedProviderOptions;
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use serde_json::{Value, json};

use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};
use crate::xai::XaiTranscriptionModel;
use crate::xai::options::NAMESPACE;
use crate::{XAIProviderSettings, create_xai};

/// `createXai({ apiKey: 'test-api-key', headers }).transcription()`, sending
/// through `fetch`.
fn transcription_model(
    fetch: &Arc<MockFetch>,
    headers: Option<HashMap<String, Option<String>>>,
) -> XaiTranscriptionModel {
    create_xai(XAIProviderSettings {
        api_key: Some("test-api-key".to_string()),
        headers,
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .expect("creating an xAI provider does not fail")
    .transcription()
}

/// `{ audio: new Uint8Array([1, 2, 3, 4]), mediaType }`.
fn audio_call(media_type: &str) -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(vec![1, 2, 3, 4]), media_type)
}

/// `providerOptions: { xai: {...} }`.
fn xai_options(value: Value) -> SharedProviderOptions {
    SharedProviderOptions::from([(
        NAMESPACE.to_string(),
        value.as_object().expect("an object").clone(),
    )])
}

/// The upstream `prepareJsonResponse` answer: a JSON body with extra headers.
fn json_answer(body: Value, headers: &[(&str, &str)]) -> Canned {
    let mut all = vec![("content-type".to_string(), "application/json".to_string())];
    all.extend(
        headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string())),
    );
    Canned {
        status: 200,
        headers: all,
        body: serde_json::to_vec(&body).expect("the body serializes"),
    }
}

/// The body of the upstream `prepareJsonResponse`.
fn default_body() -> Value {
    json!({
        "text": "Hello from the AI SDK!",
        "language": "en",
        "duration": 2.5,
        "words": [
            { "text": "Hello", "start": 0, "end": 1 },
            { "text": "from the AI SDK!", "start": 1, "end": 2.5 },
        ],
    })
}

/// The one request `fetch` recorded.
fn only_request(fetch: &MockFetch) -> Seen {
    let mut seen = fetch.seen();
    assert_eq!(seen.len(), 1, "the call sends exactly one request");
    seen.remove(0)
}

/// The user agent carries the pinned `ai-sdk-xai/<version>` token; see the
/// module notes.
fn assert_user_agent_names_the_package(seen: &Seen) {
    let agent = seen
        .headers
        .get("user-agent")
        .expect("the request has a user-agent header");
    let token = format!("ai-sdk-xai/{}", crate::shared::aisdk_pins::XAI.version);
    assert!(
        agent.split(' ').any(|part| part == token),
        "user-agent {agent:?} does not carry {token}"
    );
}

mod xai_transcription_model {
    use super::*;

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › XaiTranscriptionModel › should expose correct provider and model information
    #[test]
    fn should_expose_correct_provider_and_model_information() {
        let model = create_xai(XAIProviderSettings::default())
            .expect("creating an xAI provider does not fail")
            .transcription();

        assert_eq!(model.provider(), "xai.transcription");
        assert_eq!(model.model_id(), "");
        assert_eq!(model.specification_version(), "v4");
    }
}

mod do_generate {
    use super::*;

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doGenerate › should pass headers and the xAI user agent
    #[tokio::test]
    async fn should_pass_headers_and_the_xai_user_agent() {
        let fetch = MockFetch::new(vec![json_answer(default_body(), &[])]);
        let provider_headers = HashMap::from([(
            "Custom-Provider-Header".to_string(),
            Some("provider-header-value".to_string()),
        )]);
        let mut options = audio_call("audio/wav");
        options.headers = Some(HashMap::from([(
            "Custom-Request-Header".to_string(),
            "request-header-value".to_string(),
        )]));

        transcription_model(&fetch, Some(provider_headers))
            .do_generate(&options)
            .await
            .expect("the call succeeds");

        let seen = only_request(&fetch);
        assert_eq!(seen.headers["authorization"], "Bearer test-api-key");
        assert!(
            seen.headers["content-type"].starts_with("multipart/form-data; boundary="),
            "{}",
            seen.headers["content-type"]
        );
        assert_eq!(
            seen.headers["custom-provider-header"],
            "provider-header-value"
        );
        assert_eq!(
            seen.headers["custom-request-header"],
            "request-header-value"
        );
        assert_user_agent_names_the_package(&seen);
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doGenerate › should include response timestamp, model id, and headers
    #[tokio::test]
    async fn should_include_response_timestamp_model_id_and_headers() {
        let fetch = MockFetch::new(vec![json_answer(
            default_body(),
            &[
                ("x-request-id", "test-request-id"),
                ("x-ratelimit-remaining", "123"),
            ],
        )]);

        let before = chrono::Utc::now().timestamp_millis();
        let result = transcription_model(&fetch, None)
            .do_generate(&audio_call("audio/wav"))
            .await
            .expect("the call succeeds");
        let after = chrono::Utc::now().timestamp_millis();

        let timestamp = result
            .response
            .timestamp
            .expect("the response has a timestamp")
            .unix_millis();
        assert!(
            before <= timestamp && timestamp <= after,
            "timestamp {timestamp} is outside the call ({before}..={after})"
        );
        assert_eq!(result.response.model_id.as_deref(), Some(""));
        let headers = result.response.headers.expect("the response has headers");
        assert_eq!(headers["content-type"], "application/json");
        assert_eq!(headers["x-request-id"], "test-request-id");
        assert_eq!(headers["x-ratelimit-remaining"], "123");
    }
}

#[cfg(feature = "realtime")]
mod socket {
    //! The WebSocket double: the upstream `MockWebSocket`, as a
    //! [`WsTransport`](aimux_provider_utils::ws::WsTransport) that a
    //! [`WsConnector`] hands out.

    use std::collections::VecDeque;
    use std::io;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    use aimux_core::error::AiMuxError;
    use aimux_core::transcription_model::AudioChunk;
    use aimux_provider_utils::ws::{WebSocketRequest, WsConnection, WsConnector};
    use async_trait::async_trait;
    use futures::{Sink, Stream};
    use serde_json::Value;
    use tokio::sync::mpsc;
    use tokio_tungstenite::tungstenite::{Error as WsError, Message};

    /// A frame the model sent, as the upstream `ws.send` mock records it.
    #[derive(Debug, Clone, PartialEq)]
    pub(super) enum Frame {
        Binary(Vec<u8>),
        Text(String),
        Close,
    }

    #[derive(Default)]
    struct SocketState {
        frames: Vec<Frame>,
        /// Upstream's `send` throwing for binary audio.
        fail_binary: bool,
    }

    /// The test's side of one socket: the server events it sends, and what
    /// the model sent.
    pub(super) struct SocketHandle {
        state: Arc<Mutex<SocketState>>,
        events: mpsc::UnboundedSender<Message>,
    }

    impl SocketHandle {
        /// Every frame the model has sent so far.
        pub(super) fn frames(&self) -> Vec<Frame> {
            self.state.lock().unwrap().frames.clone()
        }

        /// Upstream `ws.message(value)`: a server event.
        pub(super) fn message(&self, value: Value) {
            self.events
                .send(Message::Text(value.to_string()))
                .expect("the socket is open");
        }

        /// Upstream's `send` throwing for binary audio from now on.
        pub(super) fn fail_binary_sends(&self) {
            self.state.lock().unwrap().fail_binary = true;
        }
    }

    /// The socket a connector hands out: server events in, frames out.
    pub(super) struct MockSocket {
        state: Arc<Mutex<SocketState>>,
        events: mpsc::UnboundedReceiver<Message>,
    }

    /// A connected socket and the handle the test drives it with.
    pub(super) fn new_socket() -> (SocketHandle, MockSocket) {
        let state = Arc::new(Mutex::new(SocketState::default()));
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        (
            SocketHandle {
                state: Arc::clone(&state),
                events: events_tx,
            },
            MockSocket {
                state,
                events: events_rx,
            },
        )
    }

    impl Stream for MockSocket {
        type Item = Result<Message, WsError>;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            self.get_mut()
                .events
                .poll_recv(cx)
                .map(|event| event.map(Ok))
        }
    }

    impl Sink<Message> for MockSocket {
        type Error = WsError;

        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), WsError>> {
            Poll::Ready(Ok(()))
        }

        fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), WsError> {
            let mut state = self.state.lock().unwrap();
            let frame = match item {
                Message::Binary(bytes) => {
                    if state.fail_binary {
                        return Err(WsError::Io(io::Error::other("send failed")));
                    }
                    Frame::Binary(bytes)
                }
                Message::Text(text) => Frame::Text(text),
                Message::Close(_) => Frame::Close,
                _ => return Ok(()),
            };
            state.frames.push(frame);
            Ok(())
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), WsError>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), WsError>> {
            Poll::Ready(Ok(()))
        }
    }

    /// One connect: the handshake URL and its headers.
    pub(super) type Connect = (String, Vec<(String, String)>);

    /// The connector the test injects. It records each connect (URL and
    /// headers) and hands out its socket, or fails with its message.
    pub(super) struct MockConnector {
        connects: Mutex<Vec<Connect>>,
        socket: Mutex<Option<MockSocket>>,
        fail: Option<String>,
    }

    impl MockConnector {
        /// A connector that hands `socket` out on its connect.
        pub(super) fn with_socket(socket: MockSocket) -> Arc<Self> {
            Arc::new(Self {
                connects: Mutex::default(),
                socket: Mutex::new(Some(socket)),
                fail: None,
            })
        }

        /// A connector whose every connect fails with `message` (upstream's
        /// `new WebSocket()` throwing).
        pub(super) fn failing(message: &str) -> Arc<Self> {
            Arc::new(Self {
                connects: Mutex::default(),
                socket: Mutex::new(None),
                fail: Some(message.to_string()),
            })
        }

        /// Every connect so far: the URL and the headers.
        pub(super) fn connects(&self) -> Vec<Connect> {
            self.connects.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl WsConnector for MockConnector {
        async fn connect(&self, request: &WebSocketRequest) -> Result<WsConnection, AiMuxError> {
            self.connects
                .lock()
                .unwrap()
                .push((request.url.clone(), request.headers.clone()));
            if let Some(message) = &self.fail {
                return Err(AiMuxError::Other(message.clone()));
            }
            let socket = self
                .socket
                .lock()
                .unwrap()
                .take()
                .expect("one connection per test");
            Ok(WsConnection::from_transport(Box::new(socket), request))
        }
    }

    /// The audio the model reads: `chunks`, then the end when `ends`, else an
    /// open stream that yields nothing more. Dropping it is the cancellation
    /// the upstream `cancel()` callback observes; the flag records it.
    pub(super) struct Audio {
        chunks: VecDeque<AudioChunk>,
        ends: bool,
        cancelled: Arc<AtomicBool>,
    }

    impl Audio {
        /// The audio, and the flag that is set once it is dropped.
        pub(super) fn new(chunks: Vec<Vec<u8>>, ends: bool) -> (Self, Arc<AtomicBool>) {
            let cancelled = Arc::new(AtomicBool::new(false));
            let audio = Self {
                chunks: chunks.into_iter().map(AudioChunk::Binary).collect(),
                ends,
                cancelled: Arc::clone(&cancelled),
            };
            (audio, cancelled)
        }
    }

    impl Stream for Audio {
        type Item = AudioChunk;

        fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<AudioChunk>> {
            let this = self.get_mut();
            if let Some(chunk) = this.chunks.pop_front() {
                Poll::Ready(Some(chunk))
            } else if this.ends {
                Poll::Ready(None)
            } else {
                Poll::Pending
            }
        }
    }

    impl Drop for Audio {
        fn drop(&mut self) {
            self.cancelled.store(true, Ordering::SeqCst);
        }
    }
}

#[cfg(feature = "realtime")]
mod do_stream {
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use aimux_core::error::AiMuxError;
    use aimux_core::transcription_model::{
        InputAudioFormat, TranscriptionStreamOptions, TranscriptionStreamPart,
    };
    use futures::StreamExt;
    use tokio::task::JoinHandle;

    use super::socket::{Audio, Frame, MockConnector, new_socket};
    use super::*;

    /// `createXai({ apiKey, headers, webSocket }).transcription()`.
    fn transcription_model_with(
        connector: Arc<MockConnector>,
        headers: Option<HashMap<String, Option<String>>>,
    ) -> XaiTranscriptionModel {
        create_xai(XAIProviderSettings {
            api_key: Some("test-api-key".to_string()),
            headers,
            web_socket: Some(connector),
            ..Default::default()
        })
        .expect("creating an xAI provider does not fail")
        .transcription()
    }

    /// `{ audio, inputAudioFormat: { type: 'audio/pcm', rate: 16000 }, providerOptions }`.
    fn stream_options(audio: Audio, provider_options: Option<Value>) -> TranscriptionStreamOptions {
        TranscriptionStreamOptions {
            audio: Box::pin(audio),
            input_audio_format: InputAudioFormat {
                format_type: "audio/pcm".to_string(),
                rate: Some(16000),
            },
            provider_options: provider_options.map(xai_options),
            abort_signal: None,
            headers: None,
            include_raw_chunks: false,
            timeout: None,
        }
    }

    /// The parts as JSON; a stream error fails the test.
    fn part_values(parts: &[Result<TranscriptionStreamPart, AiMuxError>]) -> Vec<Value> {
        parts
            .iter()
            .map(|part| serde_json::to_value(part.as_ref().expect("a stream part")).unwrap())
            .collect()
    }

    /// The first stream error of `parts`.
    fn first_error(parts: &[Result<TranscriptionStreamPart, AiMuxError>]) -> &AiMuxError {
        parts
            .iter()
            .find_map(|part| part.as_ref().err())
            .expect("the stream fails")
    }

    /// The value of a header, by name, whatever its case.
    fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Waits for the model, which reads the socket on its own task, to reach
    /// `ready`.
    async fn settle(mut ready: impl FnMut() -> bool) {
        for _ in 0..2000 {
            if ready() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        panic!("the model did not reach the expected state");
    }

    /// The task's result, bounded so a hung stream fails the test.
    async fn finished<T: Send + 'static>(task: JoinHandle<T>) -> T {
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the stream finishes")
            .expect("the stream task does not panic")
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doStream › should require channels when streaming multichannel audio
    #[tokio::test]
    async fn should_require_channels_when_streaming_multichannel_audio() {
        let (_handle, socket) = new_socket();
        let connector = MockConnector::with_socket(socket);
        let (audio, _) = Audio::new(vec![vec![1, 2, 3]], true);

        // Upstream's `argument: 'providerOptions'` is not in aimux's error.
        let error = transcription_model_with(Arc::clone(&connector), None)
            .do_stream(stream_options(audio, Some(json!({ "multichannel": true }))))
            .expect("the model has do_stream")
            .await
            .expect_err("multichannel audio needs channels");

        assert!(
            matches!(
                &error,
                AiMuxError::InvalidArgument(message)
                    if message == "providerOptions.xai.channels is required when providerOptions.xai.multichannel is true"
            ),
            "{error:?}"
        );
        assert!(connector.connects().is_empty(), "no socket is opened");
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doStream › should stream xAI STT over WebSocket
    #[tokio::test]
    async fn should_stream_xai_stt_over_websocket() {
        let (handle, socket) = new_socket();
        let connector = MockConnector::with_socket(socket);
        let (audio, _) = Audio::new(vec![vec![1, 2, 3]], true);

        let before = chrono::Utc::now().timestamp_millis();
        let result = transcription_model_with(Arc::clone(&connector), None)
            .do_stream(stream_options(
                audio,
                Some(json!({
                    "language": "en",
                    "diarize": true,
                    "keyterm": ["AI SDK", "Grok"],
                    "streaming": {
                        "interimResults": true,
                        "endpointing": 500,
                        "smartTurn": 0.7,
                        "smartTurnTimeout": 3000,
                    },
                })),
            ))
            .expect("the model has do_stream")
            .await
            .expect("do_stream succeeds");

        let connects = connector.connects();
        assert_eq!(connects.len(), 1);
        assert_eq!(
            connects[0].0,
            "wss://api.x.ai/v1/stt?sample_rate=16000&encoding=pcm&language=en&diarize=true&interim_results=true&endpointing=500&smart_turn=0.7&smart_turn_timeout=3000&keyterm=AI+SDK&keyterm=Grok"
        );
        assert_eq!(
            header(&connects[0].1, "authorization"),
            Some("Bearer test-api-key")
        );
        assert!(
            handle.frames().is_empty(),
            "nothing is sent before transcript.created"
        );

        let response = result.response.expect("the stream result has a response");
        let collecting = tokio::spawn(result.stream.collect::<Vec<_>>());
        handle.message(json!({ "type": "transcript.created" }));
        settle(|| handle.frames().len() == 2).await;

        let frames = handle.frames();
        assert_eq!(frames[0], Frame::Binary(vec![1, 2, 3]));
        let Frame::Text(done) = &frames[1] else {
            panic!("expected the audio.done text frame, got {:?}", frames[1]);
        };
        assert_eq!(
            serde_json::from_str::<Value>(done).unwrap(),
            json!({ "type": "audio.done" })
        );

        handle.message(json!({
            "type": "transcript.partial",
            "text": "Hel",
            "is_final": false,
            "speech_final": false,
            "start": 0,
            "duration": 0.5,
        }));
        handle.message(json!({
            "type": "transcript.partial",
            "text": "Hello",
            "is_final": true,
            "speech_final": true,
            "start": 0,
            "duration": 1,
        }));
        handle.message(json!({
            "type": "transcript.done",
            "text": "Hello",
            "duration": 1,
        }));

        let parts = finished(collecting).await;
        let after = chrono::Utc::now().timestamp_millis();
        assert_eq!(
            part_values(&parts),
            vec![
                json!({ "type": "stream-start", "warnings": [] }),
                json!({
                    "type": "transcript-partial",
                    "text": "Hel",
                    "startSecond": 0.0,
                    "durationInSeconds": 0.5,
                }),
                json!({
                    "type": "transcript-final",
                    "text": "Hello",
                    "startSecond": 0.0,
                    "endSecond": 1.0,
                }),
                json!({
                    "type": "finish",
                    "text": "Hello",
                    "segments": [],
                    "language": "en",
                    "durationInSeconds": 1.0,
                }),
            ]
        );

        // Upstream's `{ timestamp: testDate, modelId: '' }`, with the stamp
        // checked to fall within the call.
        let timestamp = response
            .timestamp
            .expect("the response has a timestamp")
            .unix_millis();
        assert!(
            before <= timestamp && timestamp <= after,
            "timestamp {timestamp} is outside the call ({before}..={after})"
        );
        assert_eq!(response.model_id.as_deref(), Some(""));
        assert!(response.headers.is_none() && response.body.is_none());
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doStream › should strip undefined header values before the WebSocket constructor
    #[tokio::test]
    async fn should_strip_undefined_header_values_before_the_web_socket_constructor() {
        let (_handle, socket) = new_socket();
        let connector = MockConnector::with_socket(socket);
        // `'X-Unset': undefined` is given at provider level: a per-call header
        // cannot hold an undefined value.
        let model = transcription_model_with(
            Arc::clone(&connector),
            Some(HashMap::from([("X-Unset".to_string(), None)])),
        );
        let (audio, _) = Audio::new(vec![vec![1, 2, 3]], true);
        let mut options = stream_options(audio, None);
        options.headers = Some(HashMap::from([(
            "Custom-Header".to_string(),
            "custom-value".to_string(),
        )]));

        let result = model
            .do_stream(options)
            .expect("the model has do_stream")
            .await
            .expect("do_stream succeeds");
        drop(result.stream);

        let connects = connector.connects();
        let headers = &connects[0].1;
        assert!(header(headers, "x-unset").is_none(), "{headers:?}");
        assert_eq!(
            header(headers, "authorization"),
            Some("Bearer test-api-key")
        );
        assert_eq!(header(headers, "custom-header"), Some("custom-value"));
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doStream › should cancel the audio stream when the WebSocket constructor throws
    #[tokio::test]
    async fn should_cancel_the_audio_stream_when_the_web_socket_constructor_throws() {
        let connector = MockConnector::failing("constructor failed");
        let (audio, cancelled) = Audio::new(Vec::new(), false);

        let result = transcription_model_with(connector, None)
            .do_stream(stream_options(audio, None))
            .expect("the model has do_stream")
            .await
            .expect("doStream resolves, and the connect failure is read from its stream");

        let parts = finished(tokio::spawn(result.stream.collect::<Vec<_>>())).await;
        let error = first_error(&parts);
        assert!(error.to_string().contains("constructor failed"), "{error}");
        assert!(
            cancelled.load(Ordering::SeqCst),
            "the audio stream is cancelled"
        );
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doStream › should cancel the audio stream when an audio send throws mid-stream
    #[tokio::test]
    async fn should_cancel_the_audio_stream_when_an_audio_send_throws_mid_stream() {
        let (handle, socket) = new_socket();
        handle.fail_binary_sends();
        let connector = MockConnector::with_socket(socket);
        let (audio, cancelled) = Audio::new(vec![vec![1, 2, 3]], false);

        let result = transcription_model_with(connector, None)
            .do_stream(stream_options(audio, None))
            .expect("the model has do_stream")
            .await
            .expect("do_stream succeeds");

        let collecting = tokio::spawn(result.stream.collect::<Vec<_>>());
        handle.message(json!({ "type": "transcript.created" }));
        let parts = finished(collecting).await;

        let error = first_error(&parts);
        assert!(error.to_string().contains("send failed"), "{error}");
        assert!(
            cancelled.load(Ordering::SeqCst),
            "the audio stream is cancelled"
        );
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doStream › should join finalized utterances per channel when transcript.done is empty
    #[tokio::test]
    async fn should_join_finalized_utterances_per_channel_when_transcript_done_is_empty() {
        let (handle, socket) = new_socket();
        let connector = MockConnector::with_socket(socket);
        let (audio, _) = Audio::new(vec![vec![1, 2, 3]], true);

        let result = transcription_model_with(connector, None)
            .do_stream(stream_options(audio, None))
            .expect("the model has do_stream")
            .await
            .expect("do_stream succeeds");

        let collecting = tokio::spawn(result.stream.collect::<Vec<_>>());
        handle.message(json!({ "type": "transcript.created" }));
        handle.message(json!({
            "type": "transcript.partial",
            "text": "First utterance.",
            "is_final": true,
            "speech_final": true,
            "start": 0,
            "duration": 1,
        }));
        handle.message(json!({
            "type": "transcript.partial",
            "text": "Second utterance.",
            "is_final": true,
            "speech_final": true,
            "start": 1,
            "duration": 1,
        }));
        handle.message(json!({ "type": "transcript.done", "text": "", "duration": 2 }));

        let parts = part_values(&finished(collecting).await);
        let finish = parts.last().expect("the stream has parts");
        assert_eq!(finish["type"], "finish");
        assert_eq!(finish["text"], "First utterance. Second utterance.");
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doStream › should error the stream with the server message on error events
    #[tokio::test]
    async fn should_error_the_stream_with_the_server_message_on_error_events() {
        let (handle, socket) = new_socket();
        let connector = MockConnector::with_socket(socket);
        let (audio, _) = Audio::new(vec![vec![1, 2, 3]], true);

        let result = transcription_model_with(connector, None)
            .do_stream(stream_options(audio, None))
            .expect("the model has do_stream")
            .await
            .expect("do_stream succeeds");

        let collecting = tokio::spawn(result.stream.collect::<Vec<_>>());
        handle.message(json!({ "type": "transcript.created" }));
        settle(|| handle.frames().len() == 2).await;
        handle.message(json!({ "type": "error", "message": "invalid sample_rate" }));
        let parts = finished(collecting).await;

        let error = first_error(&parts);
        assert!(error.to_string().contains("invalid sample_rate"), "{error}");
        assert!(
            handle.frames().contains(&Frame::Close),
            "the socket is closed after the error"
        );
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doStream › should close the WebSocket and stop reading audio when the stream is cancelled
    #[tokio::test]
    async fn should_close_the_web_socket_and_stop_reading_audio_when_the_stream_is_cancelled() {
        let (handle, socket) = new_socket();
        let connector = MockConnector::with_socket(socket);
        let (audio, cancelled) = Audio::new(Vec::new(), false);

        let result = transcription_model_with(connector, None)
            .do_stream(stream_options(audio, None))
            .expect("the model has do_stream")
            .await
            .expect("do_stream succeeds");

        let mut stream = result.stream;
        handle.message(json!({ "type": "transcript.created" }));
        let first = stream.next().await;
        assert!(
            matches!(first, Some(Ok(TranscriptionStreamPart::StreamStart { .. }))),
            "{first:?}"
        );

        // Upstream `result.stream.cancel()`: the stream is dropped; its
        // `await flush()`: the close frame goes out on its own task.
        drop(stream);
        settle(|| handle.frames().contains(&Frame::Close)).await;

        assert!(
            handle.frames().contains(&Frame::Close),
            "no Close frame was sent when the stream was cancelled"
        );
        assert!(
            cancelled.load(Ordering::SeqCst),
            "the audio stream is cancelled"
        );
    }
}
