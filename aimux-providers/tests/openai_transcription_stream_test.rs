//! Realtime transcription streaming tests (RFC-0028 Phase 1).
//!
//! Each test spins up a local WebSocket server (tokio-tungstenite `accept_async`),
//! plays the OpenAI realtime endpoint's role (consume `session.update` /
//! `input_audio_buffer.append` / `commit`, emit `transcription.delta` /
//! `completed` events), and asserts on the `TranscriptionStreamPart` sequence
//! produced by `do_stream`.
//!
//! These replace the TS mock-WebSocket tests that
//! `openai_transcription_test.rs` omitted ("not practical to translate" —
//! they became practical with RFC-0028's WebSocket infrastructure).

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use aimux_core::AbortSignal;
use aimux_core::error::AiMuxError;
use aimux_core::transcription_model::{
    AudioChunk, InputAudioFormat, TranscriptionModel, TranscriptionStreamOptions,
    TranscriptionStreamPart,
};
use aimux_providers::{OpenAIConfig, OpenAIProvider};

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
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
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

fn realtime_model(base_url: &str) -> aimux_providers::OpenAITranscriptionModel {
    let config = OpenAIConfig::new("test-api-key").with_base_url(base_url.to_string());
    let provider = OpenAIProvider::new(config);
    provider.transcription("gpt-realtime-whisper")
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
    let result = model.do_stream(options).await.unwrap();
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
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
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
    let result = model.do_stream(options).await.unwrap();
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
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _ = ws.next().await; // session.update
        // Hold the connection open (drain client messages without acting) so
        // the abort — not a server-side disconnect — is what ends the session.
        while let Some(_msg) = ws.next().await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });

    let model = realtime_model(&format!("http://127.0.0.1:{port}"));
    let options = stream_options(vec![AudioChunk::Binary(vec![1])], Some(abort_clone));

    let result = model.do_stream(options).await.unwrap();
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
    let config = OpenAIConfig::new("k").with_base_url("http://127.0.0.1:1");
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("whisper-1");
    let err = model
        .do_stream(stream_options(vec![], None))
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
    let config = OpenAIConfig::new("k").with_base_url("http://127.0.0.1:1".to_string());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("gpt-realtime-whisper");

    let err = model
        .do_stream(stream_options(vec![AudioChunk::Binary(vec![1])], None))
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

    let config = OpenAIConfig::new("k").with_base_url(format!("http://127.0.0.1:{port}"));
    let provider = OpenAIProvider::new(config);
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
    let err = model.do_stream(options).await.unwrap_err();
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
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        // Read session.update, then send NOTHING but KEEP the socket alive
        // (drain client appends) well past the client's 500ms first-chunk
        // deadline — the timeout must fire before any disconnect could.
        let _ = ws.next().await;
        while let Ok(Some(_)) = tokio::time::timeout(Duration::from_secs(2), ws.next()).await {}
    });

    let config = OpenAIConfig::new("k").with_base_url(format!("http://127.0.0.1:{port}"));
    let provider = OpenAIProvider::new(config);
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
    let result = model.do_stream(options).await.unwrap();
    let parts = collect(result).await;
    let timed_out = parts
        .iter()
        .any(|p| matches!(p, Err(AiMuxError::Timeout(_))));
    assert!(
        timed_out,
        "expected a Timeout error against a silent server; got: {parts:?}"
    );
}

/// The server emits a realtime `error` event — the provider message must
/// propagate verbatim and readable (not wrapped in transport noise), and the
/// session must terminate there (#118).
#[tokio::test]
async fn stream_server_error_event_propagates() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
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
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
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
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
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

    let config = OpenAIConfig::new("k").with_base_url(format!("http://127.0.0.1:{port}"));
    let provider = OpenAIProvider::new(config);
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
    let result = model.do_stream(options).await.unwrap();
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
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
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

    let config = OpenAIConfig::new("k").with_base_url(format!("http://127.0.0.1:{port}"));
    let provider = OpenAIProvider::new(config);
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
    let result = model.do_stream(options).await.unwrap();
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
