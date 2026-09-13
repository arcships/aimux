//! ElevenLabs realtime streaming tests (RFC-0034 §3, P2).
//!
//! Each test runs a local WebSocket server playing the
//! `wss://…/v1/speech-to-text/realtime` role: handshake (capturing the
//! request URI + headers), `session_started`, audio chunks until the
//! committing `input_audio_chunk`, then `partial_transcript` /
//! `committed_transcript` events.
//!
//! Wire shape is pinned field-by-field against the public API reference
//! (2026-09). Live-API smoke pending (no key at implementation time) — same
//! posture as RFC-0028 D4.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

use aimux_core::AbortSignal;
use aimux_core::error::AiMuxError;
use aimux_core::options::TimeoutConfiguration;
use aimux_core::transcription_model::{
    AudioChunk, InputAudioFormat, TranscriptionModel, TranscriptionStreamOptions,
    TranscriptionStreamPart,
};
use aimux_providers::{ElevenLabsConfig, ElevenLabsProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Captured {
    uri: Arc<Mutex<Option<String>>>,
    api_key_header: Arc<Mutex<Option<String>>>,
    /// All `input_audio_chunk` messages as (audio_base_64, commit, sample_rate).
    chunks: Arc<Mutex<Vec<(String, bool, u64)>>>,
}

impl Captured {
    fn new() -> Self {
        Self {
            uri: Arc::new(Mutex::new(None)),
            api_key_header: Arc::new(Mutex::new(None)),
            chunks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn uri(&self) -> String {
        self.uri.lock().unwrap().clone().unwrap_or_default()
    }

    fn chunks(&self) -> Vec<(String, bool, u64)> {
        self.chunks.lock().unwrap().clone()
    }
}

/// Accept a connection, capture the handshake URI + `xi-api-key` header,
/// read `input_audio_chunk` frames until the committing one, then hand the
/// socket to the scripted `after_audio` behavior.
async fn serve(
    stream: TcpStream,
    captured: Captured,
    after_audio: impl FnOnce(
        &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    ) -> futures::future::BoxFuture<'_, ()>
    + Send
    + 'static,
) {
    let uri = Arc::clone(&captured.uri);
    let api_key = Arc::clone(&captured.api_key_header);
    #[allow(clippy::result_large_err)]
    let handshake = move |req: &Request, resp: Response| {
        *uri.lock().unwrap() = Some(req.uri().to_string());
        *api_key.lock().unwrap() = req
            .headers()
            .get("xi-api-key")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        Ok(resp)
    };
    let ws = tokio_tungstenite::accept_hdr_async(stream, handshake)
        .await
        .expect("ws handshake");
    let mut ws = ws;

    // session_started first — the client holds audio until this arrives.
    ws.send(Message::Text(
        r#"{"message_type":"session_started","session_id":"s-1","sample_rate":24000}"#.into(),
    ))
    .await
    .unwrap();

    // Audio frames until the committing one.
    loop {
        let Some(Ok(Message::Text(text))) = ws.next().await else {
            return;
        };
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        if v["message_type"] != "input_audio_chunk" {
            panic!("expected input_audio_chunk, got {text}");
        }
        let entry = (
            v["audio_base_64"].as_str().unwrap_or("").to_string(),
            v["commit"].as_bool().unwrap_or(false),
            v["sample_rate"].as_u64().unwrap_or(0),
        );
        let committed = entry.1;
        captured.chunks.lock().unwrap().push(entry);
        if committed {
            break;
        }
    }

    after_audio(&mut ws).await;
}

/// Start the server; returns (base_url, captured).
async fn start(
    after_audio: impl FnOnce(
        &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    ) -> futures::future::BoxFuture<'_, ()>
    + Send
    + 'static,
) -> (String, Captured) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured = Captured::new();
    let captured_for_task = captured.clone();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        serve(stream, captured_for_task, after_audio).await;
    });
    (format!("http://127.0.0.1:{port}"), captured)
}

fn realtime_model(base_url: &str, model_id: &str) -> aimux_providers::ElevenLabsTranscriptionModel {
    let config = ElevenLabsConfig {
        api_key: "test-api-key".to_string(),
        base_url: base_url.to_string(),
        headers: None,
    };
    ElevenLabsProvider::new(config).transcription(model_id)
}

fn stream_options(
    chunks: Vec<AudioChunk>,
    abort: Option<AbortSignal>,
) -> TranscriptionStreamOptions {
    TranscriptionStreamOptions {
        audio: Box::pin(futures::stream::iter(chunks)),
        input_audio_format: InputAudioFormat {
            format_type: "audio/pcm".to_string(),
            rate: Some(24_000),
        },
        provider_options: None,
        abort_signal: abort,
        headers: None,
        include_raw_chunks: false,
        timeout: None,
    }
}

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

/// Happy path: URL/config on the query string, audio as base64 JSON with the
/// commit flag on the LAST chunk, partial/final/finish parts, close(1000).
#[tokio::test]
async fn stream_realtime_happy_path() {
    let (base_url, captured) = start(|ws| {
        Box::pin(async move {
            ws.send(Message::Text(
                r#"{"message_type":"partial_transcript","text":"Hel"}"#.into(),
            ))
            .await
            .unwrap();
            ws.send(Message::Text(
                r#"{"message_type":"committed_transcript","text":"Hello world"}"#.into(),
            ))
            .await
            .unwrap();
            match ws.next().await {
                Some(Ok(Message::Close(frame))) => {
                    let code = frame.map(|f| u16::from(f.code));
                    assert_eq!(code, Some(1000), "client must close(1000) after the final");
                }
                other => panic!("expected close frame, got {other:?}"),
            }
        })
    })
    .await;

    let model = realtime_model(&base_url, "scribe_v2_realtime");
    let result = model
        .do_stream(stream_options(
            vec![
                AudioChunk::Binary(vec![1, 2, 3]),
                AudioChunk::Binary(vec![4, 5, 6]),
            ],
            None,
        ))
        .await
        .expect("do_stream should connect");
    let parts = collect(result).await;

    // Config travels on the URL, not a session message (wire fact).
    let uri = captured.uri();
    assert!(
        uri.starts_with("/v1/speech-to-text/realtime?"),
        "path+query expected, got {uri}"
    );
    assert!(uri.contains("model_id=scribe_v2_realtime"), "in {uri}");
    assert!(uri.contains("audio_format=pcm_24000"), "in {uri}");
    assert!(uri.contains("commit_strategy=manual"), "in {uri}");
    assert_eq!(
        captured.api_key_header.lock().unwrap().as_deref(),
        Some("test-api-key"),
        "xi-api-key header must ride the WS handshake"
    );

    // Hold-one-chunk pipeline: exactly two frames, commit on the last.
    assert_eq!(
        captured.chunks(),
        vec![
            ("AQID".to_string(), false, 24_000),
            ("BAUG".to_string(), true, 24_000),
        ],
        "commit flag must ride the last real chunk"
    );

    assert_eq!(parts.len(), 4, "parts: {parts:?}");
    assert!(matches!(
        &parts[0],
        Ok(TranscriptionStreamPart::StreamStart { .. })
    ));
    match &parts[1] {
        Ok(TranscriptionStreamPart::TranscriptPartial { text, .. }) => assert_eq!(text, "Hel"),
        other => panic!("expected partial, got {other:?}"),
    }
    match &parts[2] {
        Ok(TranscriptionStreamPart::TranscriptFinal { text, .. }) => {
            assert_eq!(text, "Hello world")
        }
        other => panic!("expected final, got {other:?}"),
    }
    match &parts[3] {
        Ok(TranscriptionStreamPart::Finish { text, segments, .. }) => {
            assert_eq!(text, "Hello world");
            assert!(segments.is_empty());
        }
        other => panic!("expected finish, got {other:?}"),
    }
}

/// providerOptions: languageCode lands on the query; include_timestamps
/// switches the committed event to the words-carrying shape, which fills
/// segments (spacing entries dropped) and the finish duration.
#[tokio::test]
async fn stream_language_and_timestamps() {
    let (base_url, captured) = start(|ws| {
        Box::pin(async move {
            ws.send(Message::Text(
                r#"{"message_type":"committed_transcript_with_timestamps","text":"Hello world","language_code":"fr","words":[
                    {"text":"Hello","start":0.0,"end":0.5,"type":"word"},
                    {"text":" ","start":0.5,"end":0.5,"type":"spacing"},
                    {"text":"world","start":0.5,"end":1.0,"type":"word"}
                ]}"#
                    .replace('\n', ""),
            ))
            .await
            .unwrap();
            let _ = ws.next().await; // close
        })
    })
    .await;

    let model = realtime_model(&base_url, "scribe_v2_realtime");
    let mut options = stream_options(vec![AudioChunk::Binary(vec![1])], None);
    let mut provider_options = std::collections::HashMap::new();
    provider_options.insert(
        "elevenlabs".to_string(),
        serde_json::json!({ "languageCode": "fr", "includeTimestamps": true }),
    );
    options.provider_options = Some(provider_options);
    let result = model.do_stream(options).await.unwrap();
    let parts = collect(result).await;

    let uri = captured.uri();
    assert!(uri.contains("language_code=fr"), "in {uri}");
    assert!(uri.contains("include_timestamps=true"), "in {uri}");

    match &parts[parts.len() - 2] {
        Ok(TranscriptionStreamPart::TranscriptFinal {
            text,
            start_second,
            end_second,
            ..
        }) => {
            assert_eq!(text, "Hello world");
            assert_eq!(*start_second, Some(0.0));
            assert_eq!(*end_second, Some(1.0));
        }
        other => panic!("expected final with timestamps, got {other:?}"),
    }
    match parts.last().unwrap() {
        Ok(TranscriptionStreamPart::Finish {
            text,
            segments,
            language,
            duration_in_seconds,
            ..
        }) => {
            assert_eq!(text, "Hello world");
            assert_eq!(language.as_deref(), Some("fr"));
            assert_eq!(*duration_in_seconds, Some(1.0));
            assert_eq!(segments.len(), 2, "spacing entry dropped: {segments:?}");
            assert_eq!(segments[0].text, "Hello");
            assert_eq!(segments[1].text, "world");
        }
        other => panic!("expected finish, got {other:?}"),
    }
}

/// An empty audio stream commits via a single empty chunk.
#[tokio::test]
async fn stream_empty_audio_commits_empty_chunk() {
    let (base_url, captured) = start(|ws| {
        Box::pin(async move {
            ws.send(Message::Text(
                r#"{"message_type":"committed_transcript","text":""}"#.into(),
            ))
            .await
            .unwrap();
            let _ = ws.next().await; // close
        })
    })
    .await;

    let model = realtime_model(&base_url, "scribe_v2_realtime");
    let result = model
        .do_stream(stream_options(vec![], None))
        .await
        .expect("connect");
    let parts = collect(result).await;

    assert_eq!(
        captured.chunks(),
        vec![(String::new(), true, 24_000)],
        "no audio → one empty committing chunk"
    );
    assert!(matches!(
        parts.last(),
        Some(Ok(TranscriptionStreamPart::Finish { .. }))
    ));
}

/// Error classification (RFC-0034 §3.2): three transient names retry, the
/// rest are terminal verdicts.
#[tokio::test]
async fn stream_error_classification() {
    for (message_type, expect_retryable) in [
        ("auth_error", false),
        ("rate_limited", true),
        ("queue_overflow", true),
        ("quota_exceeded", false),
    ] {
        let (base_url, _captured) = start(move |ws| {
            let message_type = message_type.to_string();
            Box::pin(async move {
                ws.send(Message::Text(
                    serde_json::json!({
                        "message_type": message_type,
                        "error": "boom",
                    })
                    .to_string(),
                ))
                .await
                .unwrap();
            })
        })
        .await;

        let model = realtime_model(&base_url, "scribe_v2_realtime");
        let result = model
            .do_stream(stream_options(vec![AudioChunk::Binary(vec![1])], None))
            .await
            .unwrap();
        let parts = collect(result).await;
        // StreamStart precedes the error (session_started arrives first).
        let error_part = parts.iter().find_map(|p| match p {
            Err(e) => Some(e.clone()),
            Ok(_) => None,
        });
        match error_part {
            Some(AiMuxError::ApiCall(api_call)) => {
                assert_eq!(
                    api_call.is_retryable, expect_retryable,
                    "{message_type} retryability"
                );
                assert!(
                    api_call
                        .response_body
                        .as_deref()
                        .unwrap_or("")
                        .contains(message_type),
                    "raw event preserved in response_body"
                );
            }
            Some(other) => panic!("{message_type}: expected ApiCall, got {other:?}"),
            None => panic!("{message_type}: stream ended without an error part: {parts:?}"),
        }
    }
}

/// A server that goes silent after the commit trips the settle window
/// (chunk_ms) → empty Finish, stream terminates (RFC-0034 §3.3.3).
#[tokio::test]
async fn stream_silent_server_finishes_via_settle_window() {
    let (base_url, _captured) = start(|ws| {
        Box::pin(async move {
            // Silence: hold the socket open, never answer the commit.
            let _ = ws.next().await;
        })
    })
    .await;

    let model = realtime_model(&base_url, "scribe_v2_realtime");
    let mut options = stream_options(vec![AudioChunk::Binary(vec![1])], None);
    options.timeout = Some(TimeoutConfiguration {
        first_chunk_ms: Some(5_000),
        chunk_ms: Some(100),
        step_ms: None,
        total_ms: Some(5_000),
    });
    let result = model.do_stream(options).await.unwrap();
    let parts = collect(result).await;
    match parts.last() {
        Some(Ok(TranscriptionStreamPart::Finish { text, .. })) => {
            assert_eq!(text, "", "silent server → empty finish, not a hang");
        }
        other => panic!("expected empty finish, got {other:?}"),
    }
}

/// Abort mid-session surfaces `AiMuxError::Aborted`.
#[tokio::test]
async fn stream_abort_mid_session() {
    let (base_url, _captured) = start(|ws| {
        Box::pin(async move {
            // Drain client messages without acting; the abort is what ends
            // the session.
            while let Some(_msg) = ws.next().await {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
    })
    .await;

    let abort = AbortSignal::new();
    let abort_clone = abort.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        abort_clone.abort();
    });

    let model = realtime_model(&base_url, "scribe_v2_realtime");
    let result = model
        .do_stream(stream_options(
            vec![AudioChunk::Binary(vec![1])],
            Some(abort),
        ))
        .await
        .unwrap();
    let parts = collect(result).await;
    assert!(
        parts
            .iter()
            .any(|p| matches!(p, Err(AiMuxError::Aborted(_)))),
        "expected Aborted in {parts:?}"
    );
}

/// Model gating: realtime IDs reject do_generate; batch IDs reject do_stream
/// (no server needed — nothing connects).
#[tokio::test]
async fn stream_model_gating_is_symmetric() {
    let realtime = realtime_model("http://127.0.0.1:1", "scribe_v2_realtime");
    let error = realtime
        .do_generate(
            &aimux_core::transcription_model::TranscriptionCallOptions::new(
                aimux_core::transcription_model::AudioInput::Binary(vec![1]),
                "audio/pcm",
            ),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AiMuxError::UnsupportedFunctionality(_)));

    let batch = realtime_model("http://127.0.0.1:1", "scribe_v1");
    let error = batch
        .do_stream(stream_options(vec![], None))
        .await
        .unwrap_err();
    assert!(
        matches!(error, AiMuxError::UnsupportedFunctionality(ref e) if e.to_string().contains("scribe_v1")),
        "got {error:?}"
    );
}

/// The realtime endpoint accepts pcm/ulaw only; anything else fails fast
/// without connecting.
#[tokio::test]
async fn stream_rejects_non_pcm_formats() {
    let model = realtime_model("http://127.0.0.1:1", "scribe_v2_realtime");
    let mut options = stream_options(vec![], None);
    options.input_audio_format = InputAudioFormat {
        format_type: "audio/mpeg".to_string(),
        rate: None,
    };
    let error = model.do_stream(options).await.unwrap_err();
    assert!(
        matches!(error, AiMuxError::UnsupportedFunctionality(ref e) if e.to_string().contains("audio/mpeg")),
        "got {error:?}"
    );
}
