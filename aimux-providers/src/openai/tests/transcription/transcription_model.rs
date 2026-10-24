use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};
use aimux_core::shared::SharedProviderOptions;
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};

const TRANSCRIPTIONS_URL: &str = "https://api.openai.com/v1/audio/transcriptions";
const BASE_URL: &str = "https://api.openai.com/v1";
const PINNED_USER_AGENT: &str = "ai-sdk-openai/4.0.83";

fn audio_data() -> Vec<u8> {
    include_bytes!("fixtures/transcription-test.mp3").to_vec()
}

fn fixture(name: &str) -> Value {
    let text = match name {
        "openai-transcription" => include_str!("fixtures/openai-transcription.json"),
        "openai-diarized-transcription" => {
            include_str!("fixtures/openai-diarized-transcription.json")
        }
        other => panic!("unknown fixture {other}"),
    };
    serde_json::from_str(text).unwrap()
}

/// The test server's `json-value` response: the compact JSON body with its
/// `content-type` and `content-length`, plus the given headers.
fn json_response(body: &Value, headers: &[(&str, &str)]) -> Canned {
    let mut canned = Canned::json(body);
    canned
        .headers
        .push(("content-length".into(), canned.body.len().to_string()));
    for (name, value) in headers {
        canned.headers.push(((*name).into(), (*value).into()));
    }
    canned
}

fn provider_with(mock: &Arc<MockFetch>, settings: OpenAIProviderSettings) -> OpenAIProvider {
    create_openai(OpenAIProviderSettings {
        api_key: Some("test-api-key".into()),
        base_url: Some(BASE_URL.into()),
        fetch: Some(mock.transport()),
        ..settings
    })
    .unwrap()
}

fn provider(mock: &Arc<MockFetch>) -> OpenAIProvider {
    provider_with(mock, OpenAIProviderSettings::default())
}

fn options(media_type: &str) -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(audio_data()), media_type)
}

fn options_with(media_type: &str, provider_options: Value) -> TranscriptionCallOptions {
    TranscriptionCallOptions {
        provider_options: Some(
            serde_json::from_value::<SharedProviderOptions>(provider_options).unwrap(),
        ),
        ..options(media_type)
    }
}

/// The test server's `requestBodyMultipart`: text fields as strings (a
/// repeated field becomes an array), file fields as
/// `{ "filename", "contentType", "size" }`.
fn multipart_body(seen: &Seen) -> Map<String, Value> {
    let content_type = seen.headers.get("content-type").expect("content-type");
    let boundary = content_type
        .split("boundary=")
        .nth(1)
        .expect("multipart boundary");
    let delimiter = format!("--{boundary}");
    let body = &seen.body;
    let mut fields = Map::new();
    let mut positions = Vec::new();
    let mut start = 0;
    while let Some(offset) = find(&body[start..], delimiter.as_bytes()) {
        positions.push(start + offset);
        start += offset + delimiter.len();
    }
    for window in positions.windows(2) {
        let part = &body[window[0] + delimiter.len()..window[1]];
        let part = part.strip_prefix(b"\r\n").unwrap_or(part);
        let part = part.strip_suffix(b"\r\n").unwrap_or(part);
        let header_end = find(part, b"\r\n\r\n").expect("part headers");
        let headers = String::from_utf8_lossy(&part[..header_end]).to_string();
        let content = &part[header_end + 4..];
        let name = between(&headers, "name=\"", "\"").expect("part name");
        let value = match between(&headers, "filename=\"", "\"") {
            Some(filename) => json!({
                "filename": filename,
                "contentType": headers
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Type: "))
                    .unwrap_or_default(),
                "size": content.len(),
            }),
            None => Value::String(String::from_utf8(content.to_vec()).unwrap()),
        };
        match fields.get_mut(&name) {
            Some(Value::Array(values)) => values.push(value),
            Some(existing) => *existing = Value::Array(vec![existing.take(), value]),
            None => {
                fields.insert(name, value);
            }
        }
    }
    fields
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn between(text: &str, start: &str, end: &str) -> Option<String> {
    let from = text.find(start)? + start.len();
    let to = text[from..].find(end)? + from;
    Some(text[from..to].to_string())
}

/// `expect(body.file).toBeInstanceOf(File)` and the rest of the fields.
fn multipart_without_file(seen: &Seen) -> Value {
    let mut fields = multipart_body(seen);
    let file = fields.remove("file").expect("file part");
    assert!(file.get("filename").is_some(), "file is a File: {file}");
    Value::Object(fields)
}

/// `toMatchObject`: every key of `expected` matches in `actual` (objects
/// recursively, arrays element by element with the same length).
fn assert_matches_object(actual: &Value, expected: &Value) {
    fn matches(actual: &Value, expected: &Value) -> bool {
        match (actual, expected) {
            (Value::Object(actual), Value::Object(expected)) => expected
                .iter()
                .all(|(key, value)| actual.get(key).is_some_and(|a| matches(a, value))),
            (Value::Array(actual), Value::Array(expected)) => {
                actual.len() == expected.len()
                    && actual.iter().zip(expected).all(|(a, e)| matches(a, e))
            }
            _ => actual == expected,
        }
    }
    assert!(
        matches(actual, expected),
        "expected {actual:#} to match {expected:#}"
    );
}

// ── realtime WebSocket mock (the upstream `MockWebSocket`) ──────────────────

#[cfg(feature = "realtime")]
mod ws_mock {
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    use futures::channel::mpsc;
    use futures::{Sink, Stream, StreamExt};
    use serde_json::Value;
    use tokio_tungstenite::tungstenite::{Error as WsError, Message};

    use aimux_core::error::AiMuxError;
    use aimux_core::transcription_model::{
        AudioChunk, InputAudioFormat, TranscriptionStreamOptions, TranscriptionStreamPart,
    };
    use aimux_provider_utils::ws::{WebSocketRequest, WsConnection, WsConnector};

    /// What one socket saw.
    #[derive(Default)]
    pub(super) struct SocketState {
        pub(super) sent: Vec<String>,
        pub(super) close_frame: bool,
        pub(super) dropped: bool,
    }

    /// One `new WebSocket(url, protocols, { headers })`.
    pub(super) struct Socket {
        pub(super) url: String,
        pub(super) protocols: Vec<String>,
        pub(super) headers: Vec<(String, String)>,
        pub(super) state: Arc<Mutex<SocketState>>,
        incoming: mpsc::UnboundedSender<Message>,
    }

    impl Socket {
        /// `ws.message(value)`.
        pub(super) fn message(&self, value: &Value) {
            self.incoming
                .unbounded_send(Message::Text(value.to_string()))
                .unwrap();
        }

        pub(super) fn sent(&self) -> Vec<Value> {
            self.state
                .lock()
                .unwrap()
                .sent
                .iter()
                .map(|text| serde_json::from_str(text).unwrap())
                .collect()
        }

        /// `ws.close` was called: a close frame or close of the transport,
        /// or the connection was dropped (which closes the socket in Rust).
        pub(super) fn closed(&self) -> bool {
            let state = self.state.lock().unwrap();
            state.close_frame || state.dropped
        }

        /// The handshake headers, names lower-cased (HTTP header names are
        /// case-insensitive).
        pub(super) fn header_names(&self) -> Vec<String> {
            self.headers
                .iter()
                .map(|(name, _)| name.to_ascii_lowercase())
                .collect()
        }

        pub(super) fn header(&self, name: &str) -> Option<String> {
            self.headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone())
        }
    }

    /// The `webSocket` constructor: records every socket.
    #[derive(Default)]
    pub(super) struct MockWebSocket {
        pub(super) instances: Mutex<Vec<Arc<Socket>>>,
        /// `send.mockImplementation`: a send whose text contains this fails.
        pub(super) fail_send_containing: Option<&'static str>,
        /// The constructor throws this message.
        pub(super) constructor_error: Option<&'static str>,
    }

    impl MockWebSocket {
        pub(super) fn instance(&self, index: usize) -> Arc<Socket> {
            self.instances.lock().unwrap()[index].clone()
        }
    }

    #[async_trait::async_trait]
    impl WsConnector for MockWebSocket {
        async fn connect(&self, request: &WebSocketRequest) -> Result<WsConnection, AiMuxError> {
            if let Some(message) = self.constructor_error {
                return Err(AiMuxError::other(message));
            }
            let (incoming, rx) = mpsc::unbounded();
            let state = Arc::new(Mutex::new(SocketState::default()));
            self.instances.lock().unwrap().push(Arc::new(Socket {
                url: request.url.clone(),
                protocols: request.subprotocols.clone(),
                headers: request.headers.clone(),
                state: state.clone(),
                incoming,
            }));
            let transport = Transport {
                rx,
                state,
                fail_send_containing: self.fail_send_containing,
            };
            Ok(WsConnection::from_transport(Box::new(transport), request))
        }
    }

    struct Transport {
        rx: mpsc::UnboundedReceiver<Message>,
        state: Arc<Mutex<SocketState>>,
        fail_send_containing: Option<&'static str>,
    }

    impl Drop for Transport {
        fn drop(&mut self) {
            self.state.lock().unwrap().dropped = true;
        }
    }

    impl Stream for Transport {
        type Item = Result<Message, WsError>;

        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            self.rx.poll_next_unpin(cx).map(|message| message.map(Ok))
        }
    }

    impl Sink<Message> for Transport {
        type Error = WsError;

        fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), WsError>> {
            Poll::Ready(Ok(()))
        }

        fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), WsError> {
            match item {
                Message::Text(text) => {
                    if self
                        .fail_send_containing
                        .is_some_and(|needle| text.contains(needle))
                    {
                        return Err(WsError::Io(std::io::Error::other("send failed")));
                    }
                    self.state.lock().unwrap().sent.push(text);
                }
                Message::Close(_) => self.state.lock().unwrap().close_frame = true,
                _ => {}
            }
            Ok(())
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), WsError>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), WsError>> {
            self.state.lock().unwrap().close_frame = true;
            Poll::Ready(Ok(()))
        }
    }

    /// Sets the flag when the audio stream is dropped (`cancel()`).
    struct CancelFlag(Arc<AtomicBool>);

    impl Drop for CancelFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// An audio stream of `chunks`, then the end (or pending forever when
    /// `stay_open`); `cancelled` turns true when it is dropped.
    pub(super) fn audio(
        chunks: Vec<Vec<u8>>,
        stay_open: bool,
        cancelled: &Arc<AtomicBool>,
    ) -> Pin<Box<dyn Stream<Item = AudioChunk> + Send>> {
        let flag = CancelFlag(cancelled.clone());
        let chunks = futures::stream::iter(chunks.into_iter().map(AudioChunk::Binary));
        let stream: Pin<Box<dyn Stream<Item = AudioChunk> + Send>> = if stay_open {
            Box::pin(chunks.chain(futures::stream::pending()))
        } else {
            Box::pin(chunks)
        };
        Box::pin(stream.map(move |chunk| {
            let _ = &flag;
            chunk
        }))
    }

    /// `convertArrayToReadableStream([new Uint8Array([1, 2, 3])])`.
    pub(super) fn audio_123() -> Pin<Box<dyn Stream<Item = AudioChunk> + Send>> {
        audio(
            vec![vec![1, 2, 3]],
            false,
            &Arc::new(AtomicBool::new(false)),
        )
    }

    pub(super) fn stream_options(
        audio: Pin<Box<dyn Stream<Item = AudioChunk> + Send>>,
    ) -> TranscriptionStreamOptions {
        TranscriptionStreamOptions {
            audio,
            input_audio_format: InputAudioFormat {
                format_type: "audio/pcm".into(),
                rate: Some(24_000),
            },
            provider_options: None,
            abort_signal: None,
            headers: None,
            include_raw_chunks: false,
            timeout: None,
        }
    }

    /// `convertReadableStreamToArray`: the parts as JSON, or the first error.
    pub(super) async fn collect(
        mut stream: Pin<Box<dyn Stream<Item = Result<TranscriptionStreamPart, AiMuxError>> + Send>>,
    ) -> Result<Vec<Value>, AiMuxError> {
        let mut parts = Vec::new();
        while let Some(part) = stream.next().await {
            parts.push(serde_json::to_value(part?).unwrap());
        }
        Ok(parts)
    }

    /// Wait (bounded) until the condition holds; the `await flush()` of
    /// upstream, for a stream polled on another task.
    pub(super) async fn wait_until(condition: impl Fn() -> bool) {
        for _ in 0..400 {
            if condition() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }
}

mod do_generate {
    use super::*;

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-transcription"), &[])]);
        let provider = provider_with(
            &mock,
            OpenAIProviderSettings {
                organization: Some("test-organization".into()),
                project: Some("test-project".into()),
                headers: Some(HashMap::from([(
                    "Custom-Provider-Header".to_string(),
                    Some("provider-header-value".to_string()),
                )])),
                ..Default::default()
            },
        );

        provider
            .transcription("whisper-1")
            .do_generate(&TranscriptionCallOptions {
                headers: Some(HashMap::from([(
                    "Custom-Request-Header".to_string(),
                    "request-header-value".to_string(),
                )])),
                ..options("audio/wav")
            })
            .await
            .unwrap();

        let seen = &mock.seen()[0];
        assert_eq!(seen.url, TRANSCRIPTIONS_URL);
        assert_matches_object(
            &serde_json::to_value(&seen.headers).unwrap(),
            &json!({
                "authorization": "Bearer test-api-key",
                "custom-provider-header": "provider-header-value",
                "custom-request-header": "request-header-value",
                "openai-organization": "test-organization",
                "openai-project": "test-project",
            }),
        );
        // `expect.stringMatching(/^multipart\/form-data; boundary=----formdata-undici-\d+$/)`:
        // the boundary is the transport's own (aimux: `----formdata-aimux-<n>`).
        let content_type = &seen.headers["content-type"];
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=----formdata-")
            .unwrap_or_else(|| panic!("content-type {content_type}"));
        let digits = boundary.rsplit('-').next().unwrap();
        assert!(
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()),
            "content-type {content_type}"
        );
        let user_agent = seen.headers.get("user-agent").cloned().unwrap_or_default();
        assert!(user_agent.contains(PINNED_USER_AGENT), "{user_agent}");
    }

    // should extract usage: not ported (sub-feature: `TranscriptionResult`
    // has no `usage`).

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should pass response_format when `providerOptions.openai.timestampGranularities` is set
    #[tokio::test]
    async fn should_pass_response_format_when_timestamp_granularities_is_set() {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-transcription"), &[])]);
        let model = provider(&mock).transcription("whisper-1");

        model
            .do_generate(&options_with(
                "audio/wav",
                json!({ "openai": { "timestampGranularities": ["word"] } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            multipart_without_file(&mock.seen()[0]),
            json!({
                "model": "whisper-1",
                "response_format": "verbose_json",
                "temperature": "0",
                "timestamp_granularities[]": "word",
            })
        );
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should not set pass response_format to "verbose_json" when model is "gpt-4o-transcribe"
    #[tokio::test]
    async fn should_not_set_pass_response_format_to_verbose_json_for_gpt_4o_transcribe() {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-transcription"), &[])]);
        let model = provider(&mock).transcription("gpt-4o-transcribe");

        model
            .do_generate(&options_with(
                "audio/wav",
                json!({ "openai": { "timestampGranularities": ["word"] } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            multipart_without_file(&mock.seen()[0]),
            json!({
                "model": "gpt-4o-transcribe",
                "response_format": "json",
                "temperature": "0",
                "timestamp_granularities[]": "word",
            })
        );
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should support gpt-4o-transcribe-diarize
    #[tokio::test]
    async fn should_support_gpt_4o_transcribe_diarize() {
        let mock = MockFetch::new(vec![json_response(
            &fixture("openai-diarized-transcription"),
            &[],
        )]);
        let model = provider(&mock).transcription("gpt-4o-transcribe-diarize");

        let result = model.do_generate(&options("audio/mpeg")).await.unwrap();

        assert_eq!(
            multipart_without_file(&mock.seen()[0]),
            json!({
                "chunking_strategy": "auto",
                "model": "gpt-4o-transcribe-diarize",
                "response_format": "diarized_json",
            })
        );
        assert_matches_object(
            &serde_json::to_value(&result).unwrap(),
            &json!({
                "text": "Hello from Alice. Hello from Bob.",
                "durationInSeconds": 3.2,
                "segments": [
                    { "text": "Hello from Alice.", "startSecond": 0.0, "endSecond": 1.5 },
                    { "text": "Hello from Bob.", "startSecond": 1.5, "endSecond": 3.2 },
                ],
                "providerMetadata": {
                    "openai": {
                        "segments": [
                            {
                                "text": "Hello from Alice.",
                                "startSecond": 0.0,
                                "endSecond": 1.5,
                                "speaker": "A",
                            },
                            {
                                "text": "Hello from Bob.",
                                "startSecond": 1.5,
                                "endSecond": 3.2,
                                "speaker": "B",
                            },
                        ],
                    },
                },
            }),
        );
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should pass diarization response format and chunking strategy provider options
    #[tokio::test]
    async fn should_pass_diarization_response_format_and_chunking_strategy_provider_options() {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-transcription"), &[])]);
        let model = provider(&mock).transcription("gpt-4o-transcribe-diarize");

        model
            .do_generate(&options_with(
                "audio/mpeg",
                json!({ "openai": { "responseFormat": "json", "chunkingStrategy": "auto" } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            multipart_without_file(&mock.seen()[0]),
            json!({
                "chunking_strategy": "auto",
                "model": "gpt-4o-transcribe-diarize",
                "response_format": "json",
                "temperature": "0",
                "timestamp_granularities[]": "segment",
            })
        );
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should serialize a server VAD chunking strategy
    #[tokio::test]
    async fn should_serialize_a_server_vad_chunking_strategy() {
        let mock = MockFetch::new(vec![json_response(
            &fixture("openai-diarized-transcription"),
            &[],
        )]);
        let model = provider(&mock).transcription("gpt-4o-transcribe-diarize");

        model
            .do_generate(&options_with(
                "audio/mpeg",
                json!({
                    "openai": {
                        "chunkingStrategy": {
                            "type": "server_vad",
                            "threshold": 0.7,
                            "prefixPaddingMs": 400,
                            "silenceDurationMs": 300,
                        },
                    },
                }),
            ))
            .await
            .unwrap();

        assert_eq!(
            multipart_body(&mock.seen()[0]).get("chunking_strategy"),
            Some(&json!(
                r#"{"type":"server_vad","threshold":0.7,"prefix_padding_ms":400,"silence_duration_ms":300}"#
            ))
        );
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should pass timestamp_granularities when specified
    #[tokio::test]
    async fn should_pass_timestamp_granularities_when_specified() {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-transcription"), &[])]);
        let model = provider(&mock).transcription("whisper-1");

        model
            .do_generate(&options_with(
                "audio/wav",
                json!({ "openai": { "timestampGranularities": ["segment"] } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            multipart_without_file(&mock.seen()[0]),
            json!({
                "model": "whisper-1",
                "response_format": "verbose_json",
                "temperature": "0",
                "timestamp_granularities[]": "segment",
            })
        );
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should work when no words, language, or duration are returned
    #[tokio::test]
    async fn should_work_when_no_words_language_or_duration_are_returned() {
        let mock = MockFetch::new(vec![json_response(
            &json!({
                "task": "transcribe",
                "text": "Hello from the Vercel AI SDK!",
                "_request_id": "req_1234",
            }),
            &[],
        )]);
        let model = provider(&mock).transcription("whisper-1");

        let result = model.do_generate(&options("audio/wav")).await.unwrap();

        // The model's clock cannot be injected (upstream fixes it at
        // `new Date(0)`): the timestamp is asserted present, then removed.
        let mut actual = serde_json::to_value(&result).unwrap();
        assert!(
            actual["response"]
                .as_object_mut()
                .unwrap()
                .remove("timestamp")
                .is_some()
        );
        // `request: {}` is aimux's request record without a body (upstream
        // returns no request field).
        if actual.get("request") == Some(&json!({})) {
            actual.as_object_mut().unwrap().remove("request");
        }
        assert_eq!(
            actual,
            json!({
                "response": {
                    "body": {
                        "_request_id": "req_1234",
                        "task": "transcribe",
                        "text": "Hello from the Vercel AI SDK!",
                    },
                    "headers": {
                        "content-length": "85",
                        "content-type": "application/json",
                    },
                    "modelId": "whisper-1",
                },
                "segments": [],
                "text": "Hello from the Vercel AI SDK!",
                "warnings": [],
            })
        );
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should parse segments when provided in response
    #[tokio::test]
    async fn should_parse_segments_when_provided_in_response() {
        let mock = MockFetch::new(vec![json_response(
            &json!({
                "task": "transcribe",
                "text": "Hello world. How are you?",
                "segments": [
                    {
                        "id": 0,
                        "seek": 0,
                        "start": 0.0,
                        "end": 2.5,
                        "text": "Hello world.",
                        "tokens": [1234, 5678],
                        "temperature": 0.0,
                        "avg_logprob": -0.5,
                        "compression_ratio": 1.2,
                        "no_speech_prob": 0.1,
                    },
                    {
                        "id": 1,
                        "seek": 250,
                        "start": 2.5,
                        "end": 5.0,
                        "text": " How are you?",
                        "tokens": [9012, 3456],
                        "temperature": 0.0,
                        "avg_logprob": -0.6,
                        "compression_ratio": 1.1,
                        "no_speech_prob": 0.05,
                    },
                ],
                "language": "en",
                "duration": 5.0,
                "_request_id": "req_1234",
            }),
            &[],
        )]);
        let model = provider(&mock).transcription("whisper-1");

        let result = model
            .do_generate(&options_with(
                "audio/wav",
                json!({ "openai": { "timestampGranularities": ["segment"] } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.segments).unwrap(),
            json!([
                { "endSecond": 2.5, "startSecond": 0.0, "text": "Hello world." },
                { "endSecond": 5.0, "startSecond": 2.5, "text": " How are you?" },
            ])
        );
        assert_eq!(result.text, "Hello world. How are you?");
        assert_eq!(result.duration_in_seconds, Some(5.0));
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should fallback to words when segments are not available
    #[tokio::test]
    async fn should_fallback_to_words_when_segments_are_not_available() {
        let mock = MockFetch::new(vec![json_response(
            &json!({
                "task": "transcribe",
                "text": "Hello world",
                "words": [
                    { "word": "Hello", "start": 0.0, "end": 1.0 },
                    { "word": "world", "start": 1.0, "end": 2.0 },
                ],
                "language": "en",
                "duration": 2.0,
                "_request_id": "req_1234",
            }),
            &[],
        )]);
        let model = provider(&mock).transcription("whisper-1");

        let result = model
            .do_generate(&options_with(
                "audio/wav",
                json!({ "openai": { "timestampGranularities": ["word"] } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.segments).unwrap(),
            json!([
                { "endSecond": 1.0, "startSecond": 0.0, "text": "Hello" },
                { "endSecond": 2.0, "startSecond": 1.0, "text": "world" },
            ])
        );
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should handle empty segments array
    #[tokio::test]
    async fn should_handle_empty_segments_array() {
        let mock = MockFetch::new(vec![json_response(
            &json!({
                "task": "transcribe",
                "text": "Hello world",
                "segments": [],
                "language": "en",
                "duration": 2.0,
                "_request_id": "req_1234",
            }),
            &[],
        )]);
        let model = provider(&mock).transcription("whisper-1");

        let result = model.do_generate(&options("audio/wav")).await.unwrap();

        assert!(result.segments.is_empty());
        assert_eq!(result.text, "Hello world");
    }
}

#[cfg(feature = "realtime")]
mod do_stream {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::ws_mock::{MockWebSocket, audio, audio_123, collect, stream_options, wait_until};
    use super::*;

    fn realtime_provider(web_socket: &Arc<MockWebSocket>, api_key: &str) -> OpenAIProvider {
        realtime_provider_with(web_socket, api_key, None)
    }

    fn realtime_provider_with(
        web_socket: &Arc<MockWebSocket>,
        api_key: &str,
        headers: Option<aimux_provider_utils::HeaderMapOpt>,
    ) -> OpenAIProvider {
        create_openai(OpenAIProviderSettings {
            api_key: Some(api_key.into()),
            base_url: Some(BASE_URL.into()),
            headers,
            web_socket: Some(web_socket.clone()),
            ..Default::default()
        })
        .unwrap()
    }

    fn completed() -> Value {
        json!({
            "type": "conversation.item.input_audio_transcription.completed",
            "item_id": "item-1",
            "transcript": "Hello",
        })
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doStream › should stream gpt-realtime-whisper using OpenAI realtime transcription
    #[tokio::test]
    async fn should_stream_gpt_realtime_whisper_using_openai_realtime_transcription() {
        let web_socket = Arc::new(MockWebSocket::default());
        let model =
            realtime_provider(&web_socket, "test-api-key").transcription("gpt-realtime-whisper");

        let result = model
            .do_stream(
                aimux_core::transcription_model::TranscriptionStreamOptions {
                    provider_options: Some(
                        serde_json::from_value(json!({
                            "openai": {
                                "language": "en",
                                "streaming": {
                                    "delay": "low",
                                    "include": ["item.input_audio_transcription.logprobs"],
                                },
                            },
                        }))
                        .unwrap(),
                    ),
                    ..stream_options(audio_123())
                },
            )
            .unwrap()
            .await
            .unwrap();

        let parts = tokio::spawn(collect(result.stream));
        let ws = web_socket.instance(0);
        assert_eq!(
            ws.url,
            "wss://api.openai.com/v1/realtime?intent=transcription"
        );
        assert_eq!(
            ws.protocols,
            ["realtime", "openai-insecure-api-key.test-api-key"]
        );
        // OpenAI rejects handshakes that send both auth channels:
        assert!(!ws.header_names().contains(&"authorization".to_string()));

        wait_until(|| ws.sent().len() >= 3).await;
        let sent = ws.sent();
        assert_eq!(
            sent[0],
            json!({
                "type": "session.update",
                "session": {
                    "type": "transcription",
                    "audio": {
                        "input": {
                            "format": { "type": "audio/pcm", "rate": 24000 },
                            "transcription": {
                                "model": "gpt-realtime-whisper",
                                "language": "en",
                                "delay": "low",
                            },
                            "turn_detection": null,
                        },
                    },
                    "include": ["item.input_audio_transcription.logprobs"],
                },
            })
        );
        assert_eq!(
            sent[1],
            json!({ "type": "input_audio_buffer.append", "audio": "AQID" })
        );
        assert_eq!(sent[2], json!({ "type": "input_audio_buffer.commit" }));

        ws.message(&json!({
            "type": "conversation.item.input_audio_transcription.delta",
            "item_id": "item-1",
            "delta": "Hel",
        }));
        ws.message(&completed());

        assert_eq!(
            parts.await.unwrap().unwrap(),
            vec![
                json!({ "type": "stream-start", "warnings": [] }),
                json!({ "type": "transcript-delta", "id": "item-1", "delta": "Hel" }),
                json!({ "type": "transcript-final", "id": "item-1", "text": "Hello" }),
                json!({ "type": "finish", "text": "Hello", "segments": [], "language": "en" }),
            ]
        );
        // The clock cannot be injected (upstream: `timestamp: new Date(0)`).
        let response = result.response.unwrap();
        assert!(response.timestamp.is_some());
        assert_eq!(response.model_id.as_deref(), Some("gpt-realtime-whisper"));
        assert_eq!(response.headers, None);
        assert_eq!(response.body, None);
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doStream › should accept dated gpt-realtime-whisper snapshot model IDs
    #[tokio::test]
    async fn should_accept_dated_gpt_realtime_whisper_snapshot_model_ids() {
        let web_socket = Arc::new(MockWebSocket::default());
        let model = realtime_provider(&web_socket, "test-api-key")
            .transcription("gpt-realtime-whisper-2026-01-01");

        let result = model
            .do_stream(stream_options(audio_123()))
            .unwrap()
            .await
            .unwrap();

        let parts = tokio::spawn(collect(result.stream));
        let ws = web_socket.instance(0);
        wait_until(|| !ws.sent().is_empty()).await;
        assert_eq!(
            ws.sent()[0]["session"]["audio"]["input"]["transcription"]["model"],
            "gpt-realtime-whisper-2026-01-01"
        );

        ws.message(&completed());
        assert!(parts.await.unwrap().is_ok());
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doStream › should strip only the Authorization header and pass other headers to the WebSocket constructor
    #[tokio::test]
    async fn should_strip_only_the_authorization_header_and_pass_other_headers() {
        let web_socket = Arc::new(MockWebSocket::default());
        let provider = create_openai(OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(BASE_URL.into()),
            organization: Some("test-organization".into()),
            headers: Some(HashMap::from([(
                "Custom-Header".to_string(),
                Some("custom-value".to_string()),
            )])),
            web_socket: Some(web_socket.clone()),
            ..Default::default()
        })
        .unwrap();
        let model = provider.transcription("gpt-realtime-whisper");

        let result = model
            .do_stream(stream_options(audio_123()))
            .unwrap()
            .await
            .unwrap();

        let parts = tokio::spawn(collect(result.stream));
        let ws = web_socket.instance(0);
        assert_eq!(
            ws.protocols,
            ["realtime", "openai-insecure-api-key.test-api-key"]
        );
        assert_eq!(
            ws.header("OpenAI-Organization").as_deref(),
            Some("test-organization")
        );
        assert_eq!(ws.header("Custom-Header").as_deref(), Some("custom-value"));
        assert!(!ws.header_names().contains(&"authorization".to_string()));

        wait_until(|| ws.sent().len() >= 3).await;
        ws.message(&completed());
        assert!(parts.await.unwrap().is_ok());
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doStream › should use only the realtime protocol and pass headers unchanged when there is no Authorization header
    #[tokio::test]
    async fn should_use_only_the_realtime_protocol_when_there_is_no_authorization_header() {
        let web_socket = Arc::new(MockWebSocket::default());
        // No Authorization: the provider's credential header removed.
        let provider = realtime_provider_with(
            &web_socket,
            "test-api-key",
            Some(HashMap::from([
                ("Authorization".to_string(), None),
                (
                    "Custom-Header".to_string(),
                    Some("custom-value".to_string()),
                ),
            ])),
        );
        let model = provider.transcription("gpt-realtime-whisper");

        let result = model
            .do_stream(stream_options(audio_123()))
            .unwrap()
            .await
            .unwrap();

        let parts = tokio::spawn(collect(result.stream));
        let ws = web_socket.instance(0);
        assert_eq!(ws.protocols, ["realtime"]);
        assert_eq!(ws.header("Custom-Header").as_deref(), Some("custom-value"));

        wait_until(|| ws.sent().len() >= 3).await;
        ws.message(&completed());
        assert!(parts.await.unwrap().is_ok());
    }

    // should strip undefined header values before the WebSocket constructor:
    // not ported (ts-only: per-call headers are `HashMap<String, String>`, an
    // `undefined` header value cannot be expressed).

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doStream › should cancel the audio stream when the WebSocket constructor throws
    #[tokio::test]
    async fn should_cancel_the_audio_stream_when_the_websocket_constructor_throws() {
        let web_socket = Arc::new(MockWebSocket {
            constructor_error: Some("constructor failed"),
            ..Default::default()
        });
        let audio_cancelled = Arc::new(AtomicBool::new(false));
        let model =
            realtime_provider(&web_socket, "test-api-key").transcription("gpt-realtime-whisper");

        let result = model
            .do_stream(stream_options(audio(vec![], true, &audio_cancelled)))
            .unwrap()
            .await;

        // aimux connects before `do_stream` returns (RFC-0028: a connect
        // failure is the call's error, not an in-stream error part), where
        // upstream connects inside the stream; either way the caller gets the
        // constructor's error and the audio stream is cancelled (dropped).
        let error = match result {
            Ok(result) => collect(result.stream).await.unwrap_err(),
            Err(error) => error,
        };
        assert!(error.to_string().contains("constructor failed"), "{error}");
        assert!(audio_cancelled.load(Ordering::SeqCst));
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doStream › should cancel the audio stream when an audio send throws mid-stream
    #[tokio::test]
    async fn should_cancel_the_audio_stream_when_an_audio_send_throws_mid_stream() {
        let web_socket = Arc::new(MockWebSocket {
            fail_send_containing: Some("input_audio_buffer.append"),
            ..Default::default()
        });
        let audio_cancelled = Arc::new(AtomicBool::new(false));
        let model =
            realtime_provider(&web_socket, "test-api-key").transcription("gpt-realtime-whisper");

        let result = model
            .do_stream(stream_options(audio(
                vec![vec![1, 2, 3]],
                true,
                &audio_cancelled,
            )))
            .unwrap()
            .await
            .unwrap();

        let error = collect(result.stream).await.unwrap_err();
        assert!(error.to_string().contains("send failed"), "{error}");
        assert!(audio_cancelled.load(Ordering::SeqCst));
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doStream › should carry mixed-case Authorization headers into the subprotocol and strip them
    #[tokio::test]
    async fn should_carry_mixed_case_authorization_headers_into_the_subprotocol() {
        let web_socket = Arc::new(MockWebSocket::default());
        let provider = realtime_provider_with(
            &web_socket,
            "test-api-key",
            Some(HashMap::from([(
                "AUTHORIZATION".to_string(),
                Some("Bearer test-api-key".to_string()),
            )])),
        );
        let model = provider.transcription("gpt-realtime-whisper");

        let result = model
            .do_stream(stream_options(audio_123()))
            .unwrap()
            .await
            .unwrap();

        drop(result.stream);
        let ws = web_socket.instance(0);
        assert_eq!(
            ws.protocols,
            ["realtime", "openai-insecure-api-key.test-api-key"]
        );
        assert!(!ws.header_names().contains(&"authorization".to_string()));
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doStream › should let per-call authorization headers override configuration headers across casings
    #[tokio::test]
    async fn should_let_per_call_authorization_headers_override_configuration_headers() {
        let web_socket = Arc::new(MockWebSocket::default());
        let model =
            realtime_provider(&web_socket, "config-key").transcription("gpt-realtime-whisper");

        let result = model
            .do_stream(
                aimux_core::transcription_model::TranscriptionStreamOptions {
                    // lowercase scheme + lowercase header key, both case-insensitive
                    headers: Some(HashMap::from([(
                        "authorization".to_string(),
                        "bearer per-call-key".to_string(),
                    )])),
                    ..stream_options(audio_123())
                },
            )
            .unwrap()
            .await
            .unwrap();

        drop(result.stream);
        let ws = web_socket.instance(0);
        assert_eq!(
            ws.protocols,
            ["realtime", "openai-insecure-api-key.per-call-key"]
        );
        assert!(!ws.header_names().contains(&"authorization".to_string()));
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doStream › should warn about unsupported non-streaming provider options
    #[tokio::test]
    async fn should_warn_about_unsupported_non_streaming_provider_options() {
        let web_socket = Arc::new(MockWebSocket::default());
        let model =
            realtime_provider(&web_socket, "test-api-key").transcription("gpt-realtime-whisper");

        let result = model
            .do_stream(
                aimux_core::transcription_model::TranscriptionStreamOptions {
                    provider_options: Some(
                        serde_json::from_value(json!({
                            "openai": { "prompt": "context prompt", "temperature": 0.5 },
                        }))
                        .unwrap(),
                    ),
                    ..stream_options(audio_123())
                },
            )
            .unwrap()
            .await
            .unwrap();

        let parts = tokio::spawn(collect(result.stream));
        let ws = web_socket.instance(0);
        wait_until(|| ws.sent().len() >= 3).await;
        ws.message(&completed());

        let parts = parts.await.unwrap().unwrap();
        assert_eq!(
            parts[0],
            json!({
                "type": "stream-start",
                "warnings": [
                    {
                        "type": "unsupported",
                        "feature": "providerOptions.openai.prompt",
                        "details": "OpenAI streaming transcription does not support prompt.",
                    },
                    {
                        "type": "unsupported",
                        "feature": "providerOptions.openai.temperature",
                        "details": "OpenAI streaming transcription does not support temperature.",
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doStream › should error the stream with the server message on error events
    #[tokio::test]
    async fn should_error_the_stream_with_the_server_message_on_error_events() {
        let web_socket = Arc::new(MockWebSocket::default());
        let model =
            realtime_provider(&web_socket, "test-api-key").transcription("gpt-realtime-whisper");

        let result = model
            .do_stream(stream_options(audio_123()))
            .unwrap()
            .await
            .unwrap();

        let parts = tokio::spawn(collect(result.stream));
        let ws = web_socket.instance(0);
        wait_until(|| ws.sent().len() >= 3).await;

        ws.message(&json!({
            "type": "error",
            "error": { "message": "invalid session configuration" },
        }));
        let error = parts.await.unwrap().unwrap_err();
        assert!(
            error.to_string().contains("invalid session configuration"),
            "{error}"
        );

        assert!(ws.closed());
    }

    /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doStream › should close the WebSocket and stop reading audio when the stream is cancelled
    #[tokio::test]
    async fn should_close_the_websocket_and_stop_reading_audio_when_cancelled() {
        let web_socket = Arc::new(MockWebSocket::default());
        let model =
            realtime_provider(&web_socket, "test-api-key").transcription("gpt-realtime-whisper");
        let audio_cancelled = Arc::new(AtomicBool::new(false));

        let result = model
            .do_stream(stream_options(audio(vec![], true, &audio_cancelled)))
            .unwrap()
            .await
            .unwrap();

        let ws = web_socket.instance(0);
        wait_until(|| !ws.sent().is_empty()).await;

        // `stream.cancel()`: the stream is dropped.
        drop(result.stream);
        tokio::task::yield_now().await;

        assert!(ws.closed());
        assert!(audio_cancelled.load(Ordering::SeqCst));
    }
}
