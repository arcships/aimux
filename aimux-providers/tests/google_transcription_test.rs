//! Rust translation of the Google transcription model tests.
//!
//! Source: `reference/aisdk-pinned/google/src/transcription/google-transcription-model.test.ts`.
//! The live (`doStream`) cases run against a local WebSocket server that plays
//! the Gemini Live API; the upstream `_internal.finishGraceMs` and
//! `_internal.currentDate` hooks have no Rust counterpart.

use std::sync::{Arc, Mutex};

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;
use aimux_core::shared::SharedProviderOptions;
use aimux_core::transcription_model::{
    AudioChunk, AudioInput, InputAudioFormat, TranscriptionCallOptions, TranscriptionModel,
    TranscriptionStreamOptions, TranscriptionStreamPart,
};
use aimux_providers::{GoogleProviderSettings, create_google};

fn model(base_url: &str, model_id: &str) -> Arc<dyn TranscriptionModel> {
    create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(base_url.to_string()),
        ..Default::default()
    })
    .unwrap()
    .transcription_model(model_id)
    .unwrap()
    .unwrap()
}

fn google_options(value: Value) -> Option<SharedProviderOptions> {
    let Value::Object(map) = value else {
        panic!("options must be an object")
    };
    Some([("google".to_string(), map)].into_iter().collect())
}

// ── doGenerate ──────────────────────────────────────────────────────────────

async fn interactions_server(body: Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    server
}

fn hello_world() -> Value {
    json!({
        "id": "interactions/test",
        "status": "completed",
        "steps": [{"type": "model_output", "content": [{"type": "text", "text": "Hello world."}]}],
        "usage": {"total_tokens": 10, "total_input_tokens": 10, "total_output_tokens": 0},
    })
}

fn wav_call(provider_options: Option<SharedProviderOptions>) -> TranscriptionCallOptions {
    let mut call = TranscriptionCallOptions::new(AudioInput::Binary(vec![1, 2, 3, 4]), "audio/wav");
    call.provider_options = provider_options;
    call
}

/// TS: transcribes audio via the Interactions API with transcription_config
#[tokio::test]
async fn transcribes_audio_via_the_interactions_api_with_transcription_config() {
    let server = interactions_server(hello_world()).await;
    let result = model(&server.uri(), "gemini-3.5-transcribe")
        .do_generate(&wav_call(google_options(json!({
            "customVocabulary": ["Gemini", "Kubernetes"],
            "languageCodes": ["es-ES"],
            "mode": "SMART",
        }))))
        .await
        .unwrap();
    assert_eq!(result.text, "Hello world.");
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0].body_json::<Value>().unwrap(),
        json!({
            "model": "gemini-3.5-transcribe",
            "input": [{"type": "audio", "data": "AQIDBA==", "mime_type": "audio/wav"}],
            "generation_config": {"transcription_config": {
                "language_codes": ["es-ES"],
                "custom_vocabulary": ["Gemini", "Kubernetes"],
                "mode": {"type": "smart"},
            }},
        })
    );
    assert_eq!(
        requests[0].headers.get("x-goog-api-key").unwrap(),
        "test-api-key"
    );
    assert_eq!(
        serde_json::to_value(result.provider_metadata.unwrap()).unwrap(),
        json!({"google": {"usage": {"total_tokens": 10, "total_input_tokens": 10, "total_output_tokens": 0}}})
    );
}

/// TS: maps diarization and word timestamps into the mode object
#[tokio::test]
async fn maps_diarization_and_word_timestamps_into_the_mode_object() {
    let server = interactions_server(hello_world()).await;
    model(&server.uri(), "gemini-3.5-transcribe")
        .do_generate(&wav_call(google_options(
            json!({"diarization": true, "wordTimestamp": true}),
        )))
        .await
        .unwrap();
    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(
        body["generation_config"]["transcription_config"],
        json!({"mode": {
            "type": "verbatim",
            "diarization_mode": "speaker",
            "timestamp_granularities": ["word"],
        }})
    );
}

/// TS: omits generation_config when no transcription options are set
#[tokio::test]
async fn omits_generation_config_when_no_transcription_options_are_set() {
    let server = interactions_server(hello_world()).await;
    model(&server.uri(), "gemini-3.5-transcribe")
        .do_generate(&wav_call(google_options(json!({}))))
        .await
        .unwrap();
    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert!(body.get("generation_config").is_none());
}

/// TS: extracts word segments from word_info annotations
#[tokio::test]
async fn extracts_word_segments_from_word_info_annotations() {
    let word = |text: &str, start: &str, end: &str| json!({"type": "word_info", "text": text, "speaker": "spk:0", "start_offset": start, "end_offset": end});
    let server = interactions_server(json!({
        "id": "interactions/test",
        "status": "completed",
        "steps": [{"type": "model_output", "content": [{
            "type": "text",
            "text": "The quick brown fox.",
            "annotations": [
                word("The", "0.100s", "0.100s"),
                word("quick", "0.100s", "0.400s"),
                word("brown", "0.400s", "0.700s"),
                word("fox.", "0.700s", "1s"),
            ],
        }]}],
        "usage": {"total_input_tokens": 64},
    }))
    .await;
    let result = model(&server.uri(), "gemini-3.5-transcribe")
        .do_generate(&wav_call(google_options(json!({}))))
        .await
        .unwrap();
    assert_eq!(result.text, "The quick brown fox.");
    let segments: Vec<(String, f64, f64)> = result
        .segments
        .into_iter()
        .map(|s| (s.text, s.start_second, s.end_second))
        .collect();
    assert_eq!(
        segments,
        [
            ("The".to_string(), 0.1, 0.1),
            ("quick".to_string(), 0.1, 0.4),
            ("brown".to_string(), 0.4, 0.7),
            ("fox.".to_string(), 0.7, 1.0),
        ]
    );
}

/// TS: rejects unary transcription on live model ids
#[tokio::test]
async fn rejects_unary_transcription_on_live_model_ids() {
    let error = model("http://127.0.0.1:1", "gemini-3.5-transcribe-live")
        .do_generate(&wav_call(google_options(json!({}))))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("only supports streaming transcription")
    );
}

// ── doStream ────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Captured {
    uri: String,
    received: Vec<Value>,
}

/// What the local Live API server does once the setup message arrived.
struct Script {
    /// Messages sent after the audio (and `audioStreamEnd`) was received.
    messages: Vec<Value>,
    /// Wait for the audio chunk and `audioStreamEnd` before sending `messages`.
    expect_audio: bool,
    /// Close with this code (and reason) after `messages`.
    close: Option<(u16, &'static str)>,
}

async fn live_server(script: Script) -> (String, Arc<Mutex<Captured>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1beta", listener.local_addr().unwrap());
    let captured = Arc::new(Mutex::new(Captured::default()));
    let shared = Arc::clone(&captured);
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let uri = Arc::clone(&shared);
        #[allow(clippy::result_large_err)]
        let handshake = move |request: &Request, response: Response| {
            uri.lock().unwrap().uri = request.uri().to_string();
            Ok(response)
        };
        let mut ws = tokio_tungstenite::accept_hdr_async(stream, handshake)
            .await
            .unwrap();
        let read = |ws_message: Message| {
            if let Message::Text(text) = ws_message {
                shared
                    .lock()
                    .unwrap()
                    .received
                    .push(serde_json::from_str(&text).unwrap());
                return Some(serde_json::from_str::<Value>(&text).unwrap());
            }
            None
        };
        // Setup first; audio is gated on the `setupComplete` acknowledgement.
        read(ws.next().await.unwrap().unwrap());
        ws.send(Message::Text(json!({"setupComplete": {}}).to_string()))
            .await
            .unwrap();
        if script.expect_audio {
            loop {
                let Some(message) = read(ws.next().await.unwrap().unwrap()) else {
                    continue;
                };
                if message["realtimeInput"]["audioStreamEnd"] == true {
                    break;
                }
            }
        }
        for message in script.messages {
            ws.send(Message::Text(message.to_string())).await.unwrap();
        }
        if let Some((code, reason)) = script.close {
            let _ = ws
                .send(Message::Close(Some(CloseFrame {
                    code: CloseCode::from(code),
                    reason: reason.into(),
                })))
                .await;
        }
        while let Some(Ok(_)) = ws.next().await {}
    });
    (base_url, captured)
}

fn stream_options(
    audio: futures::stream::BoxStream<'static, AudioChunk>,
    rate: Option<u32>,
    provider_options: Option<SharedProviderOptions>,
    include_raw_chunks: bool,
) -> TranscriptionStreamOptions {
    TranscriptionStreamOptions {
        audio,
        input_audio_format: InputAudioFormat {
            format_type: "audio/pcm".into(),
            rate,
        },
        provider_options,
        abort_signal: None,
        headers: None,
        include_raw_chunks,
        timeout: None,
    }
}

fn one_chunk() -> futures::stream::BoxStream<'static, AudioChunk> {
    futures::stream::iter(vec![AudioChunk::Binary(vec![1, 2, 3])]).boxed()
}

async fn run(
    base_url: &str,
    model_id: &str,
    options: TranscriptionStreamOptions,
) -> Vec<Result<TranscriptionStreamPart, AiMuxError>> {
    let result = model(base_url, model_id).do_stream(options).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), result.stream.collect())
        .await
        .expect("stream finished")
}

fn describe(part: &TranscriptionStreamPart) -> String {
    match part {
        TranscriptionStreamPart::StreamStart { warnings } => format!("start:{}", warnings.len()),
        TranscriptionStreamPart::TranscriptPartial { id, text, .. } => {
            format!("partial:{}:{text}", id.as_deref().unwrap())
        }
        TranscriptionStreamPart::TranscriptDelta { id, delta, .. } => {
            format!("delta:{}:{delta}", id.as_deref().unwrap())
        }
        TranscriptionStreamPart::TranscriptFinal { id, text, .. } => {
            format!("final:{}:{text}", id.as_deref().unwrap())
        }
        TranscriptionStreamPart::Finish { text, language, .. } => {
            format!("finish:{text}:{}", language.as_deref().unwrap_or("-"))
        }
        TranscriptionStreamPart::Raw { .. } => "raw".into(),
        other => format!("{other:?}"),
    }
}

fn described(parts: &[Result<TranscriptionStreamPart, AiMuxError>]) -> Vec<String> {
    parts
        .iter()
        .map(|part| describe(part.as_ref().unwrap()))
        .collect()
}

fn idle() -> Value {
    json!({"serverContent": {"interactionStatus": "IDLE"}})
}

fn finished(text: &str) -> Value {
    json!({"serverContent": {"inputTranscription": {"text": text, "finished": true}}})
}

/// TS: streams transcription over the Gemini Live API WebSocket
#[tokio::test]
async fn streams_transcription_over_the_gemini_live_api_websocket() {
    let (base_url, captured) = live_server(Script {
        messages: vec![
            json!({"serverContent": {"interimInputTranscription": {"text": "hel"}}}),
            json!({"serverContent": {"inputTranscription": {"text": "hello ", "languageCode": "en-US"}}}),
            json!({"serverContent": {"inputTranscription": {"text": "world.", "finished": true}}}),
            json!({"usageMetadata": {"promptTokenCount": 7}, "serverContent": {"turnComplete": true, "interactionStatus": "IDLE"}}),
        ],
        expect_audio: true,
        close: None,
    })
    .await;
    let parts = run(
        &base_url,
        "gemini-3.5-transcribe-live",
        stream_options(
            one_chunk(),
            Some(16000),
            google_options(json!({"customVocabulary": ["Gemini"], "languageCodes": ["en-US"]})),
            false,
        ),
    )
    .await;
    assert_eq!(
        described(&parts),
        [
            "start:0",
            "partial:google-segment-0:hel",
            "delta:google-segment-0:hello ",
            "delta:google-segment-0:world.",
            "final:google-segment-0:hello world.",
            "finish:hello world.:en-US",
        ]
    );
    let Some(Ok(TranscriptionStreamPart::Finish {
        segments,
        provider_metadata,
        ..
    })) = parts.last()
    else {
        panic!("expected finish")
    };
    assert!(segments.is_empty());
    assert_eq!(
        serde_json::to_value(provider_metadata.as_ref().unwrap()).unwrap(),
        json!({"google": {"usageMetadata": {"promptTokenCount": 7}}})
    );
    let captured = captured.lock().unwrap();
    assert!(captured.uri.starts_with(
        "/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?key=test-api-key"
    ));
    // setup, then (after setupComplete) the audio chunk and audioStreamEnd
    assert_eq!(
        captured.received,
        [
            json!({"setup": {
                "model": "models/gemini-3.5-transcribe-live",
                "inputAudioTranscription": {"languageCodes": ["en-US"], "customVocabulary": ["Gemini"]},
            }}),
            json!({"realtimeInput": {"audio": {"data": "AQID", "mimeType": "audio/pcm;rate=16000"}}}),
            json!({"realtimeInput": {"audioStreamEnd": true}}),
        ]
    );
}

/// TS: passes the SMART transcription mode into the live setup
#[tokio::test]
async fn passes_the_smart_transcription_mode_into_the_live_setup() {
    let (base_url, captured) = live_server(Script {
        messages: vec![finished("hi"), idle()],
        expect_audio: true,
        close: None,
    })
    .await;
    run(
        &base_url,
        "gemini-3.5-transcribe-live",
        stream_options(
            one_chunk(),
            Some(16000),
            google_options(json!({"mode": "SMART"})),
            false,
        ),
    )
    .await;
    assert_eq!(
        captured.lock().unwrap().received[0],
        json!({"setup": {
            "model": "models/gemini-3.5-transcribe-live",
            "inputAudioTranscription": {"mode": "SMART"},
        }})
    );
}

/// TS: accepts the pre-launch REQUIRES_ACTION interaction status as idle
#[tokio::test]
async fn accepts_the_pre_launch_requires_action_interaction_status_as_idle() {
    let (base_url, _) = live_server(Script {
        messages: vec![
            finished("hi"),
            json!({"serverContent": {"interactionStatus": "REQUIRES_ACTION"}}),
        ],
        expect_audio: true,
        close: None,
    })
    .await;
    let parts = run(
        &base_url,
        "gemini-3.5-transcribe-live",
        stream_options(one_chunk(), Some(16000), None, false),
    )
    .await;
    assert_eq!(described(&parts).last().unwrap(), "finish:hi:-");
}

/// TS: falls back to the latest interim partial when no final segment arrives
#[tokio::test]
async fn falls_back_to_the_latest_interim_partial_when_no_final_segment_arrives() {
    let (base_url, _) = live_server(Script {
        messages: vec![
            json!({"serverContent": {"interimInputTranscription": {"text": "hello wor"}}}),
            json!({"serverContent": {"interimInputTranscription": {"text": "hello world."}}}),
            idle(),
        ],
        expect_audio: true,
        close: None,
    })
    .await;
    let parts = run(
        &base_url,
        "gemini-3.5-transcribe-live",
        stream_options(one_chunk(), Some(16000), None, false),
    )
    .await;
    let described = described(&parts);
    assert_eq!(
        described[described.len() - 2],
        "final:google-segment-0:hello world."
    );
    assert_eq!(described[described.len() - 1], "finish:hello world.:-");
}

/// TS: finishes with accumulated text when the server closes after audio ended
#[tokio::test]
async fn finishes_with_accumulated_text_when_the_server_closes_after_audio_ended() {
    let (base_url, _) = live_server(Script {
        messages: vec![json!({"serverContent": {"inputTranscription": {"text": "partial words"}}})],
        expect_audio: true,
        close: Some((1000, "")),
    })
    .await;
    let parts = run(
        &base_url,
        "gemini-3.5-transcribe-live",
        stream_options(one_chunk(), Some(16000), None, false),
    )
    .await;
    assert_eq!(described(&parts).last().unwrap(), "finish:partial words:-");
}

/// TS: errors when the socket closes before the audio ended
#[tokio::test]
async fn errors_when_the_socket_closes_before_the_audio_ended() {
    let (base_url, _) = live_server(Script {
        messages: vec![],
        expect_audio: false,
        close: Some((1011, "internal error")),
    })
    .await;
    // a never-ending audio stream
    let parts = run(
        &base_url,
        "gemini-3.5-transcribe-live",
        stream_options(futures::stream::pending().boxed(), Some(16000), None, false),
    )
    .await;
    let error = parts.last().unwrap().as_ref().unwrap_err();
    assert!(error.to_string().contains("1011"), "{error}");
}

/// TS: surfaces server error messages
#[tokio::test]
async fn surfaces_server_error_messages() {
    let (base_url, _) = live_server(Script {
        messages: vec![json!({"error": {"message": "quota exceeded"}})],
        expect_audio: true,
        close: None,
    })
    .await;
    let parts = run(
        &base_url,
        "gemini-3.5-transcribe-live",
        stream_options(one_chunk(), Some(16000), None, false),
    )
    .await;
    let error = parts.last().unwrap().as_ref().unwrap_err();
    assert!(error.to_string().contains("quota exceeded"));
}

/// TS: rejects streaming on unary model ids
#[tokio::test]
async fn rejects_streaming_on_unary_model_ids() {
    let error = model("http://127.0.0.1:1", "gemini-3.5-transcribe")
        .do_stream(stream_options(one_chunk(), Some(16000), None, false))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not support streaming transcription")
    );
}

/// TS: rejects non-16kHz PCM input audio
#[tokio::test]
async fn rejects_non_16khz_pcm_input_audio() {
    let error = model("http://127.0.0.1:1", "gemini-3.5-transcribe-live")
        .do_stream(stream_options(one_chunk(), Some(24000), None, false))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("only supports 16kHz 16-bit PCM input audio")
    );
}

/// TS: emits raw chunks when includeRawChunks is set
#[tokio::test]
async fn emits_raw_chunks_when_include_raw_chunks_is_set() {
    let (base_url, _) = live_server(Script {
        messages: vec![finished("ok"), idle()],
        expect_audio: true,
        close: None,
    })
    .await;
    let parts = run(
        &base_url,
        "gemini-3.5-transcribe-live",
        stream_options(one_chunk(), Some(16000), None, true),
    )
    .await;
    assert!(described(&parts).iter().any(|part| part == "raw"));
}
