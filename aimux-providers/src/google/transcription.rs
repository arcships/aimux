//! Google Gemini transcription (speech-to-text) model — implements
//! `TranscriptionModel`.
//!
//! Aligned with the AI SDK's `GoogleTranscriptionModel`
//! (`google/src/transcription/google-transcription-model.ts`).
//!
//! Unary models (`gemini-3.5-transcribe`) are served by the Interactions API
//! (`POST {base_url}/interactions`); live models (`gemini-3.5-transcribe-live`,
//! any id containing `-live`) stream over the Gemini Live WebSocket. Vertex
//! reuses the options and the live stream loop below.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::options::{google_metadata, no_null};
use crate::shared::EndpointConfig;
use aimux_core::error::AiMuxError;
use aimux_core::shared::JsonObject;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionResponse,
    TranscriptionResult, TranscriptionSegment, TranscriptionStreamOptions,
    TranscriptionStreamResult,
};

const LIVE_WEB_SOCKET_PATH: &str =
    "google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

// ── Provider options (`google-transcription-model-options.ts`) ──────────────

#[derive(Deserialize)]
enum Mode {
    #[serde(rename = "SMART")]
    Smart,
    #[serde(rename = "VERBATIM")]
    Verbatim,
}

impl Mode {
    fn name(&self) -> &'static str {
        match self {
            Self::Smart => "SMART",
            Self::Verbatim => "VERBATIM",
        }
    }
}

/// Speech recognition options shared by unary and live transcription.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TranscriptionOptions {
    #[serde(default, deserialize_with = "no_null")]
    language_codes: Option<Vec<String>>,
    #[serde(default, deserialize_with = "no_null")]
    custom_vocabulary: Option<Vec<String>>,
    #[serde(default, deserialize_with = "no_null")]
    word_timestamp: Option<bool>,
    #[serde(default, deserialize_with = "no_null")]
    diarization: Option<bool>,
    #[serde(default, deserialize_with = "no_null")]
    mode: Option<Mode>,
}

impl TranscriptionOptions {
    pub(crate) fn parse(raw: Option<&JsonObject>) -> Result<Self, AiMuxError> {
        match raw {
            None => Ok(Self::default()),
            Some(raw) => serde_json::from_value(Value::Object(raw.clone())).map_err(|error| {
                AiMuxError::InvalidArgument(format!(
                    "Invalid argument for parameter providerOptions: {error}"
                ))
            }),
        }
    }

    /// Google's `AudioTranscriptionConfig` (the live setup).
    pub(crate) fn audio_transcription_config(&self) -> Value {
        let mut config = json!({});
        if let Some(value) = &self.language_codes {
            config["languageCodes"] = json!(value);
        }
        if let Some(value) = &self.custom_vocabulary {
            config["customVocabulary"] = json!(value);
        }
        if let Some(value) = self.word_timestamp {
            config["wordTimestamp"] = json!(value);
        }
        if let Some(value) = self.diarization {
            config["diarization"] = json!(value);
        }
        if let Some(value) = &self.mode {
            config["mode"] = json!(value.name());
        }
        config
    }

    /// The Interactions API `transcription_config` (snake_case wire).
    /// Diarization and word timestamps are expressed inside the `mode` object.
    fn interactions_config(&self) -> Option<Value> {
        let mut config = Map::new();
        if let Some(value) = &self.language_codes {
            config.insert("language_codes".into(), json!(value));
        }
        if let Some(value) = &self.custom_vocabulary {
            config.insert("custom_vocabulary".into(), json!(value));
        }
        let diarization = self.diarization == Some(true);
        let word_timestamp = self.word_timestamp == Some(true);
        if self.mode.is_some() || diarization || word_timestamp {
            let mode = self.mode.as_ref().map_or("VERBATIM", Mode::name);
            let mut mode = json!({"type": mode.to_lowercase()});
            if diarization {
                mode["diarization_mode"] = json!("speaker");
            }
            if word_timestamp {
                mode["timestamp_granularities"] = json!(["word"]);
            }
            config.insert("mode".into(), mode);
        }
        (!config.is_empty()).then_some(Value::Object(config))
    }
}

/// Parses a Google duration offset such as `"1s"` or `"9.400s"` to seconds,
/// like `Number.parseFloat`: the longest numeric prefix counts.
pub(crate) fn parse_offset_seconds(offset: Option<&str>) -> Option<f64> {
    let value = offset?.trim_start();
    (1..=value.len())
        .rev()
        .filter(|end| value.is_char_boundary(*end))
        .find_map(|end| value[..end].parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

// ── Interactions API response ───────────────────────────────────────────────

#[derive(Deserialize)]
struct WordAnnotation {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
    start_offset: Option<String>,
    end_offset: Option<String>,
}

#[derive(Deserialize)]
struct StepContent {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
    annotations: Option<Vec<WordAnnotation>>,
}

#[derive(Deserialize)]
struct Step {
    content: Option<Vec<StepContent>>,
}

#[derive(Deserialize)]
struct InteractionsResponse {
    steps: Option<Vec<Step>>,
    usage: Option<Map<String, Value>>,
}

// ── Model ───────────────────────────────────────────────────────────────────

/// Live transcription is only supported by `*-live` model variants.
fn is_live(model_id: &str) -> bool {
    model_id.contains("-live")
}

/// A Google Gemini transcription model.
pub struct GoogleTranscriptionModel {
    model_id: String,
    config: EndpointConfig,
    #[cfg(feature = "realtime")]
    web_socket: Option<std::sync::Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl GoogleTranscriptionModel {
    pub(crate) fn from_config(
        model_id: String,
        config: EndpointConfig,
        #[cfg(feature = "realtime")] web_socket: Option<
            std::sync::Arc<dyn aimux_provider_utils::ws::WsConnector>,
        >,
    ) -> Self {
        Self {
            model_id,
            config,
            #[cfg(feature = "realtime")]
            web_socket,
        }
    }
}

#[async_trait]
impl TranscriptionModel for GoogleTranscriptionModel {
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
        if is_live(&self.model_id) {
            return Err(AiMuxError::InvalidArgument(format!(
                "Model '{}' only supports streaming transcription. Use stream_transcribe, \
                 or a unary model such as 'gemini-3.5-transcribe'.",
                self.model_id
            )));
        }
        let timestamp = chrono::Utc::now().to_rfc3339();
        let parsed = TranscriptionOptions::parse(super::options::google_options(
            options.provider_options.as_ref(),
        ))?;
        let audio = match &options.audio {
            AudioInput::Base64(value) => value.clone(),
            AudioInput::Binary(value) => {
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, value)
            }
        };

        // Unary transcription is served by the Interactions API.
        let mut body = json!({
            "model": self.model_id,
            "input": [{"type": "audio", "data": audio, "mime_type": options.media_type}],
        });
        if let Some(config) = parsed.interactions_config() {
            body["generation_config"] = json!({"transcription_config": config});
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let response = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/interactions"), options),
            body,
            aimux_provider_utils::create_json_response_handler::<InteractionsResponse>(),
            super::google_failed_response_handler(),
        )
        .await?;

        let mut text = String::new();
        let mut segments = Vec::new();
        for content in response
            .value
            .steps
            .into_iter()
            .flatten()
            .flat_map(|step| step.content.into_iter().flatten())
        {
            let (Some("text"), Some(content_text)) = (content.kind.as_deref(), content.text) else {
                continue;
            };
            text.push_str(&content_text);
            for annotation in content.annotations.into_iter().flatten() {
                if annotation.kind.as_deref() != Some("word_info") {
                    continue;
                }
                if let (Some(text), Some(start_second), Some(end_second)) = (
                    annotation.text,
                    parse_offset_seconds(annotation.start_offset.as_deref()),
                    parse_offset_seconds(annotation.end_offset.as_deref()),
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
            language: None,
            duration_in_seconds: None,
            warnings: Vec::new(),
            request: None,
            response: TranscriptionResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response.response_headers),
                body: response.raw_value,
            },
            provider_metadata: response
                .value
                .usage
                .map(|usage| google_metadata(json!({"usage": usage}))),
        })
    }

    async fn do_stream(
        &self,
        options: TranscriptionStreamOptions,
    ) -> Result<TranscriptionStreamResult, AiMuxError> {
        if !is_live(&self.model_id) {
            return Err(AiMuxError::InvalidArgument(format!(
                "Model '{}' does not support streaming transcription. \
                 Use a live model such as 'gemini-3.5-transcribe-live'.",
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
                "Enable realtime for Google Live transcription".into(),
            ))
        }
    }
}

#[cfg(feature = "realtime")]
impl GoogleTranscriptionModel {
    async fn stream_live(
        &self,
        options: TranscriptionStreamOptions,
    ) -> Result<TranscriptionStreamResult, AiMuxError> {
        use aimux_core::transcription_model::TranscriptionRequest;
        use aimux_provider_utils::ws::{WebSocketRequest, ws_connect};

        let parsed = TranscriptionOptions::parse(super::options::google_options(
            options.provider_options.as_ref(),
        ))?;
        validate_live_input_audio_format(&options)?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        // The key travels in the URL, not in a header.
        let mut api_key = None;
        let mut headers = Vec::new();
        for (name, value) in exchange.headers() {
            if name.eq_ignore_ascii_case("x-goog-api-key") {
                api_key = Some(value);
            } else {
                headers.push((name, value));
            }
        }
        let api_key = api_key.ok_or_else(|| AiMuxError::LoadApiKey {
            env_var: "GOOGLE_GENERATIVE_AI_API_KEY".into(),
            description: "Google Generative AI (streaming transcription)".into(),
        })?;
        let mut url = live_web_socket_url(exchange.base_url(), LIVE_WEB_SOCKET_PATH)?;
        url.query_pairs_mut().append_pair("key", &api_key);

        // Google's GA announcement shows the setup with
        // `generationConfig: { responseModalities: ['TEXT'] }`, but sending it
        // suppresses the final `inputTranscription` segments on the current
        // endpoint: omit generationConfig.
        let model_path = if self.model_id.contains('/') {
            self.model_id.clone()
        } else {
            format!("models/{}", self.model_id)
        };
        let setup = json!({
            "model": model_path,
            "inputAudioTranscription": parsed.audio_transcription_config(),
        });

        let mut socket = ws_connect(&WebSocketRequest::for_transcription(
            url.to_string(),
            headers,
            &options,
            self.web_socket.clone(),
        ))
        .await?;
        socket
            .send_text(&json!({"setup": setup}).to_string())
            .await?;
        Ok(TranscriptionStreamResult {
            stream: live_stream(socket, options.audio, options.include_raw_chunks, "Google"),
            request: Some(TranscriptionRequest {
                body: Some(setup.to_string()),
            }),
            response: Some(TranscriptionResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                ..Default::default()
            }),
        })
    }
}

/// `getRealtimeWebSocketURL`: the Live API paths carry their own version, so a
/// trailing `/v1beta` or `/v1alpha` of the base URL is dropped.
#[cfg(feature = "realtime")]
fn live_web_socket_url(base_url: &str, path: &str) -> Result<url::Url, AiMuxError> {
    let mut url = url::Url::parse(base_url)
        .map_err(|error| AiMuxError::InvalidArgument(format!("invalid base URL: {error}")))?;
    let mut segments: Vec<&str> = url.path().split('/').collect();
    if matches!(segments.last(), Some(&("v1beta" | "v1alpha"))) {
        segments.pop();
    }
    let base_path = segments.join("/");
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_path(&format!("{}/ws/{path}", base_path.trim_end_matches('/')));
    url.set_scheme(scheme)
        .map_err(|()| AiMuxError::InvalidArgument("invalid base URL scheme".into()))?;
    Ok(url)
}

#[cfg(feature = "realtime")]
pub(crate) fn validate_live_input_audio_format(
    options: &TranscriptionStreamOptions,
) -> Result<(), AiMuxError> {
    let format = &options.input_audio_format;
    if format.format_type != "audio/pcm" || format.rate.is_some_and(|rate| rate != 16000) {
        return Err(AiMuxError::InvalidArgument(
            "The Gemini Live transcription API only supports 16kHz 16-bit PCM input audio.".into(),
        ));
    }
    Ok(())
}

/// The Gemini Live transcription session, shared with Vertex: waits for
/// `setupComplete`, sends the audio, accumulates transcript segments and
/// finishes on the idle signal, a server close after the audio ended, or after
/// a quiet grace window. `label` names the service in error messages.
#[cfg(feature = "realtime")]
pub(crate) fn live_stream(
    mut socket: aimux_provider_utils::ws::WsConnection,
    audio: std::pin::Pin<
        Box<dyn futures::Stream<Item = aimux_core::transcription_model::AudioChunk> + Send>,
    >,
    include_raw: bool,
    label: &'static str,
) -> std::pin::Pin<
    Box<
        dyn futures::Stream<
                Item = Result<aimux_core::transcription_model::TranscriptionStreamPart, AiMuxError>,
            > + Send,
    >,
> {
    use aimux_core::transcription_model::{AudioChunk, TranscriptionStreamPart};
    use aimux_provider_utils::ws::WsMessage;
    use futures::StreamExt;
    use std::time::Duration;
    use tokio::time::Instant;

    /// After the input audio has ended, finish when no terminal signal arrives
    /// within this window. Trailing transcripts reset the timer.
    const FINISH_GRACE: Duration = Duration::from_secs(3);

    let mut audio = audio;
    Box::pin(async_stream::stream! {
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
                            deadline = Some(Instant::now() + FINISH_GRACE);
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
                        // A close frame after the input audio ended means the server
                        // delivered everything it will deliver.
                        Some(Err(AiMuxError::ApiCall(error))) if audio_ended && error.message.starts_with("websocket closed by peer") => { complete = true; break; }
                        Some(Err(error)) => { yield Err(error); break; }
                        None if audio_ended => { complete = true; break; }
                        None => {
                            yield Err(AiMuxError::InvalidResponseData(format!("{label} Live transcription WebSocket closed unexpectedly before finishing")));
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
                        yield Err(AiMuxError::InvalidResponseData(error.get("message").and_then(Value::as_str).map_or_else(|| format!("{label} Live API error"), str::to_owned)));
                        break;
                    }
                    let content = message.get("serverContent");
                    if let Some(text) = content.and_then(|value| value.get("interimInputTranscription")).and_then(|value| value.get("text")).and_then(Value::as_str).filter(|text| !text.is_empty()) {
                        if audio_ended { deadline = Some(Instant::now() + FINISH_GRACE); }
                        latest_interim = text.to_owned();
                        yield Ok(TranscriptionStreamPart::TranscriptPartial {
                            id: Some(format!("google-segment-{segment_counter}")), text: text.into(), start_second: None, duration_in_seconds: None, channel_index: None, provider_metadata: None,
                        });
                    }
                    let transcription = content.and_then(|value| value.get("inputTranscription")).filter(|value| !value.is_null()).or_else(|| message.get("inputTranscription"));
                    if let Some(transcription) = transcription {
                        if let Some(value) = transcription.get("languageCode").and_then(Value::as_str) { language = Some(value.to_owned()); }
                        if let Some(text) = transcription.get("text").and_then(Value::as_str).filter(|text| !text.is_empty()) {
                            if audio_ended { deadline = Some(Instant::now() + FINISH_GRACE); }
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
                    // `interactionStatus` idle (REQUIRES_ACTION in the EAP builds)
                    // is the definitive all-processing-complete signal.
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
                provider_metadata: usage_metadata.map(|usage| google_metadata(json!({"usageMetadata": usage}))),
            });
        }
        socket.close().await;
    })
}
