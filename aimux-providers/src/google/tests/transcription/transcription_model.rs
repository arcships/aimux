use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::DuplexStream;
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role};

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_core::transcription_model::{
    AudioChunk, AudioInput, InputAudioFormat, TranscriptionCallOptions, TranscriptionModel,
    TranscriptionStreamOptions, TranscriptionStreamPart,
};
use aimux_provider_utils::ws::{WebSocketRequest, WsConnection, WsConnector};

use crate::google::{GoogleProviderSettings, GoogleTranscriptionModel, create_google};
use crate::test_support::mock_fetch::{Canned, MockFetch};

/// The server end of one mocked WebSocket (upstream `MockWebSocket`): the
/// test reads what the model sent and plays the Gemini Live API.
type ServerSocket = WebSocketStream<DuplexStream>;

/// Upstream `webSocket: MockWebSocket`: every connection is an in-memory
/// pipe whose server end, with the requested URL, goes to the test.
struct MockWebSocket {
    instances: mpsc::UnboundedSender<(String, ServerSocket)>,
}

#[async_trait::async_trait]
impl WsConnector for MockWebSocket {
    async fn connect(&self, request: &WebSocketRequest) -> Result<WsConnection, AiMuxError> {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
        let server = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
        self.instances
            .send((request.url.clone(), server))
            .expect("test holds the receiver");
        Ok(WsConnection::from_transport(Box::new(client), request))
    }
}

/// Upstream `createModel`: the default base URL, `x-goog-api-key:
/// test-api-key`, the mock WebSocket and (for the unary cases) a mock fetch.
fn create_model(
    model_id: &str,
    fetch: Option<&Arc<MockFetch>>,
) -> (
    GoogleTranscriptionModel,
    mpsc::UnboundedReceiver<(String, ServerSocket)>,
) {
    let (instances, receiver) = mpsc::unbounded_channel();
    let model = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: fetch.map(MockFetch::transport),
        web_socket: Some(Arc::new(MockWebSocket { instances })),
        ..Default::default()
    })
    .unwrap()
    .transcription(model_id);
    (model, receiver)
}

fn google_options(value: Value) -> Option<SharedProviderOptions> {
    let Value::Object(map) = value else {
        panic!("options must be an object")
    };
    Some([("google".to_string(), map)].into_iter().collect())
}

/// `providerOptions: {}`.
fn empty_options() -> Option<SharedProviderOptions> {
    Some(SharedProviderOptions::new())
}

fn wav_call(provider_options: Option<SharedProviderOptions>) -> TranscriptionCallOptions {
    let mut call = TranscriptionCallOptions::new(AudioInput::Binary(vec![1, 2, 3, 4]), "audio/wav");
    call.provider_options = provider_options;
    call
}

fn interactions_response() -> Value {
    json!({
        "id": "interactions/test",
        "status": "completed",
        "steps": [{"type": "model_output", "content": [{"type": "text", "text": "Hello world."}]}],
        "usage": {"total_tokens": 10, "total_input_tokens": 10, "total_output_tokens": 0},
    })
}

/// `convertArrayToReadableStream(chunks)`.
fn audio_stream(
    chunks: Vec<Vec<u8>>,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = AudioChunk> + Send>> {
    Box::pin(futures::stream::iter(
        chunks.into_iter().map(AudioChunk::Binary),
    ))
}

fn stream_options(
    audio: std::pin::Pin<Box<dyn futures::Stream<Item = AudioChunk> + Send>>,
    rate: u32,
    provider_options: Option<SharedProviderOptions>,
) -> TranscriptionStreamOptions {
    TranscriptionStreamOptions {
        audio,
        input_audio_format: InputAudioFormat {
            format_type: "audio/pcm".into(),
            rate: Some(rate),
        },
        provider_options,
        abort_signal: None,
        headers: None,
        include_raw_chunks: false,
        timeout: None,
    }
}

type Parts = Vec<Result<TranscriptionStreamPart, AiMuxError>>;

/// `convertReadableStreamToArray(result.stream)`, running while the test
/// plays the server.
async fn start_stream(
    model: &GoogleTranscriptionModel,
    options: TranscriptionStreamOptions,
) -> tokio::task::JoinHandle<Parts> {
    let result = model
        .do_stream(options)
        .expect("the model streams")
        .await
        .unwrap();
    tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(10), result.stream.collect::<Parts>())
            .await
            .expect("the stream ends")
    })
}

async fn parts_of(handle: tokio::task::JoinHandle<Parts>) -> Parts {
    handle.await.unwrap()
}

fn json_parts(parts: &Parts) -> Vec<Value> {
    parts
        .iter()
        .map(|part| serde_json::to_value(part.as_ref().unwrap()).unwrap())
        .collect()
}

/// The next text frame the model sent, parsed (`ws.send.mock.calls[i][0]`).
async fn sent(ws: &mut ServerSocket) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("the model sends a frame")
        {
            Some(Ok(Message::Text(text))) => return serde_json::from_str(&text).unwrap(),
            Some(Ok(_)) => {}
            other => panic!("expected a text frame, got {other:?}"),
        }
    }
}

/// `ws.message(value)`.
async fn message(ws: &mut ServerSocket, value: Value) {
    ws.send(Message::Text(value.to_string())).await.unwrap();
}

/// `ws.serverClose(code, reason)`.
async fn server_close(ws: &mut ServerSocket, code: u16, reason: &str) {
    ws.send(Message::Close(Some(CloseFrame {
        code: CloseCode::from(code),
        reason: reason.to_string().into(),
    })))
    .await
    .unwrap();
}

/// Upstream's `await flush()` after `setupComplete`: the model has sent its
/// audio and `audioStreamEnd`.
async fn flush_audio(ws: &mut ServerSocket) {
    while sent(ws).await["realtimeInput"]["audioStreamEnd"] != true {}
}

mod do_generate {
    use super::*;

    /// upstream: @ai-sdk/google src/transcription/google-transcription-model.test.ts › doGenerate › omits generation_config when no transcription options are set
    #[tokio::test]
    async fn omits_generation_config_when_no_transcription_options_are_set() {
        let mock = MockFetch::new(vec![Canned::json(&interactions_response())]);
        let (model, _) = create_model("gemini-3.5-transcribe", Some(&mock));

        model.do_generate(&wav_call(empty_options())).await.unwrap();

        assert!(
            mock.seen()[0]
                .json_body()
                .get("generation_config")
                .is_none()
        );
    }
}

mod do_stream {
    use super::*;

    /// upstream: @ai-sdk/google src/transcription/google-transcription-model.test.ts › doStream › passes the SMART transcription mode into the live setup
    #[tokio::test]
    async fn passes_the_smart_transcription_mode_into_the_live_setup() {
        let (model, mut instances) = create_model("gemini-3.5-transcribe-live", None);

        let parts = start_stream(
            &model,
            stream_options(
                audio_stream(vec![vec![1, 2]]),
                16000,
                google_options(json!({"mode": "SMART"})),
            ),
        )
        .await;
        let (_, mut ws) = instances.recv().await.unwrap();

        assert_eq!(
            sent(&mut ws).await,
            json!({
                "setup": {
                    "model": "models/gemini-3.5-transcribe-live",
                    "inputAudioTranscription": {"mode": "SMART"},
                },
            })
        );

        message(&mut ws, json!({"setupComplete": {}})).await;
        flush_audio(&mut ws).await;
        message(
            &mut ws,
            json!({"serverContent": {"inputTranscription": {"text": "hi", "finished": true}}}),
        )
        .await;
        message(
            &mut ws,
            json!({"serverContent": {"interactionStatus": "IDLE"}}),
        )
        .await;

        let parts = parts_of(parts).await;
        assert!(parts.iter().all(Result::is_ok), "{parts:?}");
    }

    /// upstream: @ai-sdk/google src/transcription/google-transcription-model.test.ts › doStream › accepts the pre-launch REQUIRES_ACTION interaction status as idle
    #[tokio::test]
    async fn accepts_the_pre_launch_requires_action_interaction_status_as_idle() {
        let (model, mut instances) = create_model("gemini-3.5-transcribe-live", None);

        let parts = start_stream(
            &model,
            stream_options(audio_stream(vec![vec![1, 2]]), 16000, empty_options()),
        )
        .await;
        let (_, mut ws) = instances.recv().await.unwrap();
        message(&mut ws, json!({"setupComplete": {}})).await;
        flush_audio(&mut ws).await;

        message(
            &mut ws,
            json!({"serverContent": {"inputTranscription": {"text": "hi", "finished": true}}}),
        )
        .await;
        message(
            &mut ws,
            json!({"serverContent": {"interactionStatus": "REQUIRES_ACTION"}}),
        )
        .await;

        let parts = json_parts(&parts_of(parts).await);
        let last = parts.last().unwrap();
        assert_eq!(last["type"], "finish");
        assert_eq!(last["text"], "hi");
    }

    /// upstream: @ai-sdk/google src/transcription/google-transcription-model.test.ts › doStream › falls back to the latest interim partial when no final segment arrives
    #[tokio::test]
    async fn falls_back_to_the_latest_interim_partial_when_no_final_segment_arrives() {
        let (model, mut instances) = create_model("gemini-3.5-transcribe-live", None);

        let parts = start_stream(
            &model,
            stream_options(audio_stream(vec![vec![1, 2]]), 16000, empty_options()),
        )
        .await;
        let (_, mut ws) = instances.recv().await.unwrap();
        message(&mut ws, json!({"setupComplete": {}})).await;
        flush_audio(&mut ws).await;

        message(
            &mut ws,
            json!({"serverContent": {"interimInputTranscription": {"text": "hello wor"}}}),
        )
        .await;
        message(
            &mut ws,
            json!({"serverContent": {"interimInputTranscription": {"text": "hello world."}}}),
        )
        .await;
        message(
            &mut ws,
            json!({"serverContent": {"interactionStatus": "IDLE"}}),
        )
        .await;

        let parts = json_parts(&parts_of(parts).await);
        assert_eq!(
            parts[parts.len() - 2],
            json!({"type": "transcript-final", "id": "google-segment-0", "text": "hello world."})
        );
        let last = &parts[parts.len() - 1];
        assert_eq!(last["type"], "finish");
        assert_eq!(last["text"], "hello world.");
    }

    /// upstream: @ai-sdk/google src/transcription/google-transcription-model.test.ts › doStream › finishes with accumulated text when the server closes after audio ended
    #[tokio::test]
    async fn finishes_with_accumulated_text_when_the_server_closes_after_audio_ended() {
        let (model, mut instances) = create_model("gemini-3.5-transcribe-live", None);

        let parts = start_stream(
            &model,
            stream_options(audio_stream(vec![vec![1, 2]]), 16000, empty_options()),
        )
        .await;
        let (_, mut ws) = instances.recv().await.unwrap();
        message(&mut ws, json!({"setupComplete": {}})).await;
        flush_audio(&mut ws).await;

        message(
            &mut ws,
            json!({"serverContent": {"inputTranscription": {"text": "partial words"}}}),
        )
        .await;
        server_close(&mut ws, 1000, "").await;

        let parts = json_parts(&parts_of(parts).await);
        let last = parts.last().unwrap();
        assert_eq!(last["type"], "finish");
        assert_eq!(last["text"], "partial words");
    }

    /// upstream: @ai-sdk/google src/transcription/google-transcription-model.test.ts › doStream › errors when the socket closes before the audio ended
    #[tokio::test]
    async fn errors_when_the_socket_closes_before_the_audio_ended() {
        let (model, mut instances) = create_model("gemini-3.5-transcribe-live", None);

        // a never-ending audio stream: the reader stays pending
        let parts = start_stream(
            &model,
            stream_options(Box::pin(futures::stream::pending()), 16000, empty_options()),
        )
        .await;
        let (_, mut ws) = instances.recv().await.unwrap();
        message(&mut ws, json!({"setupComplete": {}})).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        server_close(&mut ws, 1011, "internal error").await;

        let parts = parts_of(parts).await;
        let error = parts.last().unwrap().as_ref().unwrap_err();
        assert!(
            error.to_string().contains(
                "closed unexpectedly before finishing (code 1011, reason: internal error)"
            ),
            "{error}"
        );
    }

    /// upstream: @ai-sdk/google src/transcription/google-transcription-model.test.ts › doStream › surfaces server error messages
    #[tokio::test]
    async fn surfaces_server_error_messages() {
        let (model, mut instances) = create_model("gemini-3.5-transcribe-live", None);

        let parts = start_stream(
            &model,
            stream_options(audio_stream(vec![vec![1]]), 16000, empty_options()),
        )
        .await;
        let (_, mut ws) = instances.recv().await.unwrap();
        message(&mut ws, json!({"setupComplete": {}})).await;
        flush_audio(&mut ws).await;
        message(&mut ws, json!({"error": {"message": "quota exceeded"}})).await;

        let parts = parts_of(parts).await;
        let error = parts.last().unwrap().as_ref().unwrap_err();
        assert!(error.to_string().contains("quota exceeded"), "{error}");
    }

    /// upstream: @ai-sdk/google src/transcription/google-transcription-model.test.ts › doStream › emits raw chunks when includeRawChunks is set
    #[tokio::test]
    async fn emits_raw_chunks_when_include_raw_chunks_is_set() {
        let (model, mut instances) = create_model("gemini-3.5-transcribe-live", None);

        let mut options = stream_options(audio_stream(vec![vec![1]]), 16000, empty_options());
        options.include_raw_chunks = true;
        let parts = start_stream(&model, options).await;
        let (_, mut ws) = instances.recv().await.unwrap();
        message(&mut ws, json!({"setupComplete": {}})).await;
        flush_audio(&mut ws).await;
        message(
            &mut ws,
            json!({"serverContent": {"inputTranscription": {"text": "ok", "finished": true}}}),
        )
        .await;
        message(
            &mut ws,
            json!({"serverContent": {"interactionStatus": "IDLE"}}),
        )
        .await;

        let parts = json_parts(&parts_of(parts).await);
        assert!(parts.iter().filter(|part| part["type"] == "raw").count() > 0);
    }
}
