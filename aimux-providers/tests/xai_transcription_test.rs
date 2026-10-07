//! Rust translation of the xAI transcription model tests.
//! Source: `reference/aisdk-pinned/xai/src/xai-transcription-model.test.ts`
//!
//! `do_generate` runs against a wiremock server (`POST {base}/stt`).
//! `do_stream` runs against a local WebSocket server that plays the xAI STT
//! role: it captures the handshake (URI + headers), sends `transcript.created`,
//! records the binary audio frames and the final `audio.done` text frame, then
//! sends the scripted events.
//!
//! Not ported (JS-only mechanics, no Rust analogue):
//! - `specificationVersion` check (part of the first provider-info case)
//! - "should cancel the audio stream when the WebSocket constructor throws"
//! - "should cancel the audio stream when an audio send throws mid-stream"
//! - "should close the WebSocket and stop reading audio when the stream is
//!   cancelled"

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::transcription_model::{
    AudioChunk, AudioInput, InputAudioFormat, TranscriptionCallOptions, TranscriptionModel,
    TranscriptionResponse, TranscriptionResult, TranscriptionStreamOptions,
    TranscriptionStreamPart,
};
use aimux_providers::{XAIProviderSettings, XaiTranscriptionModel, create_xai};

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

fn xai_options(value: Value) -> SharedProviderOptions {
    HashMap::from([("xai".to_string(), value.as_object().unwrap().clone())])
}

// ── doGenerate helpers ──────────────────────────────────────────────────────

fn call(provider_options: Option<Value>, media_type: &str) -> TranscriptionCallOptions {
    let mut options =
        TranscriptionCallOptions::new(AudioInput::Binary(vec![1, 2, 3, 4]), media_type);
    options.provider_options = provider_options.map(xai_options);
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

fn default_body() -> Value {
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

fn stream_options(
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
        provider_options: provider_options.map(xai_options),
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
        stream_options(provider_options, "audio/pcm", Some(16000)),
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

// ── tests: model information ────────────────────────────────────────────────

/// TS: should expose correct provider and model information
#[test]
fn should_expose_correct_provider_and_model_information() {
    let model = model("https://api.x.ai/v1", None);
    assert_eq!(model.provider(), "xai.transcription");
    assert_eq!(model.model_id(), "");
}

// ── tests: doGenerate ───────────────────────────────────────────────────────

/// TS: doGenerate > should send a multipart request with the audio file
#[tokio::test]
async fn should_send_a_multipart_request_with_the_audio_file() {
    let server = MockServer::start().await;
    mock_stt(&server, json_response(default_body(), &[])).await;

    generate(&server, &call(None, "audio/wav"), None).await;

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[0].method.as_str(), "POST");
    assert_eq!(requests[0].url.path(), "/stt");
    let parts = parse_multipart(&requests[0]);
    let file = parts.iter().find(|p| p.name == "file").expect("file field");
    assert_eq!(file.filename.as_deref(), Some("audio.wav"));
    assert_eq!(file.content_type.as_deref(), Some("audio/wav"));
}

/// TS: doGenerate > should map provider options onto xAI request fields
#[tokio::test]
async fn should_map_provider_options_onto_xai_request_fields() {
    let server = MockServer::start().await;
    mock_stt(&server, json_response(default_body(), &[])).await;

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

/// TS: doGenerate > should append file after all other multipart fields
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

/// TS: doGenerate > should pass headers and the xAI user agent
#[tokio::test]
async fn should_pass_headers_and_the_xai_user_agent() {
    let server = MockServer::start().await;
    mock_stt(&server, json_response(default_body(), &[])).await;

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

/// TS: doGenerate > should extract text, segments, language, and duration
#[tokio::test]
async fn should_extract_text_segments_language_and_duration() {
    let server = MockServer::start().await;
    mock_stt(&server, json_response(default_body(), &[])).await;

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

/// TS: doGenerate > should include response timestamp, model id, and headers
#[tokio::test]
async fn should_include_response_timestamp_model_id_and_headers() {
    let server = MockServer::start().await;
    let headers = [
        ("x-request-id", "test-request-id"),
        ("x-ratelimit-remaining", "123"),
    ];
    mock_stt(&server, json_response(default_body(), &headers)).await;

    let result = generate(&server, &call(None, "audio/wav"), None).await;

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id.as_deref(), Some(""));
    let response_headers = result.response.headers.unwrap();
    assert_eq!(response_headers["content-type"], "application/json");
    assert_eq!(response_headers["x-request-id"], "test-request-id");
    assert_eq!(response_headers["x-ratelimit-remaining"], "123");
    assert_eq!(result.response.body, Some(default_body()));
}

/// TS: doGenerate > should handle missing words, duration, and empty language
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

// ── tests: doStream ─────────────────────────────────────────────────────────

/// TS: doStream > should require channels when streaming multichannel audio
#[tokio::test]
async fn should_require_channels_when_streaming_multichannel_audio() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());

    let options = stream_options(
        Some(json!({"multichannel": true})),
        "audio/pcm",
        Some(16000),
    );
    let error = model(&base_url, None).do_stream(options).await.unwrap_err();

    assert!(
        matches!(&error, AiMuxError::InvalidArgument(message) if message.contains(
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

/// TS: doStream > should stream xAI STT over WebSocket
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

/// TS: doStream > should strip undefined header values before the WebSocket constructor
///
/// Rust analogue: a provider header set to `None` is not sent on the handshake.
#[tokio::test]
async fn should_not_send_removed_header_values_on_the_handshake() {
    let headers = HashMap::from([
        (
            "Custom-Header".to_string(),
            Some("custom-value".to_string()),
        ),
        ("X-Unset".to_string(), None),
    ]);
    let options = stream_options(None, "audio/pcm", Some(16000));
    let run = run_with(options, Some(headers), vec![done("Hello", 1.0)]).await;

    assert_eq!(run.cap.headers["authorization"], "Bearer test-api-key");
    assert_eq!(run.cap.headers["custom-header"], "custom-value");
    assert!(!run.cap.headers.contains_key("x-unset"));
}

/// TS: doStream > should emit one transcript-final per utterance and reconstruct the finish text when transcript.done is empty
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

/// TS: doStream > should treat is_final fragments as partials and use the speech_final text for finish
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

/// TS: doStream > should fall back to the latest pending text when no speech_final arrived before transcript.done
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

/// TS: doStream > should join finalized utterances per channel when transcript.done is empty
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

/// TS: doStream > should error the stream with the server message on error events
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
        matches!(&run.parts[1], Err(AiMuxError::Other(message)) if message == "invalid sample_rate"),
        "got {:?}",
        run.parts[1]
    );
}

/// TS: doStream > should warn on unrecognized inputAudioFormat types
#[tokio::test]
async fn should_warn_on_unrecognized_input_audio_format_types() {
    let options = stream_options(None, "audio/wav", None);
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
