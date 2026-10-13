//! Offline replay of the OpenAI realtime transcription WebSocket path
//! (RFC-0028 §3.2), driven through the connector that
//! `OpenAITranscriptionModel::with_web_socket` installs.
//!
//! `OpenAIProviderSettings::web_socket` carries a custom [`WsConnector`] that
//! never touches the network: it pairs the client with an in-process server
//! over an in-memory duplex pipe. The server records every frame the client
//! sends and replays the server events in
//! `fixtures/openai_realtime_transcription.json` once the audio is committed.
#![cfg(feature = "realtime")]

use std::sync::{Arc, Mutex};

use aimux_core::error::AiMuxError;
use aimux_core::transcription_model::{
    AudioChunk, InputAudioFormat, TranscriptionModel, TranscriptionStreamOptions,
    TranscriptionStreamPart,
};
use aimux_provider_utils::ws::{WebSocketRequest, WsConnection, WsConnector};
use aimux_providers::{OpenAIProviderSettings, create_openai};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;

/// What the in-process server saw.
#[derive(Default)]
struct Seen {
    /// URL and headers of every connect request, in order.
    connects: Vec<(String, Vec<(String, String)>)>,
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
        self.seen
            .lock()
            .unwrap()
            .connects
            .push((request.url.clone(), request.headers.clone()));
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
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/openai_realtime_transcription.json"))
            .expect("fixture is JSON");
    fixture["events"].as_array().expect("events array").clone()
}

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
    let result = model.do_stream(options).await.expect("stream opens");
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
    let (url, headers) = &seen.connects[0];
    assert_eq!(url, "wss://replay.invalid/v1/realtime?intent=transcription");
    assert!(headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("authorization") && value == "Bearer sk-replay-test"
    }));

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
