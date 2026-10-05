//! Gemini transcription through Vertex generateContent and Live API.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use super::ProjectLocationFn;
use crate::google::options::Namespace;
use crate::shared::EndpointConfig;
use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, provider_namespace};
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionResponse,
    TranscriptionResult, TranscriptionSegment, TranscriptionStreamOptions,
    TranscriptionStreamResult,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiOptions {
    #[serde(default)]
    language_codes: Option<Vec<String>>,
    #[serde(default)]
    custom_vocabulary: Option<Vec<String>>,
    #[serde(default)]
    word_timestamp: Option<bool>,
    #[serde(default)]
    diarization: Option<bool>,
    #[serde(default)]
    mode: Option<Mode>,
}

#[derive(Deserialize)]
enum Mode {
    #[serde(rename = "SMART")]
    Smart,
    #[serde(rename = "VERBATIM")]
    Verbatim,
}

fn transcription_config(options: Option<&SharedProviderOptions>) -> Result<Value, AiMuxError> {
    let Some(options) = Namespace::Vertex.read(options) else {
        return Ok(json!({}));
    };
    // z.optional() accepts absence, but not explicit null.
    for key in [
        "languageCodes",
        "customVocabulary",
        "wordTimestamp",
        "diarization",
        "mode",
    ] {
        if options.get(key).is_some_and(Value::is_null) {
            return Err(AiMuxError::InvalidArgument(format!(
                "Google Vertex transcription option {key} cannot be null"
            )));
        }
    }
    let parsed: GeminiOptions = serde_json::from_value(Value::Object(options.clone()))
        .map_err(|error| AiMuxError::InvalidArgument(error.to_string()))?;
    let mut config = json!({});
    if let Some(value) = parsed.language_codes {
        config["languageCodes"] = json!(value);
    }
    if let Some(value) = parsed.custom_vocabulary {
        config["customVocabulary"] = json!(value);
    }
    if let Some(value) = parsed.word_timestamp {
        config["wordTimestamp"] = json!(value);
    }
    if let Some(value) = parsed.diarization {
        config["diarization"] = json!(value);
    }
    if let Some(value) = parsed.mode {
        config["mode"] = json!(match value {
            Mode::Smart => "SMART",
            Mode::Verbatim => "VERBATIM",
        });
    }
    Ok(config)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Word {
    word: Option<String>,
    start_offset: Option<String>,
    end_offset: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AudioTranscription {
    text: Option<String>,
    language_code: Option<String>,
    #[serde(rename = "speakerLabel")]
    _speaker_label: Option<String>,
    words: Option<Vec<Word>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Part {
    text: Option<String>,
    audio_transcription: Option<AudioTranscription>,
}
#[derive(Deserialize)]
struct Content {
    parts: Option<Vec<Part>>,
}
#[derive(Deserialize)]
struct Candidate {
    content: Option<Content>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    candidates: Option<Vec<Candidate>>,
    usage_metadata: Option<serde_json::Map<String, Value>>,
}

fn offset(value: &Option<String>) -> Option<f64> {
    // Number.parseFloat accepts a numeric prefix, including duration suffixes.
    let value = value.as_ref()?.trim_start();
    (1..=value.len())
        .rev()
        .filter(|end| value.is_char_boundary(*end))
        .find_map(|end| value[..end].parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

/// Gemini transcription model, distinct from Cloud Speech-to-Text.
pub struct VertexGeminiTranscriptionModel {
    model_id: String,
    config: EndpointConfig,
    project_location: ProjectLocationFn,
    #[cfg(feature = "realtime")]
    web_socket: Option<std::sync::Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl VertexGeminiTranscriptionModel {
    pub(crate) fn from_config(
        model_id: String,
        config: EndpointConfig,
        project_location: ProjectLocationFn,
        #[cfg(feature = "realtime")] web_socket: Option<
            std::sync::Arc<dyn aimux_provider_utils::ws::WsConnector>,
        >,
    ) -> Self {
        Self {
            model_id,
            config,
            project_location,
            #[cfg(feature = "realtime")]
            web_socket,
        }
    }
}

#[async_trait]
impl TranscriptionModel for VertexGeminiTranscriptionModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(
        &self,
        options: &TranscriptionCallOptions,
    ) -> Result<TranscriptionResult, AiMuxError> {
        if self.model_id.contains("-live") {
            return Err(AiMuxError::InvalidArgument(format!(
                "Model '{}' only supports streaming transcription. Use stream_transcribe or a unary model.",
                self.model_id
            )));
        }
        (self.project_location)().await?;
        let timestamp = chrono::Utc::now().to_rfc3339();
        let config = transcription_config(options.provider_options.as_ref())?;
        let audio = match &options.audio {
            AudioInput::Base64(value) => value.clone(),
            AudioInput::Binary(value) => {
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, value)
            }
        };
        let mut body = json!({"contents": [{"role": "user", "parts": [{"inlineData": {"mimeType": options.media_type, "data": audio}}]}]});
        if config.as_object().is_some_and(|config| !config.is_empty()) {
            body["generationConfig"] = json!({"audioTranscriptionConfig": config});
        }
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let response = aimux_provider_utils::post_json_to_api(
            exchange.request(
                exchange.url(&format!("/models/{}:generateContent", self.model_id)),
                options,
            ),
            exchange.transform_body(body),
            aimux_provider_utils::create_json_response_handler::<Response>(),
            crate::google::google_failed_response_handler(),
        )
        .await?;
        let parts = response
            .value
            .candidates
            .unwrap_or_default()
            .into_iter()
            .next()
            .and_then(|candidate| candidate.content)
            .and_then(|content| content.parts)
            .unwrap_or_default();
        let mut text: String = parts
            .iter()
            .filter_map(|part| part.text.as_deref())
            .collect();
        if text.is_empty() {
            text = parts
                .iter()
                .filter_map(|part| part.audio_transcription.as_ref()?.text.as_deref())
                .collect();
        }
        let mut language = None;
        let mut segments = Vec::new();
        for part in parts {
            let Some(transcription) = part.audio_transcription else {
                continue;
            };
            if language.is_none() {
                language = transcription.language_code;
            }
            for word in transcription.words.unwrap_or_default() {
                if let (Some(text), Some(start_second), Some(end_second)) = (
                    word.word.clone(),
                    offset(&word.start_offset),
                    offset(&word.end_offset),
                ) {
                    segments.push(TranscriptionSegment {
                        text,
                        start_second,
                        end_second,
                    });
                }
            }
        }
        Ok(TranscriptionResult {
            text,
            segments,
            language,
            duration_in_seconds: None,
            warnings: Vec::new(),
            request: None,
            response: TranscriptionResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response.response_headers),
                body: response.raw_value,
            },
            provider_metadata: response.value.usage_metadata.map(|usage| {
                provider_namespace("google", json!({"usageMetadata": usage}))
                    .expect("provider metadata must be an object")
            }),
        })
    }

    async fn do_stream(
        &self,
        options: TranscriptionStreamOptions,
    ) -> Result<TranscriptionStreamResult, AiMuxError> {
        if !self.model_id.contains("-live") {
            return Err(AiMuxError::InvalidArgument(format!(
                "Model '{}' does not support streaming transcription. Use a live model.",
                self.model_id
            )));
        }
        #[cfg(feature = "realtime")]
        {
            self.stream_live(options).await
        }
        #[cfg(not(feature = "realtime"))]
        {
            let _ = options;
            Err(AiMuxError::UnsupportedFunctionality(
                "Enable realtime for Vertex Live transcription".into(),
            ))
        }
    }
}

#[cfg(feature = "realtime")]
impl VertexGeminiTranscriptionModel {
    async fn stream_live(
        &self,
        options: TranscriptionStreamOptions,
    ) -> Result<TranscriptionStreamResult, AiMuxError> {
        use aimux_core::transcription_model::{
            AudioChunk, TranscriptionRequest, TranscriptionStreamPart,
        };
        use aimux_provider_utils::ws::{WebSocketRequest, WsMessage, ws_connect};
        use futures::StreamExt;
        use std::time::Duration;
        use tokio::time::Instant;

        if options.input_audio_format.format_type != "audio/pcm"
            || options
                .input_audio_format
                .rate
                .is_some_and(|rate| rate != 16000)
        {
            return Err(AiMuxError::InvalidArgument(
                "The Gemini Live transcription API only supports 16kHz 16-bit PCM input audio."
                    .into(),
            ));
        }
        let target = (self.project_location)().await?;
        let config = transcription_config(options.provider_options.as_ref())?;
        let setup = json!({"setup": {
            "model": format!("projects/{}/locations/{}/publishers/google/models/{}", target.project, target.location, self.model_id),
            "inputAudioTranscription": config,
        }});
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let url = format!(
            "wss://{}/ws/google.cloud.aiplatform.v1.LlmBidiService/BidiGenerateContent",
            super::location_host(&target.location)
        );
        let mut socket = ws_connect(&WebSocketRequest {
            url,
            headers: exchange.headers(),
            subprotocols: Vec::new(),
            abort_signal: options.abort_signal.clone(),
            timeout: options.timeout,
            connector: self.web_socket.clone(),
        })
        .await?;
        socket.send_text(&setup.to_string()).await?;
        let request = Some(TranscriptionRequest {
            body: Some(setup["setup"].to_string()),
        });
        let response = Some(TranscriptionResponse {
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
            model_id: Some(self.model_id.clone()),
            ..Default::default()
        });
        let include_raw = options.include_raw_chunks;
        let mut audio = options.audio;
        let stream = async_stream::stream! {
            yield Ok(TranscriptionStreamPart::StreamStart { warnings: Vec::new() });
            let mut ready = false;
            let mut audio_ended = false;
            let mut deadline = None;
            let mut segment_counter = 0;
            let mut segment_buffer = String::new();
            let mut latest_interim = String::new();
            let mut full_text = String::new();
            let mut language = None;
            let mut usage_metadata = None;
            let mut complete = false;
            loop {
                tokio::select! {
                    chunk = audio.next(), if ready && !audio_ended => {
                        let message = match chunk {
                            Some(chunk) => {
                                let data = match chunk {
                                    AudioChunk::Base64(data) => data,
                                    AudioChunk::Binary(data) => base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data),
                                };
                                json!({"realtimeInput": {"audio": {"data": data, "mimeType": "audio/pcm;rate=16000"}}})
                            }
                            None => {
                                audio_ended = true;
                                deadline = Some(Instant::now() + Duration::from_secs(3));
                                json!({"realtimeInput": {"audioStreamEnd": true}})
                            }
                        };
                        if let Err(error) = socket.send_text(&message.to_string()).await {
                            yield Err(error); break;
                        }
                    }
                    incoming = socket.next() => {
                        let incoming = match incoming {
                            Some(Ok(incoming)) => incoming,
                            Some(Err(error)) => { yield Err(error); break; }
                            None if audio_ended => { complete = true; break; }
                            None => {
                                yield Err(AiMuxError::InvalidResponseData("Vertex Live transcription WebSocket closed unexpectedly before finishing".into()));
                                break;
                            }
                        };
                        let text = match incoming {
                            WsMessage::Text(text) => text,
                            WsMessage::Binary(data) => String::from_utf8_lossy(&data).into_owned(),
                        };
                        let Ok(message) = serde_json::from_str::<Value>(&text) else { continue; };
                        if include_raw { yield Ok(TranscriptionStreamPart::Raw { raw_value: message.clone() }); }
                        if message.get("setupComplete").is_some_and(|value| !value.is_null()) { ready = true; }
                        if let Some(value) = message.get("usageMetadata").filter(|value| !value.is_null()) { usage_metadata = Some(value.clone()); }
                        if let Some(error) = message.get("error").filter(|value| !value.is_null()) {
                            yield Err(AiMuxError::InvalidResponseData(error.get("message").and_then(Value::as_str).unwrap_or("Vertex Live API error").into()));
                            break;
                        }
                        let content = message.get("serverContent");
                        if let Some(text) = content.and_then(|value| value.get("interimInputTranscription")).and_then(|value| value.get("text")).and_then(Value::as_str).filter(|text| !text.is_empty()) {
                            if audio_ended { deadline = Some(Instant::now() + Duration::from_secs(3)); }
                            latest_interim = text.to_owned();
                            yield Ok(TranscriptionStreamPart::TranscriptPartial {
                                id: Some(format!("google-segment-{segment_counter}")), text: text.into(), start_second: None, duration_in_seconds: None, channel_index: None, provider_metadata: None,
                            });
                        }
                        let transcription = content.and_then(|value| value.get("inputTranscription")).filter(|value| !value.is_null()).or_else(|| message.get("inputTranscription"));
                        if let Some(transcription) = transcription {
                            if let Some(value) = transcription.get("languageCode").and_then(Value::as_str) { language = Some(value.to_owned()); }
                            if let Some(text) = transcription.get("text").and_then(Value::as_str).filter(|text| !text.is_empty()) {
                                if audio_ended { deadline = Some(Instant::now() + Duration::from_secs(3)); }
                                latest_interim.clear();
                                segment_buffer.push_str(text);
                                yield Ok(TranscriptionStreamPart::TranscriptDelta { id: Some(format!("google-segment-{segment_counter}")), delta: text.into(), provider_metadata: None });
                            }
                        }
                        let turn_complete = content.and_then(|value| value.get("turnComplete")).and_then(Value::as_bool) == Some(true);
                        if turn_complete || transcription.and_then(|value| value.get("finished")).and_then(Value::as_bool) == Some(true) {
                            if segment_buffer.is_empty() { segment_buffer = std::mem::take(&mut latest_interim); }
                            latest_interim.clear();
                            if !segment_buffer.is_empty() {
                                if !full_text.is_empty() { full_text.push(' '); }
                                full_text.push_str(&segment_buffer);
                                yield Ok(TranscriptionStreamPart::TranscriptFinal {
                                    id: Some(format!("google-segment-{segment_counter}")), text: std::mem::take(&mut segment_buffer), start_second: None, end_second: None, channel_index: None, provider_metadata: None,
                                });
                                segment_counter += 1;
                            }
                        }
                        let status = content.and_then(|value| value.get("interactionStatus")).and_then(Value::as_str);
                        if audio_ended && (matches!(status, Some("IDLE" | "REQUIRES_ACTION")) || (turn_complete && status.is_none())) {
                            complete = true; break;
                        }
                    }
                    _ = async {
                        match deadline { Some(deadline) => tokio::time::sleep_until(deadline).await, None => std::future::pending().await }
                    } => { complete = true; break; }
                }
            }
            if complete {
                if segment_buffer.is_empty() { segment_buffer = latest_interim; }
                if !segment_buffer.is_empty() {
                    if !full_text.is_empty() { full_text.push(' '); }
                    full_text.push_str(&segment_buffer);
                    yield Ok(TranscriptionStreamPart::TranscriptFinal {
                        id: Some(format!("google-segment-{segment_counter}")), text: segment_buffer, start_second: None, end_second: None, channel_index: None, provider_metadata: None,
                    });
                }
                yield Ok(TranscriptionStreamPart::Finish {
                    text: full_text, segments: Vec::new(), language, duration_in_seconds: None,
                    provider_metadata: usage_metadata.map(|usage| provider_namespace("google", json!({"usageMetadata": usage})).expect("provider metadata must be an object")),
                });
            }
            socket.close().await;
        };
        Ok(TranscriptionStreamResult {
            stream: Box::pin(stream),
            request,
            response,
        })
    }
}
