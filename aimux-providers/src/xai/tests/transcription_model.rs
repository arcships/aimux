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
//!
//! The older ports (formerly `tests/xai_transcription_test.rs`) follow the
//! wiremock harness helpers: they run against a loopback wiremock `MockServer`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::transcription_model::{
    AudioChunk, AudioInput, InputAudioFormat, TranscriptionCallOptions, TranscriptionModel,
    TranscriptionResponse, TranscriptionResult, TranscriptionStreamOptions,
    TranscriptionStreamPart,
};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};
use crate::xai::options::NAMESPACE;
use crate::{XAIProviderSettings, XaiTranscriptionModel, create_xai};

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

// ── The wiremock harness of the older ports ──────────────────────────────────

type Headers = HashMap<String, Option<String>>;

fn model(base_url: &str, headers: Option<Headers>) -> XaiTranscriptionModel {
    create_xai(XAIProviderSettings {
        api_key: Some("test-api-key".into()),
        base_url: Some(base_url.into()),
        headers,
        ..Default::default()
    })
    .expect("valid settings")
    .transcription()
}

fn wiremock_xai_options(value: Value) -> SharedProviderOptions {
    SharedProviderOptions::from([("xai".to_string(), value.as_object().unwrap().clone())])
}

// ── doGenerate helpers ──────────────────────────────────────────────────────

fn call(provider_options: Option<Value>, media_type: &str) -> TranscriptionCallOptions {
    let mut options =
        TranscriptionCallOptions::new(AudioInput::Binary(vec![1, 2, 3, 4]), media_type);
    options.provider_options = provider_options.map(wiremock_xai_options);
    options
}

fn json_response(body: Value, headers: &[(&str, &str)]) -> ResponseTemplate {
    headers.iter().fold(
        ResponseTemplate::new(200).set_body_json(body),
        |t, (k, v)| t.insert_header(*k, *v),
    )
}

async fn mock_stt(server: &MockServer, template: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path("/stt"))
        .respond_with(template)
        .mount(server)
        .await;
}

fn wiremock_default_body() -> Value {
    json!({
        "text": "Hello from the AI SDK!",
        "language": "en",
        "duration": 2.5,
        "words": [
            {"text": "Hello", "start": 0, "end": 1},
            {"text": "from the AI SDK!", "start": 1, "end": 2.5}
        ]
    })
}

async fn generate(
    server: &MockServer,
    options: &TranscriptionCallOptions,
    headers: Option<Headers>,
) -> TranscriptionResult {
    model(&server.uri(), headers)
        .do_generate(options)
        .await
        .expect("do_generate")
}

struct Part {
    name: String,
    filename: Option<String>,
    content_type: Option<String>,
    value: String,
}

fn quoted(line: &str, key: &str) -> Option<String> {
    let start = line.find(&format!(" {key}=\""))? + key.len() + 3;
    let end = line[start..].find('"')?;
    Some(line[start..start + end].to_string())
}

/// The multipart fields of a captured request, in wire order.
fn parse_multipart(request: &wiremock::Request) -> Vec<Part> {
    let content_type = request
        .headers
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    let boundary = content_type.split("boundary=").nth(1).unwrap();
    let body = String::from_utf8_lossy(&request.body).into_owned();
    body.split(&format!("--{boundary}"))
        .skip(1)
        .map(|chunk| chunk.trim_start_matches("\r\n"))
        .filter(|chunk| !chunk.starts_with("--"))
        .map(|chunk| {
            let (head, value) = chunk.split_once("\r\n\r\n").unwrap();
            let line = |prefix: &str| head.lines().find(|l| l.starts_with(prefix));
            Part {
                name: quoted(line("Content-Disposition").unwrap(), "name").unwrap(),
                filename: quoted(line("Content-Disposition").unwrap(), "filename"),
                content_type: line("Content-Type").map(|l| l["Content-Type: ".len()..].to_string()),
                value: value.strip_suffix("\r\n").unwrap_or(value).to_string(),
            }
        })
        .collect()
}

fn full_options() -> Value {
    json!({
        "audioFormat": "pcm", "sampleRate": 16000, "language": "en", "format": true,
        "multichannel": true, "channels": 2, "diarize": true,
        "keyterm": ["AI SDK", "Grok"], "fillerWords": true
    })
}

// ── doStream helpers ────────────────────────────────────────────────────────

#[derive(Default, Clone)]
struct Capture {
    uri: String,
    headers: HashMap<String, String>,
    /// Frames that arrived before the server sent `transcript.created`.
    early_frames: usize,
    binary: Vec<Vec<u8>>,
    texts: Vec<String>,
}

async fn serve(listener: TcpListener, cap: Arc<Mutex<Capture>>, events: Vec<Value>) {
    let (stream, _) = listener.accept().await.unwrap();
    let handshake_cap = Arc::clone(&cap);
    #[allow(clippy::result_large_err)]
    let handshake = move |req: &Request, resp: Response| {
        let mut cap = handshake_cap.lock().unwrap();
        cap.uri = req.uri().to_string();
        cap.headers = req
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
            .collect();
        Ok(resp)
    };
    let mut ws = tokio_tungstenite::accept_hdr_async(stream, handshake)
        .await
        .expect("ws handshake");

    // The client holds audio until `transcript.created`.
    if let Ok(Some(Ok(_))) = tokio::time::timeout(Duration::from_millis(100), ws.next()).await {
        cap.lock().unwrap().early_frames += 1;
    }
    ws.send(Message::Text(
        json!({"type": "transcript.created"}).to_string(),
    ))
    .await
    .unwrap();
    loop {
        match ws.next().await {
            Some(Ok(Message::Binary(bytes))) => cap.lock().unwrap().binary.push(bytes.to_vec()),
            Some(Ok(Message::Text(text))) => {
                let text = text.as_str().to_string();
                cap.lock().unwrap().texts.push(text.clone());
                if text.contains("audio.done") {
                    break;
                }
            }
            Some(Ok(_)) => {}
            _ => return,
        }
    }
    for event in events {
        ws.send(Message::Text(event.to_string())).await.unwrap();
    }
    // Wait for the client to close.
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(Ok(message)) = ws.next().await {
            if message.is_close() {
                break;
            }
        }
    })
    .await;
}

async fn start(events: Vec<Value>) -> (String, Arc<Mutex<Capture>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let cap = Arc::new(Mutex::new(Capture::default()));
    tokio::spawn(serve(listener, Arc::clone(&cap), events));
    (base_url, cap)
}

fn wiremock_stream_options(
    provider_options: Option<Value>,
    format_type: &str,
    rate: Option<u32>,
) -> TranscriptionStreamOptions {
    TranscriptionStreamOptions {
        audio: Box::pin(futures::stream::iter(vec![AudioChunk::Binary(vec![
            1, 2, 3,
        ])])),
        input_audio_format: InputAudioFormat {
            format_type: format_type.to_string(),
            rate,
        },
        provider_options: provider_options.map(wiremock_xai_options),
        abort_signal: None,
        headers: None,
        include_raw_chunks: false,
        timeout: None,
    }
}

struct Run {
    cap: Capture,
    parts: Vec<Result<TranscriptionStreamPart, AiMuxError>>,
    response: Option<TranscriptionResponse>,
}

impl Run {
    /// The parts as JSON; panics on a stream error.
    fn values(&self) -> Vec<Value> {
        self.parts
            .iter()
            .map(|p| serde_json::to_value(p.as_ref().expect("stream part")).unwrap())
            .collect()
    }
}

async fn run_with(
    options: TranscriptionStreamOptions,
    headers: Option<Headers>,
    events: Vec<Value>,
) -> Run {
    let (base_url, cap) = start(events).await;
    let mut result = model(&base_url, headers)
        .do_stream(options)
        .expect("the model has do_stream")
        .await
        .expect("do_stream");
    let response = result.response.take();
    let parts = tokio::time::timeout(Duration::from_secs(10), result.stream.collect::<Vec<_>>())
        .await
        .expect("stream timed out");
    let cap = cap.lock().unwrap().clone();
    Run {
        cap,
        parts,
        response,
    }
}

async fn run(provider_options: Option<Value>, events: Vec<Value>) -> Run {
    run_with(
        wiremock_stream_options(provider_options, "audio/pcm", Some(16000)),
        None,
        events,
    )
    .await
}

fn partial(text: &str, is_final: bool, speech_final: bool, span: Option<(f64, f64)>) -> Value {
    let mut event = json!({
        "type": "transcript.partial", "text": text,
        "is_final": is_final, "speech_final": speech_final
    });
    if let Some((start, duration)) = span {
        event["start"] = json!(start);
        event["duration"] = json!(duration);
    }
    event
}

fn done(text: &str, duration: f64) -> Value {
    json!({"type": "transcript.done", "text": text, "duration": duration})
}

fn kinds(parts: &[Value]) -> Vec<&str> {
    parts.iter().map(|p| p["type"].as_str().unwrap()).collect()
}

fn of_kind(parts: &[Value], kind: &str) -> Vec<Value> {
    parts
        .iter()
        .filter(|p| p["type"] == kind)
        .cloned()
        .collect()
}

fn finish(text: &str, language: Value, duration: Option<f64>) -> Value {
    let mut part = json!({"type": "finish", "text": text, "segments": []});
    if !language.is_null() {
        part["language"] = language;
    }
    if let Some(duration) = duration {
        part["durationInSeconds"] = json!(duration);
    }
    part
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

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doGenerate › should send a multipart request with the audio file
    #[tokio::test]
    async fn should_send_a_multipart_request_with_the_audio_file() {
        let server = MockServer::start().await;
        mock_stt(&server, json_response(wiremock_default_body(), &[])).await;

        generate(&server, &call(None, "audio/wav"), None).await;

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests[0].method.as_str(), "POST");
        assert_eq!(requests[0].url.path(), "/stt");
        let parts = parse_multipart(&requests[0]);
        let file = parts.iter().find(|p| p.name == "file").expect("file field");
        assert_eq!(file.filename.as_deref(), Some("audio.wav"));
        assert_eq!(file.content_type.as_deref(), Some("audio/wav"));
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doGenerate › should map provider options onto xAI request fields
    #[tokio::test]
    async fn should_map_provider_options_onto_xai_request_fields() {
        let server = MockServer::start().await;
        mock_stt(&server, json_response(wiremock_default_body(), &[])).await;

        generate(&server, &call(Some(full_options()), "audio/pcm"), None).await;

        let parts = parse_multipart(&server.received_requests().await.unwrap()[0]);
        let values = |name: &str| -> Vec<&str> {
            parts
                .iter()
                .filter(|p| p.name == name)
                .map(|p| p.value.as_str())
                .collect()
        };
        for (name, expected) in [
            ("audio_format", "pcm"),
            ("sample_rate", "16000"),
            ("language", "en"),
            ("format", "true"),
            ("multichannel", "true"),
            ("channels", "2"),
            ("diarize", "true"),
            ("filler_words", "true"),
        ] {
            assert_eq!(values(name), vec![expected], "field {name}");
        }
        assert_eq!(values("keyterm"), vec!["AI SDK", "Grok"]);
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doGenerate › should append file after all other multipart fields
    #[tokio::test]
    async fn should_append_file_after_all_other_multipart_fields() {
        let server = MockServer::start().await;
        mock_stt(
            &server,
            json_response(json!({"text": "Hello from the AI SDK!"}), &[]),
        )
        .await;

        generate(&server, &call(Some(full_options()), "audio/pcm"), None).await;

        let parts = parse_multipart(&server.received_requests().await.unwrap()[0]);
        let names: Vec<&str> = parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "audio_format",
                "sample_rate",
                "language",
                "format",
                "multichannel",
                "channels",
                "diarize",
                "filler_words",
                "keyterm",
                "keyterm",
                "file"
            ]
        );
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doGenerate › should extract text, segments, language, and duration
    #[tokio::test]
    async fn should_extract_text_segments_language_and_duration() {
        let server = MockServer::start().await;
        mock_stt(&server, json_response(wiremock_default_body(), &[])).await;

        let result = generate(&server, &call(None, "audio/wav"), None).await;

        assert_eq!(result.text, "Hello from the AI SDK!");
        assert_eq!(result.language.as_deref(), Some("en"));
        assert_eq!(result.duration_in_seconds, Some(2.5));
        let segments: Vec<_> = result
            .segments
            .iter()
            .map(|s| (s.text.as_str(), s.start_second, s.end_second))
            .collect();
        assert_eq!(
            segments,
            [("Hello", 0.0, 1.0), ("from the AI SDK!", 1.0, 2.5)]
        );
        assert!(result.warnings.is_empty());
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doGenerate › should handle missing words, duration, and empty language
    #[tokio::test]
    async fn should_handle_missing_words_duration_and_empty_language() {
        let server = MockServer::start().await;
        mock_stt(
            &server,
            json_response(
                json!({"text": "Hello from the AI SDK!", "language": ""}),
                &[],
            ),
        )
        .await;

        let result = generate(&server, &call(None, "audio/wav"), None).await;

        assert_eq!(result.text, "Hello from the AI SDK!");
        assert_eq!(result.language, None);
        assert_eq!(result.duration_in_seconds, None);
        assert!(result.segments.is_empty());
        assert!(result.warnings.is_empty());
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
                return Err(AiMuxError::other(message.clone()));
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
                AiMuxError::InvalidArgument { message, .. }
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

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doStream › should emit one transcript-final per utterance and reconstruct the finish text when transcript.done is empty
    #[tokio::test]
    async fn should_emit_one_final_per_utterance_and_reconstruct_finish_text() {
        let events = vec![
            partial("Hello wor", false, false, Some((0.0, 0.8))),
            partial("Hello world.", true, false, Some((0.0, 1.0))),
            partial("Hello world.", true, true, Some((0.0, 1.0))),
            done("", 1.0),
        ];
        let parts = run(None, events).await.values();

        assert_eq!(
            of_kind(&parts, "transcript-final"),
            vec![json!({
                "type": "transcript-final", "text": "Hello world.", "startSecond": 0.0,
                "endSecond": 1.0
            })]
        );
        // the speech_final:false re-send surfaces as a partial
        assert_eq!(of_kind(&parts, "transcript-partial").len(), 2);
        assert_eq!(
            parts.last().unwrap(),
            &finish("Hello world.", Value::Null, Some(1.0))
        );
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doStream › should treat is_final fragments as partials and use the speech_final text for finish
    #[tokio::test]
    async fn should_treat_is_final_fragments_as_partials_and_use_speech_final_text() {
        let events = vec![
            partial("No", true, false, Some((0.0, 0.5))),
            partial(", I'm not", true, false, Some((0.5, 0.7))),
            partial("No, I'm not.", true, true, Some((0.0, 1.2))),
            done("", 1.2),
        ];
        let parts = run(None, events).await.values();

        assert_eq!(
            kinds(&parts),
            [
                "stream-start",
                "transcript-partial",
                "transcript-partial",
                "transcript-final",
                "finish"
            ]
        );
        assert_eq!(parts.last().unwrap()["text"], "No, I'm not.");
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doStream › should fall back to the latest pending text when no speech_final arrived before transcript.done
    #[tokio::test]
    async fn should_fall_back_to_latest_pending_text_without_speech_final() {
        let events = vec![
            partial("Hello wor", false, false, None),
            partial("Hello world", true, false, None),
            done("", 1.0),
        ];
        let parts = run(None, events).await.values();

        let last = parts.last().unwrap();
        assert_eq!(kinds(&parts).last(), Some(&"finish"));
        assert_eq!(last["text"], "Hello world");
    }

    /// upstream: @ai-sdk/xai src/xai-transcription-model.test.ts › doStream › should warn on unrecognized inputAudioFormat types
    #[tokio::test]
    async fn should_warn_on_unrecognized_input_audio_format_types() {
        let options = wiremock_stream_options(None, "audio/wav", None);
        let run = run_with(options, None, vec![done("Hello", 1.0)]).await;

        match &run.parts[0] {
            Ok(TranscriptionStreamPart::StreamStart { warnings }) => match warnings.as_slice() {
                [Warning::Other { message }] => {
                    assert!(
                        message.contains("Unrecognized inputAudioFormat.type \"audio/wav\""),
                        "{message}"
                    );
                }
                other => panic!("expected one Other warning, got {other:?}"),
            },
            other => panic!("expected StreamStart, got {other:?}"),
        }
    }
}

/// Checks the provider id and the empty model id; specificationVersion is not checked. partial: should expose correct provider and model information; full port in src/xai/tests/
#[test]
fn should_expose_correct_provider_and_model_information() {
    let model = model("https://api.x.ai/v1", None);
    assert_eq!(model.provider(), "xai.transcription");
    assert_eq!(model.model_id(), "");
}

/// Checks authorization, custom headers and the multipart content type; the user agent is checked without its version and the boundary only by prefix. partial: should pass headers and the xAI user agent; full port in src/xai/tests/
#[tokio::test]
async fn should_pass_headers_and_the_xai_user_agent() {
    let server = MockServer::start().await;
    mock_stt(&server, json_response(wiremock_default_body(), &[])).await;

    let mut options = call(None, "audio/wav");
    options.headers = Some(HashMap::from([(
        "Custom-Request-Header".to_string(),
        "request-header-value".to_string(),
    )]));
    let provider_headers = HashMap::from([(
        "Custom-Provider-Header".to_string(),
        Some("provider-header-value".to_string()),
    )]);
    generate(&server, &options, Some(provider_headers)).await;

    let requests = server.received_requests().await.unwrap();
    let header = |name: &str| {
        requests[0]
            .headers
            .get(name)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(header("authorization"), "Bearer test-api-key");
    assert!(header("content-type").starts_with("multipart/form-data; boundary="));
    assert_eq!(header("custom-provider-header"), "provider-header-value");
    assert_eq!(header("custom-request-header"), "request-header-value");
    assert!(header("user-agent").contains("ai-sdk-xai/"));
}

/// Checks the response model id, headers and body; the timestamp is only checked as present. partial: should include response timestamp, model id, and headers; full port in src/xai/tests/
#[tokio::test]
async fn should_include_response_timestamp_model_id_and_headers() {
    let server = MockServer::start().await;
    let headers = [
        ("x-request-id", "test-request-id"),
        ("x-ratelimit-remaining", "123"),
    ];
    mock_stt(&server, json_response(wiremock_default_body(), &headers)).await;

    let result = generate(&server, &call(None, "audio/wav"), None).await;

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id.as_deref(), Some(""));
    let response_headers = result.response.headers.unwrap();
    assert_eq!(response_headers["content-type"], "application/json");
    assert_eq!(response_headers["x-request-id"], "test-request-id");
    assert_eq!(response_headers["x-ratelimit-remaining"], "123");
    assert_eq!(result.response.body, Some(wiremock_default_body()));
}

/// Checks the multichannel-without-channels error and that no socket is opened; the error's argument field is not checked. partial: should require channels when streaming multichannel audio; full port in src/xai/tests/
#[tokio::test]
async fn should_require_channels_when_streaming_multichannel_audio() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());

    let options = wiremock_stream_options(
        Some(json!({"multichannel": true})),
        "audio/pcm",
        Some(16000),
    );
    let error = model(&base_url, None)
        .do_stream(options)
        .expect("the model has do_stream")
        .await
        .unwrap_err();

    assert!(
        matches!(&error, AiMuxError::InvalidArgument { message, .. } if message.contains(
            "providerOptions.xai.channels is required when providerOptions.xai.multichannel is true"
        )),
        "got {error:?}"
    );
    // Nothing connected.
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

/// Checks the streaming URL, handshake, audio frames and parts; the response timestamp is only checked as present. partial: should stream xAI STT over WebSocket; full port in src/xai/tests/
#[tokio::test]
async fn should_stream_xai_stt_over_websocket() {
    let options = json!({
        "language": "en", "diarize": true, "keyterm": ["AI SDK", "Grok"],
        "streaming": {
            "interimResults": true, "endpointing": 500,
            "smartTurn": 0.7, "smartTurnTimeout": 3000
        }
    });
    let events = vec![
        partial("Hel", false, false, Some((0.0, 0.5))),
        partial("Hello", true, true, Some((0.0, 1.0))),
        done("Hello", 1.0),
    ];
    let run = run(Some(options), events).await;

    assert_eq!(
        run.cap.uri,
        "/stt?sample_rate=16000&encoding=pcm&language=en&diarize=true&interim_results=true&endpointing=500&smart_turn=0.7&smart_turn_timeout=3000&keyterm=AI+SDK&keyterm=Grok"
    );
    assert_eq!(run.cap.headers["authorization"], "Bearer test-api-key");
    // Audio waits for `transcript.created`, then the frames and `audio.done` follow.
    assert_eq!(run.cap.early_frames, 0);
    assert_eq!(run.cap.binary, vec![vec![1u8, 2, 3]]);
    assert_eq!(run.cap.texts.len(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(&run.cap.texts[0]).unwrap(),
        json!({"type": "audio.done"})
    );

    assert_eq!(
        run.values(),
        vec![
            json!({"type": "stream-start", "warnings": []}),
            json!({
                "type": "transcript-partial", "text": "Hel", "startSecond": 0.0,
                "durationInSeconds": 0.5
            }),
            json!({
                "type": "transcript-final", "text": "Hello", "startSecond": 0.0,
                "endSecond": 1.0
            }),
            finish("Hello", json!("en"), Some(1.0)),
        ]
    );
    let response = run.response.unwrap();
    assert_eq!(response.model_id.as_deref(), Some(""));
    assert!(response.timestamp.is_some());
}

/// Checks that a provider header set to None is left off the WebSocket handshake (analogue: per-call headers cannot hold None). partial: should strip undefined header values before the WebSocket constructor; full port in src/xai/tests/
#[tokio::test]
async fn should_not_send_removed_header_values_on_the_handshake() {
    let headers = HashMap::from([
        (
            "Custom-Header".to_string(),
            Some("custom-value".to_string()),
        ),
        ("X-Unset".to_string(), None),
    ]);
    let options = wiremock_stream_options(None, "audio/pcm", Some(16000));
    let run = run_with(options, Some(headers), vec![done("Hello", 1.0)]).await;

    assert_eq!(run.cap.headers["authorization"], "Bearer test-api-key");
    assert_eq!(run.cap.headers["custom-header"], "custom-value");
    assert!(!run.cap.headers.contains_key("x-unset"));
}

/// Checks that finalized utterances are joined into the finish text; the finish part type is not checked. partial: should join finalized utterances per channel when transcript.done is empty; full port in src/xai/tests/
#[tokio::test]
async fn should_join_finalized_utterances_when_transcript_done_is_empty() {
    let events = vec![
        partial("First utterance.", true, true, Some((0.0, 1.0))),
        partial("Second utterance.", true, true, Some((1.0, 1.0))),
        done("", 2.0),
    ];
    let parts = run(None, events).await.values();

    assert_eq!(
        parts.last().unwrap()["text"],
        "First utterance. Second utterance."
    );
}

/// Checks that an error event fails the stream with the server message; the WebSocket close is not checked. partial: should error the stream with the server message on error events; full port in src/xai/tests/
#[tokio::test]
async fn should_error_the_stream_with_the_server_message_on_error_events() {
    let events = vec![json!({"type": "error", "message": "invalid sample_rate"})];
    let run = run(None, events).await;

    assert!(matches!(
        run.parts[0],
        Ok(TranscriptionStreamPart::StreamStart { .. })
    ));
    assert_eq!(run.parts.len(), 2, "parts: {:?}", run.parts);
    assert!(
        matches!(&run.parts[1], Err(AiMuxError::Other { message, .. }) if message == "invalid sample_rate"),
        "got {:?}",
        run.parts[1]
    );
}

/// Only `transcribe` downloads an audio URL (layer 5's `AudioInput::Url`);
/// the model itself refuses one before any request, as every transcription
/// model does.
#[tokio::test]
async fn url_audio_is_refused_before_any_request() {
    let server = MockServer::start().await;
    let mut options = call(None, "audio/wav");
    options.audio = AudioInput::Url("https://cdn.test/a.wav".into());
    let error = model(&server.uri(), None)
        .do_generate(&options)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), AudioInput::url_error().to_string());
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// A refused connection errors the stream: upstream connects inside the
/// stream, so `doStream` resolves and the stream carries the failure.
#[tokio::test]
async fn a_connect_failure_errors_the_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    drop(listener);

    let result = model(&base_url, None)
        .do_stream(wiremock_stream_options(None, "audio/pcm", Some(16000)))
        .expect("the model has do_stream")
        .await
        .expect("do_stream resolves; the connect failure arrives on the stream");
    let parts = tokio::time::timeout(Duration::from_secs(10), result.stream.collect::<Vec<_>>())
        .await
        .expect("stream timed out");

    assert_eq!(parts.len(), 1, "{parts:?}");
    assert!(parts[0].is_err(), "{parts:?}");
}

/// Dropping the stream closes the socket: upstream's `cancel` calls
/// `connection.close()`.
#[tokio::test]
async fn dropping_the_stream_closes_the_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        ws.send(Message::Text(
            json!({"type": "transcript.created"}).to_string(),
        ))
        .await
        .unwrap();
        let mut closed = false;
        while let Some(Ok(message)) = ws.next().await {
            if message.is_close() {
                closed = true;
                break;
            }
        }
        let _ = closed_tx.send(closed);
    });

    let mut options = wiremock_stream_options(None, "audio/pcm", Some(16000));
    options.audio = Box::pin(futures::stream::pending());
    let mut result = model(&base_url, None)
        .do_stream(options)
        .expect("the model has do_stream")
        .await
        .expect("do_stream");
    let first = result.stream.next().await;
    assert!(
        matches!(first, Some(Ok(TranscriptionStreamPart::StreamStart { .. }))),
        "{first:?}"
    );
    drop(result);

    let closed = tokio::time::timeout(Duration::from_secs(5), closed_rx)
        .await
        .expect("the server saw the socket end")
        .unwrap();
    assert!(
        closed,
        "no close frame was sent when the stream was dropped"
    );
}

/// A stream dropped before its first poll still closes the socket: the
/// connect is awaited in `do_stream`, so the socket is open by then.
#[tokio::test]
async fn dropping_an_unpolled_stream_closes_the_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut closed = false;
        while let Some(Ok(message)) = ws.next().await {
            if message.is_close() {
                closed = true;
                break;
            }
        }
        let _ = closed_tx.send(closed);
    });

    let mut options = wiremock_stream_options(None, "audio/pcm", Some(16000));
    options.audio = Box::pin(futures::stream::pending());
    let result = model(&base_url, None)
        .do_stream(options)
        .expect("the model has do_stream")
        .await
        .expect("do_stream");
    drop(result);

    let closed = tokio::time::timeout(Duration::from_secs(5), closed_rx)
        .await
        .expect("the server saw the socket end")
        .unwrap();
    assert!(
        closed,
        "no close frame was sent when the unpolled stream was dropped"
    );
}
