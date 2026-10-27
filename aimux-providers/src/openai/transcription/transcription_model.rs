//! OpenAI transcription (STT) model — implements the `TranscriptionModel` trait.
//!
//! Aligned with Vercel AI SDK `OpenAITranscriptionModel`
//! (`reference/ai/packages/openai/src/transcription/openai-transcription-model.ts`).
//!
//! Endpoint: `POST {base_url}/audio/transcriptions` (multipart form-data)
//!
//! The OpenAI transcription API accepts `model`, `file`, `response_format`,
//! `language`, `prompt`, `temperature`, and `timestamp_granularities` as
//! multipart form fields and returns a JSON body with `text`, optional
//! `segments`, `words`, `language`, and `duration`.
//!
//! Realtime transcription models (`gpt-realtime-whisper*`) stream over
//! WebSocket (`wss://{base}/realtime?intent=transcription`) and do not support
//! the REST endpoint: `do_generate` returns
//! [`AiMuxError::UnsupportedFunctionality`] for those model IDs, and `do_stream`
//! (RFC-0028, requires the `realtime` feature) is realtime-models-only — the
//! inverse gating.

use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::{HttpBody, MultipartForm, media_type_to_extension};
use async_trait::async_trait;
use serde_json::Value;

use crate::openai::config::OpenAIModelConfig;
use crate::openai::transcription::transcription_api::OpenAITranscriptionResponse;
use crate::openai::transcription::transcription_model_options::parse_openai_options;

// ── Language map ────────────────────────────────────────────────────────────

/// Maps OpenAI's full language names (e.g. `"english"`) to ISO 639-1 codes
/// (e.g. `"en"`). Source: OpenAI speech-to-text supported languages.
fn language_name_to_code(name: &str) -> Option<&'static str> {
    match name {
        "afrikaans" => Some("af"),
        "arabic" => Some("ar"),
        "armenian" => Some("hy"),
        "azerbaijani" => Some("az"),
        "belarusian" => Some("be"),
        "bosnian" => Some("bs"),
        "bulgarian" => Some("bg"),
        "catalan" => Some("ca"),
        "chinese" => Some("zh"),
        "croatian" => Some("hr"),
        "czech" => Some("cs"),
        "danish" => Some("da"),
        "dutch" => Some("nl"),
        "english" => Some("en"),
        "estonian" => Some("et"),
        "finnish" => Some("fi"),
        "french" => Some("fr"),
        "galician" => Some("gl"),
        "german" => Some("de"),
        "greek" => Some("el"),
        "hebrew" => Some("he"),
        "hindi" => Some("hi"),
        "hungarian" => Some("hu"),
        "icelandic" => Some("is"),
        "indonesian" => Some("id"),
        "italian" => Some("it"),
        "japanese" => Some("ja"),
        "kannada" => Some("kn"),
        "kazakh" => Some("kk"),
        "korean" => Some("ko"),
        "latvian" => Some("lv"),
        "lithuanian" => Some("lt"),
        "macedonian" => Some("mk"),
        "malay" => Some("ms"),
        "marathi" => Some("mr"),
        "maori" => Some("mi"),
        "nepali" => Some("ne"),
        "norwegian" => Some("no"),
        "persian" => Some("fa"),
        "polish" => Some("pl"),
        "portuguese" => Some("pt"),
        "romanian" => Some("ro"),
        "russian" => Some("ru"),
        "serbian" => Some("sr"),
        "slovak" => Some("sk"),
        "slovenian" => Some("sl"),
        "spanish" => Some("es"),
        "swahili" => Some("sw"),
        "swedish" => Some("sv"),
        "tagalog" => Some("tl"),
        "tamil" => Some("ta"),
        "thai" => Some("th"),
        "turkish" => Some("tr"),
        "ukrainian" => Some("uk"),
        "urdu" => Some("ur"),
        "vietnamese" => Some("vi"),
        "welsh" => Some("cy"),
        _ => None,
    }
}

/// Realtime transcription model IDs stream over the realtime WebSocket and do
/// not support the REST transcription endpoint. Prefix matching keeps dated
/// snapshots (e.g. `gpt-realtime-whisper-2026-01-01`) working.
fn is_realtime_transcription_model_id(model_id: &str) -> bool {
    model_id == "gpt-realtime-whisper" || model_id.starts_with("gpt-realtime-whisper-")
}

/// Convert `AudioInput` to raw bytes.
fn audio_input_to_bytes(audio: &AudioInput) -> Result<Vec<u8>, AiMuxError> {
    match audio {
        AudioInput::Binary(bytes) => Ok(bytes.clone()),
        AudioInput::Base64(b64) => {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).map_err(|e| {
                AiMuxError::InvalidArgument {
                    message: format!("invalid base64: {e}"),
                    cause: Some(ErrorCause::source(&e)),
                }
            })
        }
        AudioInput::Url(_) => Err(AudioInput::url_error()),
    }
}

/// An OpenAI-compatible transcription (STT) model.
///
/// Works with any OpenAI-compatible `/audio/transcriptions` endpoint.
pub struct OpenAITranscriptionModel {
    model_id: String,
    config: OpenAIModelConfig,
    #[cfg(feature = "realtime")]
    web_socket: Option<std::sync::Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl OpenAITranscriptionModel {
    pub(crate) fn from_config(model_id: String, config: OpenAIModelConfig) -> Self {
        Self {
            model_id,
            config,
            #[cfg(feature = "realtime")]
            web_socket: None,
        }
    }
    #[cfg(feature = "realtime")]
    pub(crate) fn with_web_socket(
        mut self,
        web_socket: Option<std::sync::Arc<dyn aimux_provider_utils::ws::WsConnector>>,
    ) -> Self {
        self.web_socket = web_socket;
        self
    }
}

#[async_trait]
impl TranscriptionModel for OpenAITranscriptionModel {
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
                "non-streaming transcription with {}",
                self.model_id
            )));
        }

        let openai_options = parse_openai_options(options.provider_options.as_ref());
        let warnings: Vec<Warning> = Vec::new();

        // Build multipart form.
        let audio_bytes = audio_input_to_bytes(&options.audio)?;
        let file_extension = media_type_to_extension(&options.media_type);
        let filename = format!("audio.{file_extension}");

        let mut form = MultipartForm::new();
        form.text("model", &self.model_id)?;
        form.file("file", &filename, &options.media_type, &audio_bytes)?;

        // whisper-1 defaults to verbose_json to get segments.
        if self.model_id == "whisper-1" {
            form.text("response_format", "verbose_json")?;
        }

        // Provider-specific options.
        let is_gpt4o_transcribe_model =
            self.model_id == "gpt-4o-transcribe" || self.model_id == "gpt-4o-mini-transcribe";

        let has_openai_options = options
            .provider_options
            .as_ref()
            .is_some_and(|options| options.contains_key("openai"));

        let is_diarization_model = self.model_id == "gpt-4o-transcribe-diarize";
        // Upstream only selects a non-default format when its options exist.
        if self.model_id != "whisper-1" && has_openai_options {
            let response_format = if let Some(format) = &openai_options.response_format {
                format.as_str()
            } else if is_diarization_model {
                "diarized_json"
            } else if is_gpt4o_transcribe_model {
                "json"
            } else {
                "verbose_json"
            };
            form.text("response_format", response_format)?;
        } else if !has_openai_options && is_diarization_model {
            form.text("response_format", "diarized_json")?;
        }

        if let Some(ref include) = openai_options.include {
            for item in include {
                form.text("include[]", item)?;
            }
        }
        if let Some(ref language) = openai_options.language {
            form.text("language", language)?;
        }
        if let Some(ref prompt) = openai_options.prompt {
            form.text("prompt", prompt)?;
        }
        if let Some(temperature) = openai_options.temperature {
            form.text("temperature", &temperature.to_string())?;
        }
        if let Some(ref tg) = openai_options.timestamp_granularities {
            for item in tg {
                form.text("timestamp_granularities[]", item)?;
            }
        }

        // Temperature default is 0 when provider options are present (matching
        // the TS schema's `.default(0)`).
        if has_openai_options && openai_options.timestamp_granularities.is_none() {
            form.text("timestamp_granularities[]", "segment")?;
        }
        if has_openai_options && openai_options.temperature.is_none() {
            form.text("temperature", "0")?;
        }
        let chunking_strategy = openai_options
            .chunking_strategy
            .as_deref()
            .or(is_diarization_model.then_some("auto"));
        if let Some(chunking_strategy) = chunking_strategy {
            form.text("chunking_strategy", chunking_strategy)?;
        }

        let (body_bytes, content_type) = form.finish();

        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;

        let resp = aimux_provider_utils::post_to_api(
            self.config
                .http_request(self.config.url("/audio/transcriptions")?, headers, options),
            HttpBody::Bytes(body_bytes, content_type),
            aimux_provider_utils::create_json_response_handler::<OpenAITranscriptionResponse>(),
            crate::openai::error::openai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let parsed = resp.value;

        // Map language name to ISO 639-1 code.
        let language = parsed
            .language
            .as_deref()
            .and_then(language_name_to_code)
            .map(std::string::ToString::to_string);

        // Diarized segments keep their speaker in `providerMetadata.openai.segments`
        // (`transcription/openai-transcription-model.ts:315-353`).
        let diarized_segments: Vec<Value> = parsed
            .segments
            .iter()
            .flatten()
            .filter_map(|segment| {
                let speaker = segment.speaker.as_ref()?;
                Some(serde_json::json!({
                    "text": segment.text,
                    "startSecond": segment.start,
                    "endSecond": segment.end,
                    "speaker": speaker,
                }))
            })
            .collect();
        let provider_metadata = if diarized_segments.is_empty() {
            None
        } else {
            Some(aimux_core::shared::provider_namespace(
                "openai",
                serde_json::json!({ "segments": diarized_segments }),
            )?)
        };

        // Segments: prefer `segments`, fall back to `words`.
        let segments: Vec<TranscriptionSegment> = if let Some(segs) = parsed.segments {
            segs.into_iter()
                .map(|s| TranscriptionSegment {
                    text: s.text,
                    start_second: s.start,
                    end_second: s.end,
                })
                .collect()
        } else if let Some(words) = parsed.words {
            words
                .into_iter()
                .map(|w| TranscriptionSegment {
                    text: w.word,
                    start_second: w.start,
                    end_second: w.end,
                })
                .collect()
        } else {
            Vec::new()
        };

        let timestamp = aimux_core::Timestamp::now();

        Ok(TranscriptionResult {
            text: parsed.text,
            segments,
            language,
            duration_in_seconds: parsed.duration,
            warnings,
            request: Some(TranscriptionRequest { body: None }),
            response: TranscriptionResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
                body: Some(raw_body),
            },
            provider_metadata,
        })
    }

    /// The model has a streaming method, as upstream; the session itself is
    /// `OpenAITranscriptionModel::stream_transcription`.
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

/// The bearer token rides the `openai-insecure-api-key.<token>` subprotocol
/// next to `realtime`, and the `authorization` header is dropped: OpenAI
/// rejects handshakes that send both auth channels. Without a bearer
/// `authorization` only `realtime` is offered and the headers are kept
/// (`getOpenAIRealtimeConnection`,
/// `transcription/openai-transcription-model.ts:620-652`).
#[cfg(feature = "realtime")]
fn realtime_connection(headers: Vec<(String, String)>) -> (Vec<String>, Vec<(String, String)>) {
    let token = headers
        .iter()
        .rfind(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .and_then(|(_, value)| {
            // The HTTP auth scheme is case-insensitive.
            let (scheme, token) = value.split_once(char::is_whitespace)?;
            let token = token.trim_start();
            (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then(|| token.to_string())
        });
    match token {
        None => (vec!["realtime".to_string()], headers),
        Some(token) => (
            vec![
                "realtime".to_string(),
                format!("openai-insecure-api-key.{token}"),
            ],
            headers
                .into_iter()
                .filter(|(name, _)| !name.eq_ignore_ascii_case("authorization"))
                .collect(),
        ),
    }
}

impl OpenAITranscriptionModel {
    /// Streaming transcription over the realtime WebSocket (RFC-0028 §3.2).
    ///
    /// Only realtime model IDs (`gpt-realtime-whisper*`) support streaming;
    /// other models return `UnsupportedFunctionality` (inverse of the
    /// `do_generate` gating).
    #[cfg(feature = "realtime")]
    async fn stream_transcription(
        &self,
        options: aimux_core::transcription_model::TranscriptionStreamOptions,
    ) -> Result<aimux_core::transcription_model::TranscriptionStreamResult, AiMuxError> {
        use aimux_core::transcription_model::{
            AudioChunk, TranscriptionStreamPart, TranscriptionStreamResult,
        };
        use aimux_provider_utils::ws::{WebSocketRequest, WsMessage, ws_connect};
        use futures::StreamExt;

        if !is_realtime_transcription_model_id(&self.model_id) {
            return Err(AiMuxError::UnsupportedFunctionality(format!(
                "streaming transcription is not supported by `{}` (realtime models such as gpt-realtime-whisper only)",
                self.model_id
            )));
        }

        let openai_options = parse_openai_options(options.provider_options.as_ref());

        // Options of the REST endpoint only, read from the raw options
        // (`transcription/openai-transcription-model.ts:395-410`).
        let raw_openai = options
            .provider_options
            .as_ref()
            .and_then(|options| options.get("openai"));
        let warnings: Vec<Warning> = ["include", "prompt", "temperature", "timestampGranularities"]
            .into_iter()
            .filter(|option| {
                raw_openai
                    .and_then(|openai| openai.get(*option))
                    .is_some_and(|value| !value.is_null())
            })
            .map(|option| Warning::Unsupported {
                feature: format!("providerOptions.openai.{option}"),
                details: Some(format!(
                    "OpenAI streaming transcription does not support {option}."
                )),
            })
            .collect();

        // wss URL from the REST base URL (https→wss / http→ws), matching the
        // AI SDK's toWebSocketUrl.
        let ws_url = self.config.ws_url("/realtime?intent=transcription")?;

        // session.update — exact upstream wire shape (RFC-0028 §3.2 B1): the
        // model rides in session.audio.input.transcription.model, NOT in the
        // URL; turn_detection nests under audio.input.
        let mut session_update = serde_json::json!({
            "type": "session.update",
            "session": {
                "type": "transcription",
                "audio": {
                    "input": {
                        "format": {
                            "type": options.input_audio_format.format_type,
                        },
                        "transcription": {
                            "model": self.model_id,
                        },
                        "turn_detection": null,
                    },
                },
            },
        });
        if let Some(rate) = options.input_audio_format.rate {
            session_update["session"]["audio"]["input"]["format"]["rate"] = serde_json::json!(rate);
        }
        if let Some(lang) = &openai_options.language {
            session_update["session"]["audio"]["input"]["transcription"]["language"] =
                serde_json::json!(lang);
        }
        // `providerOptions.openai.streaming` (`buildOpenAIRealtimeTranscriptionSession`,
        // `transcription/openai-transcription-model.ts:580-617`); the REST
        // `include` does not apply to the realtime session.
        let streaming = options
            .provider_options
            .as_ref()
            .and_then(|options| options.get("openai"))
            .and_then(|openai| openai.get("streaming"));
        if let Some(delay) = streaming
            .and_then(|streaming| streaming.get("delay"))
            .filter(|delay| !delay.is_null())
        {
            session_update["session"]["audio"]["input"]["transcription"]["delay"] = delay.clone();
        }
        if let Some(include) = streaming
            .and_then(|streaming| streaming.get("include"))
            .filter(|include| !include.is_null())
        {
            session_update["session"]["include"] = include.clone();
        }

        let mut header_list = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;

        let abort = options.abort_signal.clone();
        let include_raw = options.include_raw_chunks;
        let language = openai_options.language.clone();
        let model_id = self.model_id.clone();

        // Spawn the connect before entering the stream so connection errors
        // surface from do_stream's Result (clear contract: connect failure =
        // Err, not an in-stream error part).
        let (subprotocols, headers) = realtime_connection(std::mem::take(&mut header_list));
        let req = WebSocketRequest {
            url: ws_url.clone(),
            headers,
            subprotocols,
            abort_signal: abort.clone(),
            timeout: options.timeout,
            connector: self.web_socket.clone(),
        };
        let mut ws = ws_connect(&req).await?;

        // Session establishment (send session.update). Send errors are
        // abort-aware inside ws.send_text.
        ws.send_text(&serde_json::to_string(&session_update).map_err(|e| {
            AiMuxError::JsonParse {
                message: format!("serialize session.update: {e}"),
                cause: Some(ErrorCause::source(&e)),
            }
        })?)
        .await?;

        let mut audio = options.audio;
        let mut audio_done = false;
        let mut committed = false;
        let error_url = ws_url;
        let error_request = session_update.clone();

        let stream = async_stream::stream! {
            yield Ok(TranscriptionStreamPart::StreamStart { warnings });

            loop {
                // Audio arm disabled once exhausted (select! precondition).
                let audio_next = async {
                    if audio_done {
                        // Permanently pending arm — exhausted.
                        std::future::pending::<()>().await;
                        None
                    } else {
                        audio.next().await
                    }
                };

                tokio::select! {
                    biased;
                    // Audio arm first: a backpressured send parks the loop
                    // (socket-level backpressure), which is the correct
                    // throttle for a saturated pre-recorded source. Inbound
                    // events wait until the send completes; live-mic rates
                    // interleave naturally.

                    chunk = audio_next => {
                        match chunk {
                            Some(AudioChunk::Binary(bytes)) => {
                                let b64 = base64::Engine::encode(
                                    &base64::engine::general_purpose::STANDARD, &bytes);
                                if let Err(e) = ws.send_text(&serde_json::json!({
                                    "type": "input_audio_buffer.append",
                                    "audio": b64,
                                }).to_string()).await {
                                    yield Err(e);
                                    break;
                                }
                            }
                            Some(AudioChunk::Base64(b64)) => {
                                if let Err(e) = ws.send_text(&serde_json::json!({
                                    "type": "input_audio_buffer.append",
                                    "audio": b64,
                                }).to_string()).await {
                                    yield Err(e);
                                    break;
                                }
                            }
                            None => {
                                audio_done = true;
                                if !committed {
                                    committed = true;
                                    if let Err(e) = ws.send_text(
                                        r#"{"type":"input_audio_buffer.commit"}"#).await {
                                        yield Err(e);
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    event = ws.next() => {
                        match event {
                            None => {
                                // Peer closed. If we never got `completed`,
                                // this is a truncated session — surface it
                                // rather than ending silently.
                                yield Err(AiMuxError::ApiCall(Box::new(
                                    aimux_core::error::ApiCallError::new(
                                        "realtime transcription socket closed before completion",
                                        error_url.clone(),
                                        error_request.clone(),
                                    ))));
                                break;
                            }
                            Some(Err(e)) => {
                                yield Err(e);
                                break;
                            }
                            Some(Ok(WsMessage::Binary(_))) => {
                                // Realtime transcription events are JSON text;
                                // ignore binary frames.
                            }
                            Some(Ok(WsMessage::Text(text))) => {
                                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                                    continue;
                                };
                                if include_raw {
                                    yield Ok(TranscriptionStreamPart::Raw {
                                        raw_value: value.clone(),
                                    });
                                }
                                let event_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                match event_type {
                                    "conversation.item.input_audio_transcription.delta" => {
                                        yield Ok(TranscriptionStreamPart::TranscriptDelta {
                                            id: value.get("item_id").and_then(|v| v.as_str()).map(std::string::ToString::to_string),
                                            delta: value.get("delta").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                            provider_metadata: None,
                                        });
                                    }
                                    "conversation.item.input_audio_transcription.completed" => {
                                        let transcript = value.get("transcript")
                                            .and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let item_id = value.get("item_id")
                                            .and_then(|v| v.as_str()).map(std::string::ToString::to_string);
                                        yield Ok(TranscriptionStreamPart::TranscriptFinal {
                                            id: item_id.clone(),
                                            text: transcript.clone(),
                                            start_second: None,
                                            end_second: None,
                                            channel_index: None,
                                            provider_metadata: None,
                                        });
                                        yield Ok(TranscriptionStreamPart::Finish {
                                            text: transcript,
                                            segments: vec![],
                                            language: language.clone(),
                                            duration_in_seconds: None,
                                            provider_metadata: None,
                                        });
                                        // Termination: the realtime session is
                                        // long-lived by design; the SERVER will
                                        // not close after one transcript. Close
                                        // client-side (1000) like upstream
                                        // (RFC-0028 §3.2 B2).
                                        ws.close().await;
                                        break;
                                    }
                                    "error" => {
                                        let msg = value.pointer("/error/message")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("realtime transcription error");
                                        yield Err(AiMuxError::ApiCall(Box::new(
                                            aimux_core::error::ApiCallError {
                                                response_body: Some(value.to_string()),
                                                ..aimux_core::error::ApiCallError::new(
                                                    msg,
                                                    error_url.clone(),
                                                    error_request.clone(),
                                                )
                                            })));
                                        ws.close().await;
                                        break;
                                    }
                                    // session.created / session.updated /
                                    // input_audio_buffer.committed / rate_limits
                                    // etc. — no part mapping.
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
            request: Some(TranscriptionRequest {
                body: Some(serde_json::to_string(&session_update).unwrap_or_default()),
            }),
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
        Err(AiMuxError::UnsupportedFunctionality(
            "transcription streaming requires building aimux-providers with the `realtime` feature"
                .into(),
        ))
    }
}
