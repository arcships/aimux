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

// ── Older ports (previously `tests/openai_transcription_test.rs`) ──
//
// Rust translation of the OpenAI transcription (STT) model tests.
//
// Source: `reference/ai/packages/openai/src/transcription/openai-transcription-model.test.ts`
//
// Each test spins up a `wiremock` mock server, configures a JSON response,
// creates an `OpenAITranscriptionModel` pointing at the mock, calls
// `do_generate`, and asserts on the multipart request body / headers / result.
//
// The TS tests inject a custom `currentDate` via `_internal` for timestamp
// assertions. The Rust model always uses `Utc::now()`; the timestamp tests
// therefore verify that a timestamp is present and that `model_id` matches,
// rather than asserting an exact timestamp value.
//
// The TS `doStream` tests are translated in
// `openai_transcription_stream_test.rs` (RFC-0028: local WebSocket server)
// and `openai_realtime_ws_replay_test.rs` (offline, through the provider's
// `web_socket` connector).

mod older_transcription {
    use std::collections::HashMap;

    use serde_json::{Value, json};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::{OpenAIProviderSettings, create_openai};
    use aimux_core::error::AiMuxError;
    use aimux_core::provider::Provider;
    use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions};

    // ── helpers ─────────────────────────────────────────────────────────────────

    /// Mock audio data (same as TS `audioData` — just some bytes).
    fn mock_audio_data() -> Vec<u8> {
        vec![1u8, 2, 3, 4, 5]
    }

    /// Build `TranscriptionCallOptions` with binary audio and a media type.
    fn transcription_options(audio: Vec<u8>, media_type: &str) -> TranscriptionCallOptions {
        TranscriptionCallOptions::new(AudioInput::Binary(audio), media_type)
    }

    /// Mount a mock JSON transcription response on the server at
    /// `/audio/transcriptions`.
    async fn mock_json_response(server: &MockServer, body: &Value) {
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(body.clone()),
            )
            .mount(server)
            .await;
    }

    /// Mount a mock JSON transcription response with extra response headers.
    async fn mock_json_response_with_headers(
        server: &MockServer,
        body: &Value,
        headers: &[(&str, &str)],
    ) {
        let mut template = ResponseTemplate::new(200)
            .insert_header("content-type", "application/json")
            .set_body_json(body.clone());
        for (k, v) in headers {
            template = template.insert_header(*k, *v);
        }
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(template)
            .mount(server)
            .await;
    }

    /// Parse a multipart/form-data body into a map of field name → value (string).
    ///
    /// Non-file fields are decoded as UTF-8 strings. File fields are returned as
    /// the raw bytes converted to a string (for assertion purposes).
    fn parse_multipart_form(body: &[u8]) -> HashMap<String, String> {
        let body_str = String::from_utf8_lossy(body);
        let mut form = HashMap::new();

        // The first line is the boundary with -- prefix.
        // Find the boundary.
        let boundary = body_str
            .lines()
            .next()
            .unwrap_or("")
            .trim_start_matches('-');
        if boundary.is_empty() {
            return form;
        }

        // Split on the boundary.
        for part in body_str.split(boundary) {
            let part = part.trim_matches(|c| c == '\r' || c == '\n' || c == '-');
            if part.is_empty() || part == "--" {
                continue;
            }

            // Each part has headers and a body separated by \r\n\r\n.
            if let Some(header_end) = part.find("\r\n\r\n") {
                let headers = &part[..header_end];
                let value = &part[header_end + 4..];

                // Extract the field name from Content-Disposition.
                if let Some(name_start) = headers.find("name=\"") {
                    let name_start = name_start + 6;
                    if let Some(name_end) = headers[name_start..].find('"') {
                        let name = &headers[name_start..name_start + name_end];
                        form.insert(name.to_string(), value.trim_end_matches('\r').to_string());
                    }
                }
            }
        }

        form
    }

    /// The full OpenAI transcription fixture response (matches the TS fixture).
    fn fixture_response() -> Value {
        json!({
            "task": "transcribe",
            "language": "english",
            "duration": 36.709999084472656,
            "text": "Galileo was an American robotic space program that studied the planet Jupiter and its moons, as well as several other solar system bodies.",
            "words": [
                {"word": "Galileo", "start": 0.0, "end": 0.6600000262260437},
                {"word": "was", "start": 0.6600000262260437, "end": 0.8999999761581421},
                {"word": "an", "start": 0.8999999761581421, "end": 1.1399999856948853},
                {"word": "American", "start": 1.1399999856948853, "end": 1.5},
                {"word": "robotic", "start": 1.5, "end": 2.0199999809265137}
            ],
            "usage": {"type": "duration", "seconds": 37}
        })
    }

    // ════════════════════════════════════════════════════════════════════════════
    // doGenerate
    // (openai-transcription-model.test.ts → describe('doGenerate'))
    // ════════════════════════════════════════════════════════════════════════════

    mod do_generate {
        use super::*;

        /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should reject gpt-realtime-whisper for non-streaming transcription
        #[tokio::test]
        async fn should_reject_gpt_realtime_whisper_for_non_streaming() {
            let server = MockServer::start().await;
            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider
                .transcription_model("gpt-realtime-whisper")
                .unwrap()
                .unwrap();

            let result = model
                .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
                .await;

            assert!(result.is_err());
            match result.unwrap_err() {
                AiMuxError::UnsupportedFunctionality(msg) => {
                    assert!(msg.contains("gpt-realtime-whisper"));
                }
                e => panic!("expected Unsupported error, got: {e:?}"),
            }
        }

        /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should pass the model
        #[tokio::test]
        async fn should_pass_the_model() {
            let server = MockServer::start().await;
            mock_json_response(&server, &fixture_response()).await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.transcription_model("whisper-1").unwrap().unwrap();

            model
                .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
                .await
                .unwrap();

            let requests = server.received_requests().await.expect("requests recorded");
            assert_eq!(requests.len(), 1);
            let form = parse_multipart_form(&requests[0].body);
            assert_eq!(form.get("model").unwrap(), "whisper-1");
        }

        /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should default whisper-1 to verbose_json response format
        #[tokio::test]
        async fn should_default_whisper_1_to_verbose_json() {
            let server = MockServer::start().await;
            mock_json_response(&server, &fixture_response()).await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.transcription_model("whisper-1").unwrap().unwrap();

            let result = model
                .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
                .await
                .unwrap();

            let requests = server.received_requests().await.expect("requests recorded");
            assert_eq!(requests.len(), 1);
            let form = parse_multipart_form(&requests[0].body);
            assert_eq!(form.get("model").unwrap(), "whisper-1");
            assert_eq!(form.get("response_format").unwrap(), "verbose_json");
            assert_eq!(result.duration_in_seconds, Some(36.709999084472656));
        }

        /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should extract the transcription text
        #[tokio::test]
        async fn should_extract_the_transcription_text() {
            let server = MockServer::start().await;
            mock_json_response(&server, &fixture_response()).await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.transcription_model("whisper-1").unwrap().unwrap();

            let result = model
                .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
                .await
                .unwrap();

            assert_eq!(
                result.text,
                "Galileo was an American robotic space program that studied the planet Jupiter and its moons, as well as several other solar system bodies."
            );
        }

        /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should include response data with timestamp, modelId and headers
        #[tokio::test]
        async fn should_include_response_data() {
            let server = MockServer::start().await;
            mock_json_response_with_headers(
                &server,
                &fixture_response(),
                &[
                    ("x-request-id", "test-request-id"),
                    ("x-ratelimit-remaining", "123"),
                ],
            )
            .await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.transcription_model("whisper-1").unwrap().unwrap();

            let result = model
                .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
                .await
                .unwrap();

            assert!(result.response.timestamp.is_some());
            assert_eq!(result.response.model_id, Some("whisper-1".to_string()));
            let headers = result.response.headers.as_ref().unwrap();
            assert_eq!(headers.get("content-type").unwrap(), "application/json");
            assert_eq!(headers.get("x-request-id").unwrap(), "test-request-id");
            assert_eq!(headers.get("x-ratelimit-remaining").unwrap(), "123");
        }

        /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should use real date when no custom date provider is specified
        #[tokio::test]
        async fn should_use_real_date() {
            let server = MockServer::start().await;
            mock_json_response(&server, &fixture_response()).await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.transcription_model("whisper-1").unwrap().unwrap();

            let result = model
                .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
                .await
                .unwrap();

            assert!(result.response.timestamp.is_some());
            assert_eq!(result.response.model_id, Some("whisper-1".to_string()));
        }

        /// upstream: @ai-sdk/openai src/transcription/openai-transcription-model.test.ts › doGenerate › should handle segments with missing optional fields
        #[tokio::test]
        async fn should_handle_segments_with_missing_optional_fields() {
            let server = MockServer::start().await;
            mock_json_response(
                &server,
                &json!({
                    "task": "transcribe",
                    "text": "Test",
                    "segments": [
                        {
                            "id": 0, "seek": 0, "start": 0.0, "end": 1.0,
                            "text": "Test", "tokens": [1234],
                            "temperature": 0.0, "avg_logprob": -0.5,
                            "compression_ratio": 1.0, "no_speech_prob": 0.1
                        }
                    ],
                    "_request_id": "req_1234"
                }),
            )
            .await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.transcription_model("whisper-1").unwrap().unwrap();

            let result = model
                .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
                .await
                .unwrap();

            assert_eq!(result.segments.len(), 1);
            assert_eq!(result.segments[0].text, "Test");
            assert_eq!(result.segments[0].start_second, 0.0);
            assert_eq!(result.segments[0].end_second, 1.0);
            assert_eq!(result.language, None);
            assert_eq!(result.duration_in_seconds, None);
        }
    }

    /// partial: `should pass headers`; full port in src/openai/tests/transcription/transcription_model.rs.
    #[tokio::test]
    async fn should_pass_headers() {
        let server = MockServer::start().await;
        mock_json_response(&server, &fixture_response()).await;

        let mut provider_headers = HashMap::new();
        provider_headers.insert(
            "Custom-Provider-Header".to_string(),
            "provider-header-value".to_string(),
        );
        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            organization: Some("test-organization".into()),
            project: Some("test-project".into()),
            headers: Some(
                provider_headers
                    .into_iter()
                    .map(|(k, v)| (k, Some(v)))
                    .collect(),
            ),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.transcription_model("whisper-1").unwrap().unwrap();

        let mut options = transcription_options(mock_audio_data(), "audio/wav");
        let mut request_headers = HashMap::new();
        request_headers.insert(
            "Custom-Request-Header".to_string(),
            "request-header-value".to_string(),
        );
        options.headers = Some(request_headers);

        model.do_generate(&options).await.unwrap();

        let requests = server.received_requests().await.expect("requests recorded");
        assert_eq!(requests.len(), 1);
        let h = &requests[0].headers;
        assert_eq!(h.get("authorization").unwrap(), "Bearer test-api-key");
        assert_eq!(h.get("openai-organization").unwrap(), "test-organization");
        assert_eq!(h.get("openai-project").unwrap(), "test-project");
        assert_eq!(
            h.get("custom-provider-header").unwrap(),
            "provider-header-value"
        );
        assert_eq!(
            h.get("custom-request-header").unwrap(),
            "request-header-value"
        );
    }

    /// partial: `should work when no words, language, or duration are returned`; full port in src/openai/tests/transcription/transcription_model.rs.
    #[tokio::test]
    async fn should_work_without_optional_fields() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            &json!({
                "task": "transcribe",
                "text": "Hello from the Vercel AI SDK!",
                "_request_id": "req_1234"
            }),
        )
        .await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.transcription_model("whisper-1").unwrap().unwrap();

        let result = model
            .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
            .await
            .unwrap();

        assert_eq!(result.duration_in_seconds, None);
        assert_eq!(result.language, None);
        assert!(result.segments.is_empty());
        assert_eq!(result.text, "Hello from the Vercel AI SDK!");
        assert!(result.warnings.is_empty());
        assert!(result.response.timestamp.is_some());
        assert_eq!(result.response.model_id, Some("whisper-1".to_string()));
        let headers = result.response.headers.as_ref().unwrap();
        assert_eq!(headers.get("content-type").unwrap(), "application/json");
    }

    /// partial: `should handle empty segments array`; full port in src/openai/tests/transcription/transcription_model.rs.
    #[tokio::test]
    async fn should_handle_empty_segments() {
        let server = MockServer::start().await;
        mock_json_response(
            &server,
            &json!({
                "task": "transcribe",
                "text": "Hello world",
                "segments": [],
                "language": "english",
                "duration": 2.0,
                "_request_id": "req_1234"
            }),
        )
        .await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.transcription_model("whisper-1").unwrap().unwrap();

        let result = model
            .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
            .await
            .unwrap();

        assert!(result.segments.is_empty());
        assert_eq!(result.text, "Hello world");
    }

    /// The diarized fixture response (`__fixtures__/openai-diarized-transcription.json`).
    fn diarized_response() -> Value {
        json!({
            "task": "transcribe",
            "duration": 3.2,
            "text": "Hello from Alice. Hello from Bob.",
            "segments": [
                {"type": "transcript.text.segment", "id": "seg_001", "start": 0, "end": 1.5, "text": "Hello from Alice.", "speaker": "A"},
                {"type": "transcript.text.segment", "id": "seg_002", "start": 1.5, "end": 3.2, "text": "Hello from Bob.", "speaker": "B"}
            ]
        })
    }

    /// Sends one transcription with `provider_options` and returns the form.
    async fn diarize_form(provider_options: Option<Value>) -> HashMap<String, String> {
        let server = MockServer::start().await;
        mock_json_response(&server, &diarized_response()).await;
        let provider = create_openai(OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        })
        .unwrap();
        let model = provider
            .transcription_model("gpt-4o-transcribe-diarize")
            .unwrap()
            .unwrap();
        let mut options = transcription_options(mock_audio_data(), "audio/mpeg");
        options.provider_options = provider_options
            .map(|value| serde_json::from_value(value).expect("provider options are an object"));
        model.do_generate(&options).await.unwrap();
        let requests = server.received_requests().await.expect("requests recorded");
        parse_multipart_form(&requests[0].body)
    }

    /// `gpt-4o-transcribe-diarize` defaults to `diarized_json` with `auto`
    /// chunking; `responseFormat` and `chunkingStrategy` (`"auto"` or a server-VAD
    /// object sent as snake_case JSON) are passed through
    /// (`transcription/openai-transcription-model.ts:196-259`).
    #[tokio::test]
    async fn diarization_response_format_and_chunking_strategy() {
        let form = diarize_form(None).await;
        assert_eq!(form["response_format"], "diarized_json");
        assert_eq!(form["chunking_strategy"], "auto");
        assert!(!form.contains_key("temperature"));

        let form = diarize_form(Some(json!({
            "openai": {"responseFormat": "json", "chunkingStrategy": "auto"}
        })))
        .await;
        assert_eq!(form["response_format"], "json");
        assert_eq!(form["chunking_strategy"], "auto");

        let form = diarize_form(Some(json!({
        "openai": {"chunkingStrategy": {
            "type": "server_vad", "threshold": 0.7, "prefixPaddingMs": 400, "silenceDurationMs": 300
        }}
    })))
    .await;
        assert_eq!(form["response_format"], "diarized_json");
        assert_eq!(
            form["chunking_strategy"],
            r#"{"type":"server_vad","threshold":0.7,"prefix_padding_ms":400,"silence_duration_ms":300}"#
        );
    }

    /// Diarized segments come back as the usual segments plus
    /// `providerMetadata.openai.segments` with each speaker
    /// (`transcription/openai-transcription-model.ts:315-353`).
    #[tokio::test]
    async fn diarized_segments_carry_the_speaker_in_provider_metadata() {
        let server = MockServer::start().await;
        mock_json_response(&server, &diarized_response()).await;
        let provider = create_openai(OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        })
        .unwrap();
        let model = provider
            .transcription_model("gpt-4o-transcribe-diarize")
            .unwrap()
            .unwrap();
        let result = model
            .do_generate(&transcription_options(mock_audio_data(), "audio/mpeg"))
            .await
            .unwrap();

        assert_eq!(result.segments.len(), 2);
        assert_eq!(result.segments[1].text, "Hello from Bob.");
        assert_eq!(
            serde_json::to_value(result.provider_metadata).unwrap(),
            json!({"openai": {"segments": [
                {"text": "Hello from Alice.", "startSecond": 0.0, "endSecond": 1.5, "speaker": "A"},
                {"text": "Hello from Bob.", "startSecond": 1.5, "endSecond": 3.2, "speaker": "B"}
            ]}})
        );
    }
}

// ── Older ports (previously `tests/openai_transcription_stream_test.rs`) ──
//
// Realtime transcription streaming tests (RFC-0028 Phase 1).
//
// Each test spins up a local WebSocket server (tokio-tungstenite `accept_hdr_async`),
// plays the OpenAI realtime endpoint's role (consume `session.update` /
// `input_audio_buffer.append` / `commit`, emit `transcription.delta` /
// `completed` events), and asserts on the `TranscriptionStreamPart` sequence
// produced by `do_stream`.
//
// These replace the TS mock-WebSocket tests that
// `openai_transcription_test.rs` omitted ("not practical to translate" —
// they became practical with RFC-0028's WebSocket infrastructure). The
// provider is built with `create_openai`, whose base URL also names the
// realtime endpoint (`ws://` for an `http://` base URL).

#[cfg(feature = "realtime")]
mod older_transcription_stream {
    use std::time::Duration;

    use futures::{SinkExt, StreamExt};
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;

    use crate::{OpenAIProvider, OpenAIProviderSettings, create_openai};
    use aimux_core::AbortSignal;
    use aimux_core::error::AiMuxError;
    use aimux_core::transcription_model::{
        AudioChunk, InputAudioFormat, TranscriptionModel, TranscriptionStreamOptions,
        TranscriptionStreamPart,
    };

    // ── helpers ─────────────────────────────────────────────────────────────────

    /// A local WebSocket "OpenAI realtime" server. Handshake + `session.update`
    /// ack, then run the scripted behavior.
    struct RealtimeServer {
        port: u16,
    }

    impl RealtimeServer {
        /// Start a listener; the accept loop runs on a spawned task.
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                // Accept a single connection (tests make one).
                let (stream, _) = listener.accept().await.unwrap();
                let mut ws = accept_realtime(stream).await;
                serve(&mut ws).await;
            });
            Self { port }
        }

        fn base_url(&self) -> String {
            format!("http://127.0.0.1:{}", self.port)
        }
    }

    /// Server behavior shared by the happy-path tests: read session.update,
    /// ack it, collect audio appends until commit, then emit delta + completed,
    /// and finally assert the client closes with code 1000.
    async fn serve(ws: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) {
        // 1. Read session.update, capture the model + format from it.
        let mut session_model = String::new();
        let mut format_type = String::new();
        if let Some(Ok(Message::Text(text))) = ws.next().await {
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(
                v["type"], "session.update",
                "first message must be session.update"
            );
            assert_eq!(v["session"]["type"], "transcription");
            session_model = v["session"]["audio"]["input"]["transcription"]["model"]
                .as_str()
                .unwrap()
                .to_string();
            format_type = v["session"]["audio"]["input"]["format"]["type"]
                .as_str()
                .unwrap()
                .to_string();
            assert!(
                v["session"]["audio"]["input"]["turn_detection"].is_null(),
                "turn_detection must be null (nested under audio.input)"
            );
            ws.send(Message::Text(r#"{"type":"session.created"}"#.to_string()))
                .await
                .unwrap();
            ws.send(Message::Text(r#"{"type":"session.updated"}"#.to_string()))
                .await
                .unwrap();
        }
        let _ = (&session_model, &format_type);

        // 2. Collect audio appends until commit.
        let mut appended: Vec<String> = Vec::new();
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
                    match v["type"].as_str().unwrap_or("") {
                        "input_audio_buffer.append" => {
                            appended.push(v["audio"].as_str().unwrap().to_string());
                        }
                        "input_audio_buffer.commit" => break,
                        other => panic!("unexpected client message: {other}"),
                    }
                }
                other => panic!("expected client text message, got {other:?}"),
            }
        }
        assert!(
            !appended.is_empty(),
            "client must append at least one audio chunk"
        );

        // 3. Emit transcript deltas + completed (the shape OpenAI sends).
        ws.send(Message::Text(
        r#"{"type":"conversation.item.input_audio_transcription.delta","item_id":"item-1","delta":"Hello"}"#.to_string(),
    ))
    .await
    .unwrap();
        ws.send(Message::Text(
        r#"{"type":"conversation.item.input_audio_transcription.delta","item_id":"item-1","delta":" world"}"#.to_string(),
    ))
    .await
    .unwrap();
        ws.send(Message::Text(
        r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"item-1","transcript":"Hello world"}"#.to_string(),
    ))
    .await
    .unwrap();

        // 4. Expect the client to close with code 1000 (normal).
        match ws.next().await {
            Some(Ok(Message::Close(frame))) => {
                let code = frame.map(|f| u16::from(f.code));
                assert_eq!(code, Some(1000), "client must close(1000) after completion");
            }
            other => panic!("expected close frame, got {other:?}"),
        }
    }

    /// Accept the handshake the way the OpenAI realtime endpoint does: the client
    /// offers `realtime` plus `openai-insecure-api-key.<key>` and sends no
    /// Authorization header (`transcription/openai-transcription-model.ts:620-652`);
    /// the server selects `realtime`.
    async fn accept_realtime(
        stream: tokio::net::TcpStream,
    ) -> tokio_tungstenite::WebSocketStream<tokio::net::TcpStream> {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
        #[allow(clippy::result_large_err)]
        let handshake = |request: &Request, mut response: Response| {
            let offered = request
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            assert!(
                offered.starts_with("realtime, openai-insecure-api-key."),
                "{offered}"
            );
            assert!(request.headers().get("authorization").is_none());
            response
                .headers_mut()
                .insert("sec-websocket-protocol", "realtime".parse().unwrap());
            Ok(response)
        };
        tokio_tungstenite::accept_hdr_async(stream, handshake)
            .await
            .unwrap()
    }

    /// `create_openai` pointed at `base_url` with `api_key`.
    fn openai_at(base_url: impl Into<String>, api_key: &str) -> OpenAIProvider {
        create_openai(OpenAIProviderSettings {
            api_key: Some(api_key.to_string()),
            base_url: Some(base_url.into()),
            ..Default::default()
        })
        .expect("valid settings")
    }

    fn realtime_model(base_url: &str) -> crate::OpenAITranscriptionModel {
        openai_at(base_url, "test-api-key").transcription("gpt-realtime-whisper")
    }

    fn stream_options(
        chunks: Vec<AudioChunk>,
        abort: Option<AbortSignal>,
    ) -> TranscriptionStreamOptions {
        TranscriptionStreamOptions {
            audio: Box::pin(futures::stream::iter(chunks)),
            input_audio_format: InputAudioFormat {
                format_type: "audio/pcm".to_string(),
                rate: Some(24000),
            },
            provider_options: None,
            abort_signal: abort,
            headers: None,
            include_raw_chunks: false,
            timeout: None,
        }
    }

    /// Collect all parts from a do_stream result into a Vec (Err passthrough).
    async fn collect(
        result: aimux_core::transcription_model::TranscriptionStreamResult,
    ) -> Vec<Result<TranscriptionStreamPart, AiMuxError>> {
        let mut out = Vec::new();
        let mut stream = result.stream;
        while let Some(part) = stream.next().await {
            out.push(part);
        }
        out
    }

    // ── tests ───────────────────────────────────────────────────────────────────

    /// partial: `should stream gpt-realtime-whisper using OpenAI realtime transcription`; full port in src/openai/tests/transcription/transcription_model.rs.
    /// Happy path: connect → session.update → appends → commit → deltas →
    /// completed → final + finish → client close(1000).
    #[tokio::test]
    async fn stream_realtime_happy_path() {
        let server = RealtimeServer::start().await;
        let model = realtime_model(&server.base_url());

        let options = stream_options(
            vec![
                AudioChunk::Binary(vec![1, 2, 3]),
                AudioChunk::Binary(vec![4, 5, 6]),
            ],
            None,
        );
        let result = model
            .do_stream(options)
            .expect("the OpenAI transcription model streams")
            .await
            .expect("do_stream should connect");
        let parts = collect(result).await;

        // StreamStart + 2 deltas + final + finish.
        assert_eq!(parts.len(), 5, "parts: {parts:?}");

        assert!(matches!(
            &parts[0],
            Ok(TranscriptionStreamPart::StreamStart { .. })
        ));

        match &parts[1] {
            Ok(TranscriptionStreamPart::TranscriptDelta { id, delta, .. }) => {
                assert_eq!(id.as_deref(), Some("item-1"));
                assert_eq!(delta, "Hello");
            }
            other => panic!("expected delta 'Hello', got {other:?}"),
        }
        match &parts[2] {
            Ok(TranscriptionStreamPart::TranscriptDelta { delta, .. }) => {
                assert_eq!(delta, " world");
            }
            other => panic!("expected delta ' world', got {other:?}"),
        }
        match &parts[3] {
            Ok(TranscriptionStreamPart::TranscriptFinal { id, text, .. }) => {
                assert_eq!(id.as_deref(), Some("item-1"));
                assert_eq!(text, "Hello world");
            }
            other => panic!("expected final, got {other:?}"),
        }
        match &parts[4] {
            Ok(TranscriptionStreamPart::Finish { text, segments, .. }) => {
                assert_eq!(text, "Hello world");
                assert!(segments.is_empty());
            }
            other => panic!("expected finish, got {other:?}"),
        }
    }

    /// The request snapshot records the exact session.update wire shape.
    #[tokio::test]
    async fn stream_request_carries_session_update() {
        let server = RealtimeServer::start().await;
        let model = realtime_model(&server.base_url());

        let options = stream_options(vec![AudioChunk::Binary(vec![1])], None);
        let result = model
            .do_stream(options)
            .expect("the OpenAI transcription model streams")
            .await
            .unwrap();
        let body = result
            .request
            .as_ref()
            .and_then(|r| r.body.as_ref())
            .expect("request body snapshot");
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        // B1 regression guard: model in audio.input.transcription.model (not URL),
        // turn_detection nested under audio.input, format carries the rate.
        assert_eq!(v["type"], "session.update");
        assert_eq!(
            v["session"]["audio"]["input"]["transcription"]["model"],
            "gpt-realtime-whisper"
        );
        assert_eq!(
            v["session"]["audio"]["input"]["format"]["type"],
            "audio/pcm"
        );
        assert_eq!(v["session"]["audio"]["input"]["format"]["rate"], 24000);
        assert!(v["session"]["audio"]["input"]["turn_detection"].is_null());
        drop(result);
    }

    /// Binary audio chunks are base64-encoded in the append messages.
    #[tokio::test]
    async fn stream_audio_chunks_are_base64() {
        // Server variant that asserts the base64 payload.
        struct AssertBase64Server;
        let _ = AssertBase64Server;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_realtime(stream).await;
            // session.update
            let _ = ws.next().await;
            // first append — verify base64 of [1,2,3]
            if let Some(Ok(Message::Text(text))) = ws.next().await {
                let v: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(v["type"], "input_audio_buffer.append");
                assert_eq!(v["audio"], "AQID", "binary [1,2,3] must arrive as base64");
            }
            // second append — Base64 chunk passes through verbatim
            if let Some(Ok(Message::Text(text))) = ws.next().await {
                let v: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(
                    v["audio"], "cGFzcw==",
                    "base64 chunk must pass through as-is"
                );
            }
            // commit
            let _ = ws.next().await;
            // complete the session so the client closes cleanly
            ws.send(Message::Text(
            r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"i","transcript":"x"}"#
                .to_string(),
        ))
        .await
        .unwrap();
            let _ = ws.next().await; // close
        });

        let model = realtime_model(&format!("http://127.0.0.1:{port}"));
        let options = stream_options(
            vec![
                AudioChunk::Binary(vec![1, 2, 3]),
                AudioChunk::Base64("cGFzcw==".to_string()),
            ],
            None,
        );
        let result = model
            .do_stream(options)
            .expect("the OpenAI transcription model streams")
            .await
            .unwrap();
        let parts = collect(result).await;
        // StreamStart + final + finish (no deltas in this server script).
        assert_eq!(parts.len(), 3, "parts: {parts:?}");
    }

    /// Abort mid-session surfaces `AiMuxError::Aborted`.
    #[tokio::test]
    async fn stream_abort_mid_session() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let abort = AbortSignal::new();
        let abort_clone = abort.clone();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_realtime(stream).await;
            let _ = ws.next().await; // session.update
            // Hold the connection open (drain client messages without acting) so
            // the abort — not a server-side disconnect — is what ends the session.
            while let Some(_msg) = ws.next().await {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        let model = realtime_model(&format!("http://127.0.0.1:{port}"));
        let options = stream_options(vec![AudioChunk::Binary(vec![1])], Some(abort_clone));

        let result = model
            .do_stream(options)
            .expect("the OpenAI transcription model streams")
            .await
            .unwrap();
        // Abort shortly after the stream starts.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            abort.abort();
        });
        let parts = collect(result).await;
        assert!(
            parts
                .iter()
                .any(|p| matches!(p, Err(AiMuxError::Aborted(_)))),
            "expected Aborted in parts: {parts:?}"
        );
    }

    /// Non-realtime models are rejected from do_stream (inverse gating).
    #[tokio::test]
    async fn stream_rejects_non_realtime_models() {
        let provider = openai_at("http://127.0.0.1:1", "k");
        let model = provider.transcription("whisper-1");
        let err = model
            .do_stream(stream_options(vec![], None))
            .expect("the OpenAI transcription model streams")
            .await
            .unwrap_err();
        match err {
            AiMuxError::UnsupportedFunctionality(msg) => {
                assert!(msg.contains("whisper-1"), "got: {msg}");
            }
            other => panic!("expected UnsupportedFunctionality, got {other:?}"),
        }
    }

    /// A closed port (connection refused) — `do_stream` itself must fail with a
    /// clear connect error, not an in-stream error part (#118).
    #[tokio::test]
    async fn stream_connect_failure_errors() {
        // Port 1 is never listening on loopback: TCP refuses immediately.
        let provider = openai_at("http://127.0.0.1:1".to_string(), "k");
        let model = provider.transcription("gpt-realtime-whisper");

        let err = model
            .do_stream(stream_options(vec![AudioChunk::Binary(vec![1])], None))
            .expect("the OpenAI transcription model streams")
            .await
            .unwrap_err();
        match &err {
            AiMuxError::ApiCall(d) => {
                assert!(
                    d.message.contains("websocket connect failed"),
                    "connect failure must name the phase, got: {}",
                    d.message
                );
            }
            other => panic!("expected ApiCall connect error, got {other:?}"),
        }
    }

    /// Abort while the WS handshake is still in flight — `Aborted`, not a
    /// timeout and not an IO error (#118).
    #[tokio::test]
    async fn stream_abort_during_connect() {
        // A black-hole server: accepts TCP but never completes the WS handshake,
        // so `connect_async` pends and only the abort can end the connect phase.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            // Hold the socket open without writing the 101 response.
            let _hold = stream;
            std::future::pending::<()>().await;
        });

        let abort = AbortSignal::new();
        let abort_clone = abort.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            abort.abort();
        });

        let model = realtime_model(&format!("http://127.0.0.1:{port}"));
        let err = model
            .do_stream(stream_options(
                vec![AudioChunk::Binary(vec![1])],
                Some(abort_clone),
            ))
            .expect("the OpenAI transcription model streams")
            .await
            .unwrap_err();
        // Abort semantics exactly: distinct from timeout and from IO failure.
        assert!(
            matches!(err, AiMuxError::Aborted(_)),
            "connect-phase abort must surface Aborted, got {err:?}"
        );
        assert!(!matches!(&err, AiMuxError::Timeout(_)));
        assert!(!matches!(&err, AiMuxError::ApiCall(_)));
    }

    /// `first_chunk_ms` doubles as the connect budget: against a black-hole
    /// server the CONNECT phase itself must time out with its own message —
    /// distinguishable from both the first-EVENT timeout and a connect IO
    /// failure (#118).
    #[tokio::test]
    async fn stream_connect_timeout_fires() {
        // Accepts TCP but never answers the WS handshake — connect pends.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let _hold = stream;
            std::future::pending::<()>().await;
        });

        let provider = openai_at(format!("http://127.0.0.1:{port}"), "k");
        let model = provider.transcription("gpt-realtime-whisper");

        let options = TranscriptionStreamOptions {
            audio: Box::pin(futures::stream::iter(vec![AudioChunk::Binary(vec![1])])),
            input_audio_format: InputAudioFormat {
                format_type: "audio/pcm".to_string(),
                rate: None,
            },
            provider_options: None,
            abort_signal: None,
            headers: None,
            include_raw_chunks: false,
            timeout: Some(aimux_core::options::TimeoutConfiguration {
                first_chunk_ms: Some(300),
                total_ms: None,
                step_ms: None,
                chunk_ms: None,
            }),
        };
        let err = model
            .do_stream(options)
            .expect("the OpenAI transcription model streams")
            .await
            .unwrap_err();
        match &err {
            AiMuxError::Timeout(msg) => {
                assert_eq!(
                    msg, "websocket connect timed out",
                    "connect timeout must carry its own message"
                );
                assert!(
                    !msg.contains("first websocket event"),
                    "must not be confused with the first-event timeout"
                );
            }
            other => panic!("expected connect Timeout, got {other:?}"),
        }
    }

    /// A silent server (accepts + reads session.update, then goes quiet) — the
    /// first-chunk timeout must fire (B1/timer coverage).
    #[tokio::test]
    async fn stream_first_chunk_timeout_fires() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_realtime(stream).await;
            // Read session.update, then send NOTHING but KEEP the socket alive
            // (drain client appends) well past the client's 500ms first-chunk
            // deadline — the timeout must fire before any disconnect could.
            let _ = ws.next().await;
            while let Ok(Some(_)) = tokio::time::timeout(Duration::from_secs(2), ws.next()).await {}
        });

        let provider = openai_at(format!("http://127.0.0.1:{port}"), "k");
        let model = provider.transcription("gpt-realtime-whisper");

        let options = TranscriptionStreamOptions {
            audio: Box::pin(futures::stream::iter(vec![AudioChunk::Binary(vec![1])])),
            input_audio_format: InputAudioFormat {
                format_type: "audio/pcm".to_string(),
                rate: None,
            },
            provider_options: None,
            abort_signal: None,
            headers: None,
            include_raw_chunks: false,
            timeout: Some(aimux_core::options::TimeoutConfiguration {
                // first_chunk covers connect + session ack; 500ms is generous on
                // loopback but far below the server's 2s hold.
                first_chunk_ms: Some(500),
                total_ms: None,
                step_ms: None,
                chunk_ms: None,
            }),
        };
        let result = model
            .do_stream(options)
            .expect("the OpenAI transcription model streams")
            .await
            .unwrap();
        let parts = collect(result).await;
        let timed_out = parts
            .iter()
            .any(|p| matches!(p, Err(AiMuxError::Timeout(_))));
        assert!(
            timed_out,
            "expected a Timeout error against a silent server; got: {parts:?}"
        );
    }

    /// partial: `should error the stream with the server message on error events`; full port in src/openai/tests/transcription/transcription_model.rs.
    /// The server emits a realtime `error` event — the provider message must
    /// propagate verbatim and readable (not wrapped in transport noise), and the
    /// session must terminate there (#118).
    #[tokio::test]
    async fn stream_server_error_event_propagates() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_realtime(stream).await;
            let _ = ws.next().await; // session.update
            ws.send(Message::Text(
            r#"{"type":"error","error":{"message":"insufficient quota for realtime transcription","type":"insufficient_quota"}}"#
                .to_string(),
        ))
        .await
        .unwrap();
            // The client closes(1000) after surfacing the error.
            let _ = ws.next().await;
        });

        let model = realtime_model(&format!("http://127.0.0.1:{port}"));
        let result = model
            .do_stream(stream_options(vec![AudioChunk::Binary(vec![1])], None))
            .expect("the OpenAI transcription model streams")
            .await
            .unwrap();
        let parts = collect(result).await;

        // StreamStart then the terminal error — nothing after it.
        assert_eq!(parts.len(), 2, "parts: {parts:?}");
        assert!(matches!(
            &parts[0],
            Ok(TranscriptionStreamPart::StreamStart { .. })
        ));
        match &parts[1] {
            Err(AiMuxError::ApiCall(d)) => {
                assert_eq!(
                    d.message, "insufficient quota for realtime transcription",
                    "the provider's own message must propagate verbatim"
                );
                assert!(
                    !d.message.contains("websocket"),
                    "must not be wrapped in transport noise: {}",
                    d.message
                );
            }
            other => panic!("expected ApiCall from server error event, got {other:?}"),
        }
    }

    /// The server closes the socket (close frame with code + reason) after the
    /// handshake — the client must surface peer-closed semantics carrying the
    /// code/reason, not a bare EOF or a hang (#118).
    #[tokio::test]
    async fn stream_peer_close_before_completion() {
        use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
        use tokio_tungstenite::tungstenite::protocol::frame::{CloseFrame as WsCloseFrame, Frame};

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_realtime(stream).await;
            let _ = ws.next().await; // session.update
            // Server-initiated abnormal close: code 1011 + a human reason.
            let frame = WsCloseFrame {
                code: CloseCode::Error,
                reason: std::borrow::Cow::Borrowed("server overloaded"),
            };
            ws.send(Message::Frame(Frame::close(Some(frame))))
                .await
                .unwrap();
            // Drain until the client side tears down.
            let _ = ws.next().await;
        });

        let model = realtime_model(&format!("http://127.0.0.1:{port}"));
        let result = model
            .do_stream(stream_options(vec![AudioChunk::Binary(vec![1])], None))
            .expect("the OpenAI transcription model streams")
            .await
            .unwrap();
        let parts = collect(result).await;

        assert_eq!(parts.len(), 2, "parts: {parts:?}");
        match &parts[1] {
            Err(AiMuxError::ApiCall(d)) => {
                assert!(
                    d.message.contains("websocket closed by peer"),
                    "peer close must be identifiable, got: {}",
                    d.message
                );
                assert!(
                    d.message.contains("1011") && d.message.contains("server overloaded"),
                    "close code and reason must be carried: {}",
                    d.message
                );
            }
            other => panic!("expected ApiCall peer-closed error, got {other:?}"),
        }
    }

    /// chunk-idle timeout: the server delivers one event, then goes silent while
    /// holding the socket open — `chunk_ms` must trip with its own message,
    /// distinct from the total timeout (#118).
    #[tokio::test]
    async fn stream_chunk_idle_timeout_fires() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_realtime(stream).await;
            let _ = ws.next().await; // session.update
            // One event so the stream is established and alive…
            ws.send(Message::Text(r#"{"type":"session.created"}"#.to_string()))
                .await
                .unwrap();
            // …then total silence while KEEPING the socket open (drain client
            // messages for 2s, far past the 400ms idle window) — the idle timer,
            // not a disconnect, must end the session.
            while let Ok(Some(_)) = tokio::time::timeout(Duration::from_secs(2), ws.next()).await {}
        });

        let provider = openai_at(format!("http://127.0.0.1:{port}"), "k");
        let model = provider.transcription("gpt-realtime-whisper");

        let options = TranscriptionStreamOptions {
            audio: Box::pin(futures::stream::iter(vec![AudioChunk::Binary(vec![1])])),
            input_audio_format: InputAudioFormat {
                format_type: "audio/pcm".to_string(),
                rate: None,
            },
            provider_options: None,
            abort_signal: None,
            headers: None,
            include_raw_chunks: false,
            timeout: Some(aimux_core::options::TimeoutConfiguration {
                first_chunk_ms: None,
                total_ms: None,
                step_ms: None,
                chunk_ms: Some(400),
            }),
        };
        let result = model
            .do_stream(options)
            .expect("the OpenAI transcription model streams")
            .await
            .unwrap();
        let parts = collect(result).await;

        assert_eq!(parts.len(), 2, "parts: {parts:?}");
        match &parts[1] {
            Err(AiMuxError::Timeout(msg)) => {
                assert_eq!(
                    msg, "websocket chunk idle timeout",
                    "chunk-idle must fire with its own message"
                );
                assert!(
                    !msg.contains("total"),
                    "must not be confused with the total timeout"
                );
            }
            other => panic!("expected chunk-idle Timeout, got {other:?}"),
        }
    }

    /// total timeout: the server keeps events flowing (so chunk-idle never
    /// trips) — the hard connection deadline must still fire, with a message
    /// distinct from chunk-idle (#118).
    #[tokio::test]
    async fn stream_total_timeout_fires() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_realtime(stream).await;
            let _ = ws.next().await; // session.update
            // Keep the stream visibly alive: a delta every 100ms for 2s — well
            // inside the 300ms chunk window, well past the 700ms total deadline.
            for i in 0..20 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let event = format!(
                    r#"{{"type":"conversation.item.input_audio_transcription.delta","item_id":"i","delta":"{i}"}}"#
                );
                if ws.send(Message::Text(event)).await.is_err() {
                    break; // client tore down after the total timeout
                }
            }
        });

        let provider = openai_at(format!("http://127.0.0.1:{port}"), "k");
        let model = provider.transcription("gpt-realtime-whisper");

        let options = TranscriptionStreamOptions {
            audio: Box::pin(futures::stream::iter(vec![AudioChunk::Binary(vec![1])])),
            input_audio_format: InputAudioFormat {
                format_type: "audio/pcm".to_string(),
                rate: None,
            },
            provider_options: None,
            abort_signal: None,
            headers: None,
            include_raw_chunks: false,
            timeout: Some(aimux_core::options::TimeoutConfiguration {
                first_chunk_ms: None,
                total_ms: Some(700),
                step_ms: None,
                chunk_ms: Some(300),
            }),
        };
        let result = model
            .do_stream(options)
            .expect("the OpenAI transcription model streams")
            .await
            .unwrap();
        let parts = collect(result).await;

        // StreamStart + several deltas (alive stream) + the terminal total error.
        let deltas = parts
            .iter()
            .filter(|p| matches!(p, Ok(TranscriptionStreamPart::TranscriptDelta { .. })))
            .count();
        assert!(
            deltas >= 2,
            "stream must be alive pre-timeout; parts: {parts:?}"
        );
        let last = parts.last().expect("must end with an error part");
        match last {
            Err(AiMuxError::Timeout(msg)) => {
                assert_eq!(
                    msg, "websocket total timeout",
                    "total deadline must fire with its own message"
                );
                assert!(
                    !msg.contains("chunk"),
                    "must not be confused with the chunk-idle timeout"
                );
            }
            other => panic!("expected total Timeout as the last part, got {other:?}"),
        }
    }
}

// ── Older ports (previously `tests/openai_realtime_ws_replay_test.rs`) ──
//
// Offline replay of the OpenAI realtime transcription WebSocket path
// (RFC-0028 §3.2), driven through the connector that
// `OpenAITranscriptionModel::with_web_socket` installs.
//
// `OpenAIProviderSettings::web_socket` carries a custom [`WsConnector`] that
// never touches the network: it pairs the client with an in-process server
// over an in-memory duplex pipe. The server records every frame the client
// sends and replays the server events in
// `fixtures/openai_realtime_transcription.json` once the audio is committed.

#[cfg(feature = "realtime")]
mod older_realtime_ws_replay {
    use std::sync::{Arc, Mutex};

    use crate::{OpenAIProviderSettings, create_openai};
    use aimux_core::error::AiMuxError;
    use aimux_core::transcription_model::{
        AudioChunk, InputAudioFormat, TranscriptionModel, TranscriptionStreamOptions,
        TranscriptionStreamPart,
    };
    use aimux_provider_utils::ws::{WebSocketRequest, WsConnection, WsConnector};
    use futures::{SinkExt, StreamExt};
    use serde_json::Value;
    use tokio_tungstenite::WebSocketStream;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::protocol::Role;

    /// URL, headers and subprotocols of one connect request.
    type Connect = (String, Vec<(String, String)>, Vec<String>);

    /// What the in-process server saw.
    #[derive(Default)]
    struct Seen {
        /// Every connect request, in order.
        connects: Vec<Connect>,
        /// Every text frame the client sent, parsed.
        frames: Vec<Value>,
        /// Whether the client closed the socket.
        closed: bool,
    }

    /// A connector that serves the fixture over an in-memory pipe.
    struct ReplayConnector {
        seen: Arc<Mutex<Seen>>,
        events: Vec<Value>,
    }

    #[async_trait::async_trait]
    impl WsConnector for ReplayConnector {
        async fn connect(&self, request: &WebSocketRequest) -> Result<WsConnection, AiMuxError> {
            self.seen.lock().unwrap().connects.push((
                request.url.clone(),
                request.headers.clone(),
                request.subprotocols.clone(),
            ));
            let (client_io, server_io) = tokio::io::duplex(64 * 1024);
            let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
            let server = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
            tokio::spawn(serve(server, Arc::clone(&self.seen), self.events.clone()));
            Ok(WsConnection::from_transport(Box::new(client), request))
        }
    }

    /// Record client frames; after `input_audio_buffer.commit`, replay `events`.
    async fn serve(
        mut ws: WebSocketStream<tokio::io::DuplexStream>,
        seen: Arc<Mutex<Seen>>,
        events: Vec<Value>,
    ) {
        while let Some(message) = ws.next().await {
            match message {
                Ok(Message::Text(text)) => {
                    let frame: Value = serde_json::from_str(&text).expect("client sends JSON");
                    let commit = frame["type"] == "input_audio_buffer.commit";
                    seen.lock().unwrap().frames.push(frame);
                    if commit {
                        for event in &events {
                            ws.send(Message::Text(event.to_string()))
                                .await
                                .expect("replay event");
                        }
                    }
                }
                Ok(Message::Close(_)) | Err(_) => {
                    seen.lock().unwrap().closed = true;
                    break;
                }
                Ok(_) => {}
            }
        }
    }

    fn fixture_events() -> Vec<Value> {
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/openai_realtime_transcription.json"
        )))
        .expect("fixture is JSON");
        fixture["events"].as_array().expect("events array").clone()
    }

    /// partial: `should stream gpt-realtime-whisper using OpenAI realtime transcription`; full port in src/openai/tests/transcription/transcription_model.rs.
    #[tokio::test]
    async fn realtime_transcription_replays_through_injected_web_socket() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let connector = ReplayConnector {
            seen: Arc::clone(&seen),
            events: fixture_events(),
        };
        let provider = create_openai(OpenAIProviderSettings {
            api_key: Some("sk-replay-test".to_string()),
            base_url: Some("https://replay.invalid/v1".into()),
            web_socket: Some(Arc::new(connector)),
            ..Default::default()
        })
        .expect("valid settings");
        let model = provider.transcription("gpt-realtime-whisper");

        let options = TranscriptionStreamOptions {
            audio: Box::pin(futures::stream::iter(vec![
                AudioChunk::Binary(vec![1, 2, 3]),
                AudioChunk::Base64("BAUG".into()),
            ])),
            input_audio_format: InputAudioFormat {
                format_type: "audio/pcm".to_string(),
                rate: Some(24_000),
            },
            provider_options: None,
            abort_signal: None,
            headers: None,
            include_raw_chunks: false,
            timeout: None,
        };
        let result = model
            .do_stream(options)
            .expect("the OpenAI transcription model streams")
            .await
            .expect("stream opens");
        let parts: Vec<TranscriptionStreamPart> = result
            .stream
            .map(|part| part.expect("no stream error"))
            .collect()
            .await;

        // Transcript events, mapped from the replayed server events.
        let deltas: Vec<&str> = parts
            .iter()
            .filter_map(|part| match part {
                TranscriptionStreamPart::TranscriptDelta { delta, id, .. } => {
                    assert_eq!(id.as_deref(), Some("item_1"));
                    Some(delta.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(deltas, ["Hello", " world"]);
        assert!(parts.iter().any(|part| matches!(
            part,
            TranscriptionStreamPart::TranscriptFinal { text, id, .. }
                if text == "Hello world" && id.as_deref() == Some("item_1")
        )));
        assert!(matches!(
            parts.last(),
            Some(TranscriptionStreamPart::Finish { text, .. }) if text == "Hello world"
        ));
        assert!(matches!(
            parts.first(),
            Some(TranscriptionStreamPart::StreamStart { .. })
        ));

        // The injected connector carried the session: exactly one connect, to
        // the realtime URL derived from the base URL, with the credential.
        // The client closes the long-lived session after `completed` (B2).
        for _ in 0..100 {
            if seen.lock().unwrap().closed {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let seen = seen.lock().unwrap();
        assert!(seen.closed, "client did not close the session");
        assert_eq!(seen.connects.len(), 1, "with_web_socket connector not used");
        let (url, headers, subprotocols) = &seen.connects[0];
        assert_eq!(url, "wss://replay.invalid/v1/realtime?intent=transcription");
        // The key rides the `openai-insecure-api-key` subprotocol and the
        // Authorization header is dropped: OpenAI rejects a handshake that sends
        // both (`transcription/openai-transcription-model.ts:620-652`).
        assert_eq!(
            subprotocols,
            &["realtime", "openai-insecure-api-key.sk-replay-test"]
        );
        assert!(
            !headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        );

        // Frames the server saw: session.update, two appends, commit.
        let types: Vec<&str> = seen
            .frames
            .iter()
            .map(|frame| frame["type"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(
            types,
            [
                "session.update",
                "input_audio_buffer.append",
                "input_audio_buffer.append",
                "input_audio_buffer.commit",
            ]
        );
        let session = &seen.frames[0]["session"];
        assert_eq!(session["type"], "transcription");
        assert_eq!(
            session["audio"]["input"]["transcription"]["model"],
            "gpt-realtime-whisper"
        );
        assert_eq!(session["audio"]["input"]["format"]["type"], "audio/pcm");
        assert_eq!(session["audio"]["input"]["format"]["rate"], 24_000);
        assert_eq!(seen.frames[1]["audio"], "AQID");
        assert_eq!(seen.frames[2]["audio"], "BAUG");
    }

    /// Streams the fixture with `provider_options`; returns the parts and the
    /// `session.update` the client sent.
    async fn replay_with(provider_options: Value) -> (Vec<TranscriptionStreamPart>, Value) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let provider = create_openai(OpenAIProviderSettings {
            api_key: Some("sk-replay-test".to_string()),
            base_url: Some("https://replay.invalid/v1".into()),
            web_socket: Some(Arc::new(ReplayConnector {
                seen: Arc::clone(&seen),
                events: fixture_events(),
            })),
            ..Default::default()
        })
        .expect("valid settings");
        let options = TranscriptionStreamOptions {
            audio: Box::pin(futures::stream::iter(vec![AudioChunk::Binary(vec![
                1, 2, 3,
            ])])),
            input_audio_format: InputAudioFormat {
                format_type: "audio/pcm".to_string(),
                rate: Some(24_000),
            },
            provider_options: Some(serde_json::from_value(provider_options).unwrap()),
            abort_signal: None,
            headers: None,
            include_raw_chunks: false,
            timeout: None,
        };
        let result = provider
            .transcription("gpt-realtime-whisper")
            .do_stream(options)
            .expect("the OpenAI transcription model streams")
            .await
            .expect("stream opens");
        let parts = result
            .stream
            .map(|part| part.expect("no stream error"))
            .collect()
            .await;
        let session = seen.lock().unwrap().frames[0].clone();
        (parts, session)
    }

    /// `providerOptions.openai.streaming.delay` and `.include` configure the
    /// realtime session; the REST `include` does not
    /// (`transcription/openai-transcription-model.ts:580-617`).
    #[tokio::test]
    async fn realtime_session_takes_the_streaming_options() {
        let (_, session) = replay_with(serde_json::json!({"openai": {
            "language": "en",
            "include": ["logprobs"],
            "streaming": {"delay": "low", "include": ["item.input_audio_transcription.logprobs"]},
        }}))
        .await;
        assert_eq!(
            session["session"]["audio"]["input"]["transcription"],
            serde_json::json!({"model": "gpt-realtime-whisper", "language": "en", "delay": "low"})
        );
        assert_eq!(
            session["session"]["include"],
            serde_json::json!(["item.input_audio_transcription.logprobs"])
        );
    }

    /// `stream-start` warns about each REST-only option that is set
    /// (`transcription/openai-transcription-model.ts:395-410`).
    #[tokio::test]
    async fn realtime_stream_start_warns_about_rest_only_options() {
        let (parts, _) = replay_with(serde_json::json!({"openai": {
            "prompt": "context prompt",
            "temperature": 0.5,
            "language": "en",
        }}))
        .await;
        let Some(TranscriptionStreamPart::StreamStart { warnings }) = parts.first() else {
            panic!("expected stream-start first, got {parts:?}");
        };
        assert_eq!(
            serde_json::to_value(warnings).unwrap(),
            serde_json::json!([
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
            ])
        );
    }
}
