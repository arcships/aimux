//! The ElevenLabs transcription model (`elevenlabs-transcription-model.ts`),
//! its options (`elevenlabs-transcription-model-options.ts`,
//! `elevenlabs-transcription-options.ts`) and response schema
//! (`elevenlabs-api-types.ts`).

#[cfg(feature = "realtime")]
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::{HttpBody, MultipartForm, media_type_to_extension};

use crate::shared::EndpointConfig;

use super::error::elevenlabs_failed_response_handler;
use super::options;

// ════════════════════════════════════════════════════════════════════════════
// Transcription (STT) model
// ════════════════════════════════════════════════════════════════════════════

/// `elevenLabsTranscriptionModelOptionsSchema`
/// (elevenlabs-transcription-model-options.ts). Every key is `.nullish()`:
/// `null` reads as absent, and a key left out takes the schema's default
/// (`tagAudioEvents` true, `timestampsGranularity` `word`, `diarize` false,
/// `fileFormat` `other`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ElevenLabsTranscriptionOptions {
    #[serde(default)]
    language_code: Option<String>,
    #[serde(default = "some_true")]
    tag_audio_events: Option<bool>,
    /// An integer from 1 to 32.
    #[serde(default, deserialize_with = "num_speakers")]
    num_speakers: Option<u8>,
    #[serde(default = "word_granularity")]
    timestamps_granularity: Option<TimestampsGranularity>,
    #[serde(default = "some_false")]
    diarize: Option<bool>,
    #[serde(default = "other_file_format")]
    file_format: Option<FileFormat>,
    /// The options of a realtime session; a batch call warns about them.
    #[serde(default)]
    streaming: Option<RealtimeOptions>,
}

fn some_true() -> Option<bool> {
    Some(true)
}

fn some_false() -> Option<bool> {
    Some(false)
}

fn word_granularity() -> Option<TimestampsGranularity> {
    Some(TimestampsGranularity::Word)
}

fn other_file_format() -> Option<FileFormat> {
    Some(FileFormat::Other)
}

/// `z.number().int().min(1).max(32).nullish()`.
fn num_speakers<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<u8>, D::Error> {
    match Option::<f64>::deserialize(deserializer)? {
        None => Ok(None),
        Some(value) if value.fract() == 0.0 && (1.0..=32.0).contains(&value) => {
            Ok(Some(value as u8))
        }
        Some(_) => Err(serde::de::Error::custom("expected an integer from 1 to 32")),
    }
}

/// `z.enum(['none', 'word', 'character'])`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TimestampsGranularity {
    None,
    Word,
    Character,
}

impl TimestampsGranularity {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Word => "word",
            Self::Character => "character",
        }
    }
}

/// `z.enum(['pcm_s16le_16', 'other'])`.
#[derive(Debug, Clone, Copy, Deserialize)]
enum FileFormat {
    #[serde(rename = "pcm_s16le_16")]
    PcmS16le16,
    #[serde(rename = "other")]
    Other,
}

impl FileFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::PcmS16le16 => "pcm_s16le_16",
            Self::Other => "other",
        }
    }
}

/// The `streaming` object of the schema. Only checked here: aimux's realtime
/// session reads its own two keys (RFC-0034 D5).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the realtime session reads its own keys"
)]
struct RealtimeOptions {
    #[serde(default)]
    commit_strategy: Option<CommitStrategy>,
    #[serde(default)]
    enable_logging: Option<bool>,
    #[serde(default)]
    filter_background_audio: Option<bool>,
    #[serde(default)]
    include_language_detection: Option<bool>,
    #[serde(default)]
    include_timestamps: Option<bool>,
    /// At most 50, each at most 20 characters.
    #[serde(default, deserialize_with = "keyterms")]
    keyterms: Option<Vec<String>>,
    /// An integer from 50 to 2000.
    #[serde(default, deserialize_with = "duration_ms")]
    min_silence_duration_ms: Option<f64>,
    /// An integer from 50 to 2000.
    #[serde(default, deserialize_with = "duration_ms")]
    min_speech_duration_ms: Option<f64>,
    #[serde(default)]
    no_verbatim: Option<bool>,
    #[serde(default)]
    previous_text: Option<String>,
    #[serde(default)]
    secondary_languages: Option<Vec<String>>,
    /// 0.3 to 3.
    #[serde(default, deserialize_with = "vad_silence_threshold_secs")]
    vad_silence_threshold_secs: Option<f64>,
    /// 0.1 to 0.9.
    #[serde(default, deserialize_with = "vad_threshold")]
    vad_threshold: Option<f64>,
}

/// `z.enum(['manual', 'vad'])`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CommitStrategy {
    Manual,
    Vad,
}

/// A `.nullish()` number in `range` (an integer when `integer`).
fn nullish_number_in<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
    range: std::ops::RangeInclusive<f64>,
    integer: bool,
    expected: &'static str,
) -> Result<Option<f64>, D::Error> {
    match Option::<f64>::deserialize(deserializer)? {
        Some(value) if !range.contains(&value) || (integer && value.fract() != 0.0) => {
            Err(serde::de::Error::custom(expected))
        }
        value => Ok(value),
    }
}

fn duration_ms<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
    nullish_number_in(
        deserializer,
        50.0..=2000.0,
        true,
        "expected an integer from 50 to 2000",
    )
}

fn vad_silence_threshold_secs<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    nullish_number_in(
        deserializer,
        0.3..=3.0,
        false,
        "expected a number from 0.3 to 3",
    )
}

fn vad_threshold<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    nullish_number_in(
        deserializer,
        0.1..=0.9,
        false,
        "expected a number from 0.1 to 0.9",
    )
}

/// `z.array(z.string().max(20)).max(50).nullish()`.
fn keyterms<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error> {
    let keyterms = Option::<Vec<String>>::deserialize(deserializer)?;
    if keyterms
        .as_ref()
        .is_some_and(|terms| terms.len() > 50 || terms.iter().any(|term| term.chars().count() > 20))
    {
        return Err(serde::de::Error::custom(
            "expected at most 50 keyterms of at most 20 characters",
        ));
    }
    Ok(keyterms)
}

/// `parseProviderOptions({ provider: 'elevenlabs', providerOptions, schema:
/// elevenLabsTranscriptionModelOptionsSchema })`
/// (elevenlabs-transcription-model.ts getArgs); `None` without an
/// `elevenlabs` namespace.
fn parse_transcription_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<Option<ElevenLabsTranscriptionOptions>, AiMuxError> {
    aimux_provider_utils::parse_provider_options(options::NAMESPACE, provider_options)
}

/// ElevenLabs transcription response word.
#[derive(Debug, Deserialize)]
struct ElevenLabsTranscriptionWord {
    text: String,
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
}

/// ElevenLabs transcription API response body.
#[derive(Debug, Deserialize)]
struct ElevenLabsTranscriptionResponse {
    language_code: String,
    #[allow(dead_code)]
    language_probability: f64,
    text: String,
    #[serde(default)]
    words: Option<Vec<ElevenLabsTranscriptionWord>>,
}

/// ElevenLabs transcription (STT) model — implements `TranscriptionModel`.
///
/// Aligned with Vercel AI SDK `ElevenLabsTranscriptionModel`
/// (`reference/ai/packages/elevenlabs/src/elevenlabs-transcription-model.ts`).
///
/// Endpoint: `POST {base_url}/v1/speech-to-text` (multipart form-data)
pub struct ElevenLabsTranscriptionModel {
    model_id: String,
    config: EndpointConfig,
    #[cfg(feature = "realtime")]
    web_socket: Option<Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl ElevenLabsTranscriptionModel {
    pub(crate) fn from_config(
        model_id: String,
        config: EndpointConfig,
        #[cfg(feature = "realtime")] web_socket: Option<
            Arc<dyn aimux_provider_utils::ws::WsConnector>,
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

fn audio_input_to_bytes_stt(audio: &AudioInput) -> Result<Vec<u8>, AiMuxError> {
    match audio {
        AudioInput::Binary(bytes) => Ok(bytes.clone()),
        AudioInput::Base64(b64) => aimux_provider_utils::convert_base64_to_uint8_array(b64)
            .map_err(|e| AiMuxError::InvalidArgument {
                message: format!("invalid base64: {e}"),
                cause: Some(ErrorCause::source(&e)),
            }),
        AudioInput::Url(_) => Err(AudioInput::url_error()),
    }
}

/// Realtime streaming model IDs (`scribe_v2_realtime*`) use the WebSocket
/// realtime endpoint; every other model uses the batch REST endpoint.
/// Wire shape per the public API reference (2026-09, RFC-0034 §3).
fn is_realtime_transcription_model_id(model_id: &str) -> bool {
    model_id == "scribe_v2_realtime" || model_id.starts_with("scribe_v2_realtime-")
}

#[async_trait]
impl TranscriptionModel for ElevenLabsTranscriptionModel {
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
        if is_realtime_transcription_model_id(&self.model_id) {
            return Err(AiMuxError::UnsupportedFunctionality(format!(
                "non-streaming transcription is not supported by `{}` \
                 (realtime models stream over a WebSocket session)",
                self.model_id
            )));
        }

        let mut warnings: Vec<Warning> = Vec::new();

        let audio_bytes = audio_input_to_bytes_stt(&options.audio)?;
        let file_extension = media_type_to_extension(&options.media_type);
        let filename = format!("audio.{file_extension}");

        let mut form = MultipartForm::new();
        form.text("model_id", &self.model_id)?;
        form.file("file", &filename, &options.media_type, &audio_bytes)?;
        form.text("diarize", "true")?;

        // elevenlabs-transcription-model.ts getArgs: the schema's defaults
        // apply when the namespace is present.
        if let Some(el) = parse_transcription_options(options.provider_options.as_ref())? {
            if el.streaming.is_some() {
                warnings.push(Warning::Unsupported {
                    feature: "providerOptions.elevenlabs.streaming".to_string(),
                    details: Some(
                        "ElevenLabs batch transcription does not support streaming options."
                            .to_string(),
                    ),
                });
            }
            if let Some(v) = el.diarize {
                form.text("diarize", &v.to_string())?;
            }
            if let Some(v) = &el.language_code {
                form.text("language_code", v)?;
            }
            if let Some(v) = el.tag_audio_events {
                form.text("tag_audio_events", &v.to_string())?;
            }
            if let Some(v) = el.num_speakers {
                form.text("num_speakers", &v.to_string())?;
            }
            if let Some(v) = el.timestamps_granularity {
                form.text("timestamps_granularity", v.as_str())?;
            }
            if let Some(v) = el.file_format {
                form.text("file_format", v.as_str())?;
            }
        }

        let (body_bytes, content_type) = form.finish();

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_to_api(
            exchange.request(exchange.url("/v1/speech-to-text"), options),
            HttpBody::Bytes(body_bytes, content_type),
            aimux_provider_utils::create_json_response_handler::<ElevenLabsTranscriptionResponse>(),
            elevenlabs_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let parsed = resp.value;

        let segments: Vec<TranscriptionSegment> = parsed
            .words
            .as_ref()
            .map(|words| {
                words
                    .iter()
                    .map(|w| TranscriptionSegment {
                        text: w.text.clone(),
                        start_second: w.start.unwrap_or(0.0),
                        end_second: w.end.unwrap_or(0.0),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let duration_in_seconds = parsed
            .words
            .as_ref()
            .and_then(|w| w.last())
            .and_then(|w| w.end);

        let timestamp = aimux_core::Timestamp::now();

        Ok(TranscriptionResult {
            text: parsed.text,
            segments,
            language: Some(parsed.language_code),
            duration_in_seconds,
            warnings,
            request: Some(TranscriptionRequest { body: None }),
            response: TranscriptionResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
                body: Some(raw_body),
            },
            provider_metadata: None,
        })
    }

    /// The model has a streaming method; the session itself is
    /// `ElevenLabsTranscriptionModel::stream_transcription`.
    fn do_stream(
        &self,
        options: aimux_core::transcription_model::TranscriptionStreamOptions,
    ) -> Option<
        futures::future::BoxFuture<
            '_,
            Result<aimux_core::transcription_model::TranscriptionStreamResult, AiMuxError>,
        >,
    > {
        Some(Box::pin(self.stream_transcription(options)))
    }
}

impl ElevenLabsTranscriptionModel {
    /// Streaming transcription over the ElevenLabs realtime WebSocket
    /// (RFC-0034 §3, wire shape per the public API reference 2026-09).
    ///
    /// Config travels on the URL (no `session.update` message); audio rides
    /// base64 in `input_audio_chunk` JSON frames; `commit_strategy` is fixed
    /// to `manual` (D3) so the stream ends when the caller's audio ends —
    /// the final chunk carries `commit: true`, the resulting
    /// `committed_transcript` is THE final, then the client closes (1000).
    /// A settle window (`chunk_ms`, default 5s) bounds the wait for that
    /// final event so the stream always terminates.
    ///
    /// Live-API smoke pending (no key at implementation time); the wire
    /// shape is pinned field-by-field against the documented reference by
    /// the local mock-server tests — same posture as RFC-0028 D4.
    #[cfg(feature = "realtime")]
    async fn stream_transcription(
        &self,
        options: aimux_core::transcription_model::TranscriptionStreamOptions,
    ) -> Result<aimux_core::transcription_model::TranscriptionStreamResult, AiMuxError> {
        use aimux_core::transcription_model::{TranscriptionStreamPart, TranscriptionStreamResult};
        use aimux_provider_utils::ws::{WebSocketRequest, WsMessage, ws_connect};
        use futures::StreamExt;

        if !is_realtime_transcription_model_id(&self.model_id) {
            return Err(AiMuxError::UnsupportedFunctionality(format!(
                "streaming transcription is not supported by `{}` \
                 (realtime models such as scribe_v2_realtime only)",
                self.model_id
            )));
        }

        // Parameter surface is deliberately minimal (RFC-0034 D5):
        // languageCode + includeTimestamps. Anything else waits for a user.
        let mut language_code: Option<String> = None;
        let mut include_timestamps = false;
        if let Some(el) = options::elevenlabs_options(options.provider_options.as_ref()) {
            if let Some(v) = el.get("languageCode").and_then(serde_json::Value::as_str) {
                language_code = Some(v.to_string());
            }
            if let Some(v) = el
                .get("includeTimestamps")
                .and_then(serde_json::Value::as_bool)
            {
                include_timestamps = v;
            }
        }

        // Audio format: the realtime endpoint takes pcm_{rate} / ulaw_{rate}.
        let default_rate = if options.input_audio_format.format_type == "audio/pcmu" {
            8_000
        } else {
            16_000
        };
        let sample_rate = options.input_audio_format.rate.unwrap_or(default_rate);
        let audio_format = match options.input_audio_format.format_type.as_str() {
            "audio/pcm" => format!("pcm_{sample_rate}"),
            "audio/pcmu" => format!("ulaw_{sample_rate}"),
            other => {
                return Err(AiMuxError::UnsupportedFunctionality(format!(
                    "ElevenLabs realtime accepts audio/pcm or audio/pcmu input, got {other}"
                )));
            }
        };

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let base = exchange.base_url();
        let (scheme, host) = if let Some(rest) = base.strip_prefix("https://") {
            ("wss", rest)
        } else if let Some(rest) = base.strip_prefix("http://") {
            ("ws", rest)
        } else {
            ("wss", base)
        };
        // Scoped: the serializer is not `Send`; keep it inside this block so
        // nothing non-Send is alive across the connect await below.
        let ws_url = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query
                .append_pair("model_id", &self.model_id)
                .append_pair("audio_format", &audio_format)
                .append_pair("commit_strategy", "manual");
            if let Some(code) = &language_code {
                query.append_pair("language_code", code);
            }
            if include_timestamps {
                query.append_pair("include_timestamps", "true");
            }
            format!(
                "{scheme}://{host}/v1/speech-to-text/realtime?{}",
                query.finish()
            )
        };

        let header_list = exchange.headers();

        // Connect BEFORE the stream so connect failures surface from
        // do_stream's Result (same contract as the OpenAI realtime path).
        let req = WebSocketRequest {
            url: ws_url.clone(),
            headers: header_list,
            subprotocols: Vec::new(),
            abort_signal: options.abort_signal.clone(),
            timeout: options.timeout,
            connector: self.web_socket.clone(),
        };
        let mut ws = ws_connect(&req).await?;

        // Settle window for the post-commit final event: the server does not
        // close the session (RFC-0034 §3.3), so an empty Finish must be
        // possible. chunk_ms when configured, otherwise 5s.
        let settle = options
            .timeout
            .as_ref()
            .and_then(|t| t.chunk_ms)
            .map(tokio::time::Duration::from_millis)
            .unwrap_or(tokio::time::Duration::from_secs(5));

        let include_raw = options.include_raw_chunks;
        let model_id = self.model_id.clone();
        let error_url = ws_url.clone();
        let mut audio = options.audio;

        let stream = async_stream::stream! {
            // One held chunk: the commit flag must ride the LAST real audio
            // chunk, so each incoming chunk flushes the previous one and the
            // stream-end flushes the held one with commit=true. An
            // empty-audio stream commits via a single empty chunk.
            let mut held: Option<String> = None;
            let mut audio_done = false;
            let mut commit_deadline: Option<tokio::time::Instant> = None;
            // Audio is not sent before the session is established: the
            // server confirms configuration with session_started first.
            let mut session_started = false;

            loop {
                let audio_next = async {
                    if audio_done || !session_started {
                        std::future::pending::<()>().await;
                        None
                    } else {
                        audio.next().await
                    }
                };

                tokio::select! {
                    biased;

                    chunk = audio_next => {
                        // Hold-one-chunk pipeline: the commit flag must ride
                        // the LAST real chunk. A new chunk flushes the
                        // previously held one (commit=false); the stream end
                        // flushes the held one with commit=true; an
                        // empty-audio stream commits a single empty chunk.
                        let (payload, commit) = match chunk {
                            None => {
                                audio_done = true;
                                commit_deadline = Some(tokio::time::Instant::now() + settle);
                                (held.take().unwrap_or_default(), true)
                            }
                            Some(aimux_core::transcription_model::AudioChunk::Binary(bytes)) => {
                                let b64 = aimux_provider_utils::convert_uint8_array_to_base64(bytes);
                                match held.replace(b64) {
                                    Some(previous) => (previous, false),
                                    // First chunk: hold it, nothing to send yet.
                                    None => continue,
                                }
                            }
                            Some(aimux_core::transcription_model::AudioChunk::Base64(b64)) => {
                                match held.replace(b64) {
                                    Some(previous) => (previous, false),
                                    None => continue,
                                }
                            }
                        };
                        let message = serde_json::json!({
                            "message_type": "input_audio_chunk",
                            "audio_base_64": payload,
                            "commit": commit,
                            "sample_rate": sample_rate,
                        });
                        if let Err(e) = ws.send_text(&message.to_string()).await {
                            yield Err(e);
                            break;
                        }
                    }

                    _ = async {
                        match commit_deadline {
                            Some(d) => tokio::time::sleep_until(d).await,
                            None => std::future::pending::<()>().await,
                        }
                    } => {
                        // Server went silent after commit: an empty Finish
                        // beats hanging (RFC-0034 §3.3.3).
                        yield Ok(TranscriptionStreamPart::Finish {
                            text: String::new(),
                            segments: vec![],
                            language: language_code.clone(),
                            duration_in_seconds: None,
                            provider_metadata: None,
                        });
                        ws.close().await;
                        break;
                    }

                    event = ws.next() => {
                        match event {
                            None => {
                                yield Err(AiMuxError::ApiCall(Box::new(
                                    aimux_core::error::ApiCallError::new(
                                        "realtime transcription socket closed before the committed transcript",
                                        error_url.clone(),
                                        serde_json::json!({}),
                                    )
                                    .with_origin(aimux_core::ApiCallErrorOrigin::Transport),
                                )));
                                break;
                            }
                            Some(Err(e)) => {
                                yield Err(e);
                                break;
                            }
                            Some(Ok(WsMessage::Binary(_))) => {}
                            Some(Ok(WsMessage::Text(text))) => {
                                let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                                    continue;
                                };
                                if include_raw {
                                    yield Ok(TranscriptionStreamPart::Raw {
                                        raw_value: value.clone(),
                                    });
                                }
                                let event_type = value.get("message_type")
                                    .and_then(|t| t.as_str()).unwrap_or("");
                                match event_type {
                                    "session_started" => {
                                        session_started = true;
                                        yield Ok(TranscriptionStreamPart::StreamStart {
                                            warnings: vec![],
                                        });
                                    }
                                    "partial_transcript" => {
                                        yield Ok(TranscriptionStreamPart::TranscriptPartial {
                                            id: None,
                                            text: value.get("text")
                                                .and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                            start_second: None,
                                            duration_in_seconds: None,
                                            channel_index: None,
                                            provider_metadata: None,
                                        });
                                    }
                                    // include_timestamps=true swaps the event
                                    // shape; both carry `text`, the timestamps
                                    // variant adds `words[]`.
                                    "committed_transcript"
                                    | "committed_transcript_with_timestamps" => {
                                        let text = value.get("text")
                                            .and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let segments = value.get("words")
                                            .and_then(|v| v.as_array())
                                            .map(|words| {
                                                words.iter().filter_map(|w| {
                                                    let word = w.get("text")
                                                        .and_then(|v| v.as_str())?;
                                                    // Spacing-type entries are
                                                    // layout, not content.
                                                    if word.trim().is_empty() {
                                                        return None;
                                                    }
                                                    Some(TranscriptionSegment {
                                                        text: word.to_string(),
                                                        start_second: w.get("start")
                                                            .and_then(serde_json::Value::as_f64).unwrap_or(0.0),
                                                        end_second: w.get("end")
                                                            .and_then(serde_json::Value::as_f64).unwrap_or(0.0),
                                                    })
                                                }).collect::<Vec<_>>()
                                            })
                                            .unwrap_or_default();
                                        let duration = segments.last().map(|s| s.end_second);
                                        yield Ok(TranscriptionStreamPart::TranscriptFinal {
                                            id: None,
                                            text: text.clone(),
                                            start_second: segments.first()
                                                .map(|s| s.start_second),
                                            end_second: segments.last()
                                                .map(|s| s.end_second),
                                            channel_index: None,
                                            provider_metadata: None,
                                        });
                                        // Manual strategy commits exactly once
                                        // (on our final flag): this committed
                                        // transcript IS the finish.
                                        yield Ok(TranscriptionStreamPart::Finish {
                                            text,
                                            segments,
                                            language: language_code.clone(),
                                            duration_in_seconds: duration,
                                            provider_metadata: None,
                                        });
                                        ws.close().await;
                                        break;
                                    }
                                    "warning" => {
                                        // No part mapping (StreamStart already
                                        // went out); visible via Raw when
                                        // include_raw_chunks is set.
                                    }
                                    other if matches!(
                                        other,
                                        "error" | "auth_error" | "quota_exceeded"
                                        | "commit_throttled" | "unaccepted_terms"
                                        | "rate_limited" | "queue_overflow"
                                        | "resource_exhausted"
                                        | "session_time_limit_exceeded"
                                        | "input_error" | "invalid_request"
                                        | "chunk_size_exceeded"
                                        | "insufficient_audio_activity"
                                        | "transcriber_error"
                                    ) => {
                                        let message = value.get("error")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("realtime transcription error");
                                        // Retryable classification (RFC-0034
                                        // §3.2): three transient names, the
                                        // rest are terminal verdicts.
                                        let is_retryable = matches!(
                                            other,
                                            "rate_limited" | "queue_overflow"
                                            | "resource_exhausted"
                                        );
                                        yield Err(AiMuxError::ApiCall(Box::new(
                                            aimux_core::error::ApiCallError {
                                                is_retryable,
                                                response_body: Some(value.to_string()),
                                                origin: aimux_core::ApiCallErrorOrigin::StreamEvent,
                                                ..aimux_core::error::ApiCallError::new(
                                                    format!("elevenlabs realtime: {message}"),
                                                    error_url.clone(),
                                                    serde_json::json!({}),
                                                )
                                            },
                                        )));
                                        ws.close().await;
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        };

        Ok(TranscriptionStreamResult {
            stream: Box::pin(stream),
            request: Some(TranscriptionRequest { body: Some(ws_url) }),
            response: Some(TranscriptionResponse {
                timestamp: Some(aimux_core::Timestamp::now()),
                model_id: Some(model_id),
                headers: None,
                body: None,
            }),
        })
    }

    /// Without the `realtime` feature the WebSocket path is compiled out.
    #[cfg(not(feature = "realtime"))]
    async fn stream_transcription(
        &self,
        _options: aimux_core::transcription_model::TranscriptionStreamOptions,
    ) -> Result<aimux_core::transcription_model::TranscriptionStreamResult, AiMuxError> {
        Err(AiMuxError::UnsupportedFunctionality(format!(
            "streaming transcription with `{}` requires building aimux-providers \
             with the `realtime` feature",
            self.model_id
        )))
    }
}
