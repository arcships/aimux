use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_core::transcription_model::{
    AudioChunk, AudioInput, InputAudioFormat, TranscriptionCallOptions, TranscriptionModel,
    TranscriptionStreamOptions,
};
use aimux_provider_utils::{HeaderMapOpt, Resolvable};

use crate::shared::{Endpoint, EndpointConfig, ResolveEndpoint};
use crate::test_support::mock_fetch::{Canned, RouteFetch};
use crate::vertex::{VertexGeminiTranscriptionModel, VertexProviderSettings, create_google_vertex};

const BASE_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/test-project/locations/us-central1/publishers/google";

/// A base URL and headers fixed for every request (the upstream model config).
struct StaticEndpoint {
    base_url: String,
    headers: HeaderMapOpt,
}

#[async_trait::async_trait]
impl ResolveEndpoint for StaticEndpoint {
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        Ok(Endpoint {
            base_url: self.base_url.clone(),
            headers: self.headers.clone(),
        })
    }
}

/// `createModel(modelId)`: `provider: 'test-provider'`, `baseURL: BASE_URL`,
/// `headers: () => ({ Authorization: 'Bearer test-oauth-token' })`, project
/// `test-project` and location `us-central1` (taken from a provider with those
/// settings), and the mock WebSocket.
fn create_model(
    model_id: &str,
    fetch: Option<&Arc<RouteFetch>>,
    #[cfg(feature = "realtime")] web_socket: Option<Arc<MockWebSocket>>,
) -> VertexGeminiTranscriptionModel {
    let provider = create_google_vertex(VertexProviderSettings {
        project: Some("test-project".into()),
        location: Some("us-central1".into()),
        access_token: Some(Resolvable::Value("unused".into())),
        ..Default::default()
    })
    .unwrap();
    VertexGeminiTranscriptionModel::from_config(
        model_id.to_string(),
        EndpointConfig::new(
            "test-provider".to_string(),
            StaticEndpoint {
                base_url: BASE_URL.to_string(),
                headers: [(
                    "Authorization".to_string(),
                    Some("Bearer test-oauth-token".to_string()),
                )]
                .into(),
            },
            fetch.map(RouteFetch::transport),
        ),
        provider.project_location("transcription"),
        #[cfg(feature = "realtime")]
        web_socket.map(|socket| socket as Arc<dyn aimux_provider_utils::ws::WsConnector>),
    )
}

/// The `doGenerate` test server: the generateContent URL answers `body`,
/// anything else 404.
fn test_server(body: Value) -> Arc<RouteFetch> {
    let url = format!("{BASE_URL}/models/gemini-3.5-transcribe:generateContent");
    let response = Canned::json(&body);
    RouteFetch::new(move |seen| {
        if seen.url == url {
            response.clone()
        } else {
            Canned {
                status: 404,
                headers: Vec::new(),
                body: b"Not found".to_vec(),
            }
        }
    })
}

fn default_server() -> Arc<RouteFetch> {
    test_server(json!({
        "candidates": [{"content": {"parts": [{"text": "Hello "}, {"text": "world."}]}}],
        "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 4},
    }))
}

fn provider_options(value: Value) -> Option<SharedProviderOptions> {
    Some(serde_json::from_value(value).unwrap())
}

fn generate_options(audio: Vec<u8>, provider_options_value: Value) -> TranscriptionCallOptions {
    let mut options = TranscriptionCallOptions::new(AudioInput::Binary(audio), "audio/wav");
    options.provider_options = provider_options(provider_options_value);
    options
}

/// The message of an error, without the variant's display prefix.
fn message(error: &AiMuxError) -> String {
    match error {
        AiMuxError::InvalidArgument { message, .. }
        | AiMuxError::InvalidResponseData { message, .. }
        | AiMuxError::UnsupportedFunctionality(message) => message.clone(),
        AiMuxError::ApiCall(error) => error.message.clone(),
        other => other.to_string(),
    }
}

#[cfg(feature = "realtime")]
use ws_mock::*;

/// An in-process stand-in for upstream's `MockWebSocket`: the connector pairs
/// the client with a scripted server over an in-memory pipe and records what
/// the client did.
#[cfg(feature = "realtime")]
mod ws_mock {
    use super::*;

    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::WebSocketStream;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::protocol::Role;
    use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

    use aimux_provider_utils::ws::{WebSocketRequest, WsConnection, WsConnector};

    /// One step of the server side.
    pub(super) enum Step {
        /// Receive the client's first (setup) message.
        ReadSetup,
        /// Give the client time to send more before `setupComplete`, then note
        /// how many messages it had sent (`expect(ws.send).toHaveBeenCalledTimes`).
        CountSentBeforeSetupComplete,
        /// Send a JSON message (`ws.message(value)`).
        Send(Value),
        /// Receive messages until `audioStreamEnd`.
        WaitForAudioStreamEnd,
        /// Close from the server side (`ws.serverClose(code, reason)`).
        ServerClose(u16, &'static str),
    }

    #[derive(Default)]
    pub(super) struct WsSeen {
        pub(super) url: Option<String>,
        pub(super) headers: Vec<(String, String)>,
        /// Every message the client sent, parsed (`ws.send.mock.calls`).
        pub(super) sent: Vec<Value>,
        pub(super) sent_before_setup_complete: Option<usize>,
        /// The code of the client's close frame (`ws.close` call argument).
        pub(super) close_code: Option<u16>,
    }

    pub(super) struct MockWebSocket {
        pub(super) seen: Arc<Mutex<WsSeen>>,
        script: Mutex<Option<Vec<Step>>>,
    }

    impl MockWebSocket {
        pub(super) fn new(script: Vec<Step>) -> Arc<Self> {
            Arc::new(Self {
                seen: Arc::default(),
                script: Mutex::new(Some(script)),
            })
        }

        /// Waits (bounded) until the client closed the socket.
        pub(super) async fn close_code(&self) -> Option<u16> {
            for _ in 0..200 {
                if let Some(code) = self.seen.lock().unwrap().close_code {
                    return Some(code);
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            None
        }
    }

    #[async_trait::async_trait]
    impl WsConnector for MockWebSocket {
        async fn connect(&self, request: &WebSocketRequest) -> Result<WsConnection, AiMuxError> {
            {
                let mut seen = self.seen.lock().unwrap();
                seen.url = Some(request.url.clone());
                seen.headers = request.headers.clone();
            }
            let script = self.script.lock().unwrap().take().expect("one connection");
            let (client_io, server_io) = tokio::io::duplex(64 * 1024);
            let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
            let server = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
            tokio::spawn(serve(server, Arc::clone(&self.seen), script));
            Ok(WsConnection::from_transport(Box::new(client), request))
        }
    }

    type Server = WebSocketStream<tokio::io::DuplexStream>;

    /// Receives one frame; records a text frame and returns it.
    async fn receive(ws: &mut Server, seen: &Mutex<WsSeen>) -> Option<Option<Value>> {
        match ws.next().await? {
            Ok(Message::Text(text)) => {
                let value: Value = serde_json::from_str(text.as_str()).expect("client sends JSON");
                seen.lock().unwrap().sent.push(value.clone());
                Some(Some(value))
            }
            Ok(Message::Close(frame)) => {
                seen.lock().unwrap().close_code =
                    Some(frame.map_or(1005, |frame| u16::from(frame.code)));
                None
            }
            Ok(_) => Some(None),
            Err(_) => None,
        }
    }

    async fn serve(mut ws: Server, seen: Arc<Mutex<WsSeen>>, script: Vec<Step>) {
        for step in script {
            match step {
                Step::ReadSetup => while let Some(None) = receive(&mut ws, &seen).await {},
                Step::CountSentBeforeSetupComplete => {
                    while let Ok(Some(_)) = tokio::time::timeout(
                        std::time::Duration::from_millis(100),
                        receive(&mut ws, &seen),
                    )
                    .await
                    {}
                    let count = seen.lock().unwrap().sent.len();
                    seen.lock().unwrap().sent_before_setup_complete = Some(count);
                }
                Step::Send(value) => {
                    ws.send(Message::Text(value.to_string())).await.unwrap();
                }
                Step::WaitForAudioStreamEnd => loop {
                    match receive(&mut ws, &seen).await {
                        Some(Some(value)) if value["realtimeInput"]["audioStreamEnd"] == true => {
                            break;
                        }
                        Some(_) => {}
                        None => return,
                    }
                },
                Step::ServerClose(code, reason) => {
                    let _ = ws
                        .send(Message::Close(Some(CloseFrame {
                            code: CloseCode::from(code),
                            reason: reason.into(),
                        })))
                        .await;
                }
            }
        }
        while receive(&mut ws, &seen).await.is_some() {}
    }
}

#[cfg(feature = "realtime")]
fn stream_options(
    audio: futures::stream::BoxStream<'static, AudioChunk>,
    rate: u32,
    provider_options_value: Value,
) -> TranscriptionStreamOptions {
    TranscriptionStreamOptions {
        audio,
        input_audio_format: InputAudioFormat {
            format_type: "audio/pcm".into(),
            rate: Some(rate),
        },
        provider_options: provider_options(provider_options_value),
        abort_signal: None,
        headers: None,
        include_raw_chunks: false,
        timeout: None,
    }
}

#[cfg(feature = "realtime")]
fn chunks(chunks: Vec<Vec<u8>>) -> futures::stream::BoxStream<'static, AudioChunk> {
    use futures::StreamExt;
    futures::stream::iter(chunks.into_iter().map(AudioChunk::Binary)).boxed()
}

/// `convertReadableStreamToArray(result.stream)`, each part as its JSON.
#[cfg(feature = "realtime")]
async fn collect_parts(
    stream: std::pin::Pin<
        Box<
            dyn futures::Stream<
                    Item = Result<
                        aimux_core::transcription_model::TranscriptionStreamPart,
                        AiMuxError,
                    >,
                > + Send,
        >,
    >,
) -> Result<Vec<Value>, AiMuxError> {
    use futures::StreamExt;
    let parts: Vec<_> = tokio::time::timeout(std::time::Duration::from_secs(10), stream.collect())
        .await
        .expect("the stream ends");
    parts
        .into_iter()
        .map(|part| part.map(|part| serde_json::to_value(part).unwrap()))
        .collect()
}

mod do_generate {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/gemini-transcription/google-vertex-gemini-transcription-model.test.ts › doGenerate › transcribes audio via Vertex generateContent with audioTranscriptionConfig
    #[tokio::test]
    async fn transcribes_audio_via_vertex_generate_content_with_audio_transcription_config() {
        let server = default_server();
        let model = create_model(
            "gemini-3.5-transcribe",
            Some(&server),
            #[cfg(feature = "realtime")]
            None,
        );
        let result = model
            .do_generate(&generate_options(
                vec![1, 2, 3, 4],
                json!({
                    "googleVertex": {
                        "customVocabulary": ["Gemini", "Kubernetes"],
                        "languageCodes": ["es-ES"],
                        "mode": "SMART",
                    },
                }),
            ))
            .await
            .unwrap();
        assert_eq!(result.text, "Hello world.");
        let seen = server.seen();
        assert_eq!(
            seen[0].json_body(),
            json!({
                "contents": [{
                    "role": "user",
                    "parts": [{"inlineData": {"mimeType": "audio/wav", "data": "AQIDBA=="}}],
                }],
                "generationConfig": {
                    "audioTranscriptionConfig": {
                        "languageCodes": ["es-ES"],
                        "customVocabulary": ["Gemini", "Kubernetes"],
                        "mode": "SMART",
                    },
                },
            })
        );
        assert_eq!(
            seen[0].headers.get("authorization").map(String::as_str),
            Some("Bearer test-oauth-token")
        );
        assert_eq!(
            serde_json::to_value(result.provider_metadata).unwrap(),
            json!({"google": {"usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 4}}})
        );
        // `result.usage` is not asserted: aimux's `TranscriptionResult` has no
        // `usage` field.
    }

    /// upstream: @ai-sdk/google-vertex src/gemini-transcription/google-vertex-gemini-transcription-model.test.ts › doGenerate › accepts options under the google namespace as a fallback
    #[tokio::test]
    async fn accepts_options_under_the_google_namespace_as_a_fallback() {
        let server = default_server();
        let model = create_model(
            "gemini-3.5-transcribe",
            Some(&server),
            #[cfg(feature = "realtime")]
            None,
        );
        model
            .do_generate(&generate_options(
                vec![1, 2, 3, 4],
                json!({"google": {"mode": "SMART"}}),
            ))
            .await
            .unwrap();
        assert_eq!(
            server.seen()[0].json_body()["generationConfig"],
            json!({"audioTranscriptionConfig": {"mode": "SMART"}})
        );
    }

    /// upstream: @ai-sdk/google-vertex src/gemini-transcription/google-vertex-gemini-transcription-model.test.ts › doGenerate › extracts text, language, and word segments from the audioTranscription part
    #[tokio::test]
    async fn extracts_text_language_and_word_segments_from_the_audio_transcription_part() {
        let server = test_server(json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "audioTranscription": {
                            "text": "The quick brown fox.",
                            "languageCode": "en-US",
                            "speakerLabel": "spk:0",
                            "words": [
                                {"word": "The", "startOffset": "0.100s", "endOffset": "0.100s"},
                                {"word": "quick", "startOffset": "0.100s", "endOffset": "0.400s"},
                                {"word": "brown", "startOffset": "0.400s", "endOffset": "0.700s"},
                                {"word": "fox.", "startOffset": "0.700s", "endOffset": "1s"},
                            ],
                        },
                    }],
                },
            }],
            "usageMetadata": {"promptTokenCount": 64},
        }));
        let model = create_model(
            "gemini-3.5-transcribe",
            Some(&server),
            #[cfg(feature = "realtime")]
            None,
        );
        let result = model
            .do_generate(&generate_options(vec![1, 2, 3, 4], json!({})))
            .await
            .unwrap();
        assert_eq!(result.text, "The quick brown fox.");
        assert_eq!(result.language.as_deref(), Some("en-US"));
        assert_eq!(
            serde_json::to_value(&result.segments).unwrap(),
            json!([
                {"text": "The", "startSecond": 0.1, "endSecond": 0.1},
                {"text": "quick", "startSecond": 0.1, "endSecond": 0.4},
                {"text": "brown", "startSecond": 0.4, "endSecond": 0.7},
                {"text": "fox.", "startSecond": 0.7, "endSecond": 1.0},
            ])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/gemini-transcription/google-vertex-gemini-transcription-model.test.ts › doGenerate › rejects unary transcription on live model ids
    #[tokio::test]
    async fn rejects_unary_transcription_on_live_model_ids() {
        let server = default_server();
        let model = create_model(
            "gemini-3.5-transcribe-live",
            Some(&server),
            #[cfg(feature = "realtime")]
            None,
        );
        let error = model
            .do_generate(&generate_options(vec![1], json!({})))
            .await
            .unwrap_err();
        assert!(
            message(&error).contains("only supports streaming transcription"),
            "{error:?}"
        );
    }
}

#[cfg(feature = "realtime")]
mod do_stream {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/gemini-transcription/google-vertex-gemini-transcription-model.test.ts › doStream › streams transcription over the Vertex Live API WebSocket
    #[tokio::test]
    async fn streams_transcription_over_the_vertex_live_api_web_socket() {
        let ws = MockWebSocket::new(vec![
            Step::ReadSetup,
            Step::CountSentBeforeSetupComplete,
            Step::Send(json!({"setupComplete": {}})),
            Step::WaitForAudioStreamEnd,
            Step::Send(json!({"serverContent": {"interimInputTranscription": {"text": "hel"}}})),
            Step::Send(json!({
                "serverContent": {"inputTranscription": {"text": "hello world.", "finished": true}},
            })),
            Step::Send(json!({
                "usageMetadata": {"promptTokenCount": 7},
                "serverContent": {"turnComplete": true, "interactionStatus": "IDLE"},
            })),
        ]);
        let model = create_model("gemini-3.5-transcribe-live", None, Some(ws.clone()));
        let result = model
            .do_stream(stream_options(
                chunks(vec![vec![1, 2, 3]]),
                16000,
                json!({"googleVertex": {"customVocabulary": ["Gemini"], "mode": "SMART"}}),
            ))
            .expect("streaming is supported")
            .await
            .unwrap();
        let parts = collect_parts(result.stream).await.unwrap();

        {
            let seen = ws.seen.lock().unwrap();
            assert_eq!(
                seen.url.as_deref(),
                Some(
                    "wss://us-central1-aiplatform.googleapis.com/ws/google.cloud.aiplatform.v1.LlmBidiService/BidiGenerateContent"
                )
            );
            // Header names compared case-insensitively.
            let headers: std::collections::BTreeMap<String, String> = seen
                .headers
                .iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
                .collect();
            assert_eq!(
                serde_json::to_value(headers).unwrap(),
                json!({"authorization": "Bearer test-oauth-token"})
            );

            // audio is gated on setupComplete: only the setup message was sent
            assert_eq!(seen.sent_before_setup_complete, Some(1));
            assert_eq!(
                seen.sent[0],
                json!({
                    "setup": {
                        "model": "projects/test-project/locations/us-central1/publishers/google/models/gemini-3.5-transcribe-live",
                        "inputAudioTranscription": {"customVocabulary": ["Gemini"], "mode": "SMART"},
                    },
                })
            );

            // audio chunk + audioStreamEnd
            assert_eq!(seen.sent.len(), 3);
            assert_eq!(
                seen.sent[1],
                json!({"realtimeInput": {"audio": {"data": "AQID", "mimeType": "audio/pcm;rate=16000"}}})
            );
            assert_eq!(
                seen.sent[2],
                json!({"realtimeInput": {"audioStreamEnd": true}})
            );
        }

        // The finish part's `usage` is left out: aimux's finish part has no
        // `usage` field.
        assert_eq!(
            parts,
            vec![
                json!({"type": "stream-start", "warnings": []}),
                json!({"type": "transcript-partial", "id": "google-segment-0", "text": "hel"}),
                json!({"type": "transcript-delta", "id": "google-segment-0", "delta": "hello world."}),
                json!({"type": "transcript-final", "id": "google-segment-0", "text": "hello world."}),
                json!({
                    "type": "finish",
                    "text": "hello world.",
                    "segments": [],
                    "providerMetadata": {"google": {"usageMetadata": {"promptTokenCount": 7}}},
                }),
            ]
        );
        assert_eq!(ws.close_code().await, Some(1000));
    }

    /// upstream: @ai-sdk/google-vertex src/gemini-transcription/google-vertex-gemini-transcription-model.test.ts › doStream › falls back to the latest interim partial when no final segment arrives
    #[tokio::test]
    async fn falls_back_to_the_latest_interim_partial_when_no_final_segment_arrives() {
        let ws = MockWebSocket::new(vec![
            Step::ReadSetup,
            Step::Send(json!({"setupComplete": {}})),
            Step::WaitForAudioStreamEnd,
            Step::Send(
                json!({"serverContent": {"interimInputTranscription": {"text": "hello world."}}}),
            ),
            Step::Send(json!({"serverContent": {"interactionStatus": "IDLE"}})),
        ]);
        let model = create_model("gemini-3.5-transcribe-live", None, Some(ws.clone()));
        let result = model
            .do_stream(stream_options(chunks(vec![vec![1, 2]]), 16000, json!({})))
            .expect("streaming is supported")
            .await
            .unwrap();
        let parts = collect_parts(result.stream).await.unwrap();
        // `toMatchObject`
        let last = parts.last().expect("a part");
        assert_eq!(last["type"], "finish");
        assert_eq!(last["text"], "hello world.");
    }

    /// upstream: @ai-sdk/google-vertex src/gemini-transcription/google-vertex-gemini-transcription-model.test.ts › doStream › errors when the socket closes before the audio ended
    #[tokio::test]
    async fn errors_when_the_socket_closes_before_the_audio_ended() {
        use futures::StreamExt;

        let ws = MockWebSocket::new(vec![
            Step::ReadSetup,
            Step::Send(json!({"setupComplete": {}})),
            Step::ServerClose(1011, "internal error"),
        ]);
        let model = create_model("gemini-3.5-transcribe-live", None, Some(ws.clone()));
        let result = model
            .do_stream(stream_options(
                futures::stream::pending().boxed(),
                16000,
                json!({}),
            ))
            .expect("streaming is supported")
            .await
            .unwrap();
        let error = collect_parts(result.stream).await.unwrap_err();
        assert!(
            message(&error).contains(
                "closed unexpectedly before finishing (code 1011, reason: internal error)"
            ),
            "{error:?}"
        );
    }

    /// upstream: @ai-sdk/google-vertex src/gemini-transcription/google-vertex-gemini-transcription-model.test.ts › doStream › rejects streaming on unary model ids
    #[tokio::test]
    async fn rejects_streaming_on_unary_model_ids() {
        let ws = MockWebSocket::new(Vec::new());
        let model = create_model("gemini-3.5-transcribe", None, Some(ws));
        let error = model
            .do_stream(stream_options(chunks(vec![vec![1]]), 16000, json!({})))
            .expect("the model implements doStream")
            .await
            .unwrap_err();
        assert!(
            message(&error).contains("does not support streaming transcription"),
            "{error:?}"
        );
    }

    /// upstream: @ai-sdk/google-vertex src/gemini-transcription/google-vertex-gemini-transcription-model.test.ts › doStream › rejects non-16kHz PCM input audio
    #[tokio::test]
    async fn rejects_non_16khz_pcm_input_audio() {
        let ws = MockWebSocket::new(Vec::new());
        let model = create_model("gemini-3.5-transcribe-live", None, Some(ws));
        let error = model
            .do_stream(stream_options(chunks(vec![vec![1]]), 24000, json!({})))
            .expect("the model implements doStream")
            .await
            .unwrap_err();
        assert!(
            message(&error).contains("only supports 16kHz 16-bit PCM input audio"),
            "{error:?}"
        );
    }
}

/// aimux's own: Vertex reads the `google` namespace when no Vertex one is
/// given; a wrong type there fails as `parseProviderOptions` fails, naming
/// that namespace.
#[tokio::test]
async fn a_wrong_typed_google_fallback_fails_with_the_parse_provider_options_message() {
    let server = default_server();
    let model = create_model(
        "gemini-3.5-transcribe",
        Some(&server),
        #[cfg(feature = "realtime")]
        None,
    );
    let error = model
        .do_generate(&generate_options(
            vec![1],
            json!({ "google": { "languageCodes": "en" } }),
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AiMuxError::InvalidArgument { message, cause: Some(_) }
            if message == "Invalid argument for parameter providerOptions: invalid google provider options"),
        "{error:?}"
    );
}
