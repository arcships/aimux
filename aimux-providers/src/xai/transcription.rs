//! xAI transcription (STT) model.
//!
//! Aligned with `XaiTranscriptionModel`
//! (`reference/aisdk-pinned/xai/src/xai-transcription-model.ts`).
//!
//! `do_generate`: `POST {base_url}/stt`, multipart, `file` last. `do_stream`:
//! the same path over a WebSocket (`ws(s)://{base_url}/stt?...`): the server
//! sends `transcript.created`, the client then sends binary audio frames and
//! `{"type":"audio.done"}`, and the server answers with `transcript.partial`
//! events and one `transcript.done` per channel.

#[cfg(feature = "realtime")]
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::shared::JsonObject;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionResponse,
    TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::{HttpBody, MultipartForm, media_type_to_extension};

use super::options;
use crate::shared::EndpointConfig;

/// An xAI transcription model. It has no model id: `model_id()` is `""`.
pub struct XaiTranscriptionModel {
    config: EndpointConfig,
    #[cfg(feature = "realtime")]
    web_socket: Option<Arc<dyn aimux_provider_utils::ws::WsConnector>>,
}

impl XaiTranscriptionModel {
    pub(crate) fn from_config(
        config: EndpointConfig,
        #[cfg(feature = "realtime")] web_socket: Option<
            Arc<dyn aimux_provider_utils::ws::WsConnector>,
        >,
    ) -> Self {
        Self {
            config,
            #[cfg(feature = "realtime")]
            web_socket,
        }
    }
}

/// `xaiTranscriptionModelOptionsSchema`, validated. The streaming fields are
/// read by the WebSocket path only.
#[derive(Default)]
#[cfg_attr(not(feature = "realtime"), allow(dead_code))]
struct Opts {
    audio_format: Option<String>,
    sample_rate: Option<i64>,
    language: Option<String>,
    format: Option<bool>,
    multichannel: Option<bool>,
    channels: Option<i64>,
    diarize: Option<bool>,
    keyterm: Vec<String>,
    filler_words: Option<bool>,
    interim_results: Option<bool>,
    endpointing: Option<i64>,
    smart_turn: Option<f64>,
    smart_turn_timeout: Option<i64>,
}

impl Opts {
    fn parse(
        provider_options: Option<&aimux_core::shared::SharedProviderOptions>,
    ) -> Result<Self, AiMuxError> {
        let Some(xai) = options::xai_options(provider_options) else {
            return Ok(Self::default());
        };
        let streaming = match xai.get("streaming").filter(|v| !v.is_null()) {
            None => None,
            Some(Value::Object(map)) => Some(map),
            Some(_) => {
                return Err(AiMuxError::InvalidArgument(
                    "Invalid argument for parameter providerOptions: xai.streaming must be an object".into(),
                ));
            }
        };
        let empty = JsonObject::new();
        let streaming = streaming.unwrap_or(&empty);
        Ok(Self {
            audio_format: options::opt_enum(xai, "audioFormat", &["pcm", "mulaw", "alaw"])?,
            sample_rate: options::sample_rate(xai)?,
            language: options::opt_string(xai, "language")?,
            format: options::opt_bool(xai, "format")?,
            multichannel: options::opt_bool(xai, "multichannel")?,
            channels: options::opt_int(xai, "channels", 2, 8, &[])?,
            diarize: options::opt_bool(xai, "diarize")?,
            keyterm: options::keyterms(xai)?,
            filler_words: options::opt_bool(xai, "fillerWords")?,
            interim_results: options::opt_bool(streaming, "interimResults")?,
            endpointing: options::opt_int(streaming, "endpointing", 0, 5000, &[])?,
            smart_turn: options::opt_f64(streaming, "smartTurn", 0.0, 1.0)?,
            smart_turn_timeout: options::opt_int(streaming, "smartTurnTimeout", 1, 5000, &[])?,
        })
    }
}

#[derive(Deserialize)]
struct SttResponse {
    text: String,
    language: Option<String>,
    duration: Option<f64>,
    words: Option<Vec<SttWord>>,
}

#[derive(Deserialize)]
struct SttWord {
    text: String,
    start: f64,
    end: f64,
}

#[async_trait]
impl TranscriptionModel for XaiTranscriptionModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        ""
    }

    async fn do_generate(
        &self,
        options: &TranscriptionCallOptions,
    ) -> Result<TranscriptionResult, AiMuxError> {
        let xai = Opts::parse(options.provider_options.as_ref())?;
        let mut form = MultipartForm::new();
        for (key, value) in [
            ("audio_format", xai.audio_format),
            ("sample_rate", xai.sample_rate.map(|v| v.to_string())),
            ("language", xai.language),
            ("format", xai.format.map(|v| v.to_string())),
            ("multichannel", xai.multichannel.map(|v| v.to_string())),
            ("channels", xai.channels.map(|v| v.to_string())),
            ("diarize", xai.diarize.map(|v| v.to_string())),
            ("filler_words", xai.filler_words.map(|v| v.to_string())),
        ] {
            if let Some(value) = value {
                form.text(key, &value)?;
            }
        }
        for keyterm in &xai.keyterm {
            form.text("keyterm", keyterm)?;
        }
        let audio = match &options.audio {
            AudioInput::Binary(bytes) => bytes.clone(),
            AudioInput::Base64(data) => base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| AiMuxError::InvalidArgument(format!("invalid base64: {e}")))?,
        };
        // xAI requires `file` to be the final multipart field.
        form.file(
            "file",
            &format!("audio.{}", media_type_to_extension(&options.media_type)),
            &options.media_type,
            &audio,
        )?;
        let (body, content_type) = form.finish();

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let resp = aimux_provider_utils::post_to_api(
            exchange.request(exchange.url("/stt"), options),
            HttpBody::Bytes(body, content_type),
            aimux_provider_utils::create_json_response_handler::<SttResponse>(),
            super::xai_failed_response_handler(),
        )
        .await?;
        let response = resp.value;

        Ok(TranscriptionResult {
            text: response.text,
            segments: response
                .words
                .unwrap_or_default()
                .into_iter()
                .map(|w| TranscriptionSegment {
                    text: w.text,
                    start_second: w.start,
                    end_second: w.end,
                })
                .collect(),
            language: response.language.filter(|l| !l.is_empty()),
            duration_in_seconds: response.duration,
            warnings: Vec::new(),
            request: None,
            response: TranscriptionResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(String::new()),
                headers: Some(resp.response_headers),
                body: resp.raw_value,
            },
            provider_metadata: None,
        })
    }

    #[cfg(feature = "realtime")]
    async fn do_stream(
        &self,
        options: aimux_core::transcription_model::TranscriptionStreamOptions,
    ) -> Result<aimux_core::transcription_model::TranscriptionStreamResult, AiMuxError> {
        stream::do_stream(self, options).await
    }
}

#[cfg(feature = "realtime")]
mod stream {
    use std::collections::{BTreeMap, HashMap};

    use aimux_core::shared::Warning;
    use aimux_core::transcription_model::{
        AudioChunk, TranscriptionRequest, TranscriptionStreamOptions, TranscriptionStreamPart,
        TranscriptionStreamResult,
    };
    use aimux_provider_utils::ws::{WebSocketRequest, WsMessage, ws_connect};
    use futures::StreamExt;

    use super::*;

    fn append<T: url::form_urlencoded::Target>(
        query: &mut url::form_urlencoded::Serializer<'_, T>,
        key: &str,
        value: Option<impl ToString>,
    ) {
        if let Some(value) = value {
            query.append_pair(key, &value.to_string());
        }
    }

    /// `buildXaiStreamingTranscriptionUrl`.
    fn build_url(
        base_url: &str,
        rate: Option<u32>,
        format_type: &str,
        xai: &Opts,
    ) -> Result<url::Url, AiMuxError> {
        let mut url = url::Url::parse(&format!("{base_url}/stt"))
            .map_err(|e| AiMuxError::InvalidArgument(format!("invalid base URL: {e}")))?;
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .map_err(|()| AiMuxError::InvalidArgument("invalid base URL scheme".into()))?;
        let encoding = match format_type {
            "audio/pcmu" => "mulaw",
            "audio/pcma" => "alaw",
            _ => "pcm",
        };
        let mut query = url.query_pairs_mut();
        append(
            &mut query,
            "sample_rate",
            xai.sample_rate.or(rate.map(i64::from)),
        );
        append(
            &mut query,
            "encoding",
            Some(xai.audio_format.as_deref().unwrap_or(encoding)),
        );
        append(&mut query, "language", xai.language.as_deref());
        append(&mut query, "diarize", xai.diarize);
        append(&mut query, "filler_words", xai.filler_words);
        append(&mut query, "multichannel", xai.multichannel);
        append(&mut query, "channels", xai.channels);
        append(&mut query, "interim_results", xai.interim_results);
        append(&mut query, "endpointing", xai.endpointing);
        append(&mut query, "smart_turn", xai.smart_turn);
        append(&mut query, "smart_turn_timeout", xai.smart_turn_timeout);
        for keyterm in &xai.keyterm {
            query.append_pair("keyterm", keyterm);
        }
        drop(query);
        Ok(url)
    }

    fn channel_id(index: Option<u32>) -> Option<String> {
        index.map(|i| format!("channel-{i}"))
    }

    pub(super) async fn do_stream(
        model: &XaiTranscriptionModel,
        options: TranscriptionStreamOptions,
    ) -> Result<TranscriptionStreamResult, AiMuxError> {
        let xai = Opts::parse(options.provider_options.as_ref())?;
        if xai.multichannel == Some(true) && xai.channels.is_none() {
            return Err(AiMuxError::InvalidArgument(
                "providerOptions.xai.channels is required when providerOptions.xai.multichannel is true"
                    .into(),
            ));
        }
        let mut warnings = Vec::new();
        if xai.format.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "providerOptions.xai.format".into(),
                details: Some("xAI streaming transcription does not support format.".into()),
            });
        }
        let format_type = options.input_audio_format.format_type.clone();
        if xai.audio_format.is_none()
            && !matches!(
                format_type.as_str(),
                "audio/pcm" | "audio/pcmu" | "audio/pcma"
            )
        {
            warnings.push(Warning::Other {
                message: format!(
                    "Unrecognized inputAudioFormat.type \"{format_type}\"; \
                     falling back to raw PCM encoding. \
                     Use audio/pcm, audio/pcmu, or audio/pcma, \
                     or set providerOptions.xai.audioFormat explicitly."
                ),
            });
        }

        let exchange = model.config.exchange(options.headers.as_ref()).await?;
        let url = build_url(
            exchange.base_url(),
            options.input_audio_format.rate,
            &format_type,
            &xai,
        )?;
        let ws_url = url.to_string();
        let mut ws = ws_connect(&WebSocketRequest {
            url: ws_url.clone(),
            headers: exchange.headers(),
            subprotocols: Vec::new(),
            abort_signal: options.abort_signal.clone(),
            timeout: options.timeout,
            connector: model.web_socket.clone(),
        })
        .await?;

        let expected_done = if xai.multichannel == Some(true) {
            xai.channels.unwrap_or(1) as usize
        } else {
            1
        };
        let language = xai.language.clone();
        let include_raw = options.include_raw_chunks;
        let mut audio = options.audio;

        let stream = async_stream::stream! {
            let mut created = false;
            let mut audio_done = false;
            let mut done_texts: BTreeMap<u32, String> = BTreeMap::new();
            // Per-channel finalized utterances and the latest revisable text:
            // `transcript.done` carries no text, so the finish text is rebuilt.
            let mut finalized: HashMap<u32, Vec<String>> = HashMap::new();
            let mut pending: HashMap<u32, String> = HashMap::new();
            let mut done_duration: Option<f64> = None;

            loop {
                let audio_next = async {
                    if audio_done || !created {
                        std::future::pending::<()>().await;
                        None
                    } else {
                        audio.next().await
                    }
                };
                tokio::select! {
                    biased;

                    chunk = audio_next => {
                        let sent = match chunk {
                            None => {
                                audio_done = true;
                                ws.send_text(r#"{"type":"audio.done"}"#).await
                            }
                            Some(AudioChunk::Binary(bytes)) => ws.send_binary(&bytes).await,
                            Some(AudioChunk::Base64(data)) => {
                                match base64::engine::general_purpose::STANDARD.decode(data) {
                                    Ok(bytes) => ws.send_binary(&bytes).await,
                                    Err(e) => Err(AiMuxError::InvalidArgument(format!("invalid base64: {e}"))),
                                }
                            }
                        };
                        if let Err(e) = sent {
                            yield Err(e);
                            break;
                        }
                    }

                    event = ws.next() => {
                        let text = match event {
                            // The peer closed before `transcript.done`: the stream just ends.
                            None => break,
                            Some(Err(AiMuxError::ApiCall(e))) if e.message.starts_with("websocket closed by peer") => break,
                            Some(Err(e)) => {
                                yield Err(e);
                                break;
                            }
                            Some(Ok(WsMessage::Binary(_))) => continue,
                            Some(Ok(WsMessage::Text(text))) => text,
                        };
                        let Ok(raw) = serde_json::from_str::<Value>(&text) else { continue };
                        if include_raw {
                            yield Ok(TranscriptionStreamPart::Raw { raw_value: raw.clone() });
                        }
                        let channel_index = raw.get("channel_index").and_then(Value::as_u64).map(|v| v as u32);
                        let channel = channel_index.unwrap_or(0);
                        let text_of = || raw.get("text").and_then(Value::as_str).unwrap_or("").to_owned();
                        let number = |key: &str| raw.get(key).and_then(Value::as_f64);
                        let flag = |key: &str| raw.get(key).and_then(Value::as_bool).unwrap_or(false);

                        match raw.get("type").and_then(Value::as_str) {
                            Some("transcript.created") => {
                                created = true;
                                yield Ok(TranscriptionStreamPart::StreamStart { warnings: warnings.clone() });
                            }
                            Some("transcript.partial") => {
                                let text = text_of();
                                // Only `speech_final` completes an utterance; `is_final`
                                // fragments are revised later, so they stay partials.
                                if flag("is_final") && flag("speech_final") {
                                    if !text.is_empty() {
                                        finalized.entry(channel).or_default().push(text.clone());
                                    }
                                    pending.remove(&channel);
                                    let start = number("start");
                                    yield Ok(TranscriptionStreamPart::TranscriptFinal {
                                        id: channel_id(channel_index),
                                        text,
                                        start_second: start,
                                        end_second: start.zip(number("duration")).map(|(s, d)| s + d),
                                        channel_index,
                                        provider_metadata: None,
                                    });
                                } else {
                                    pending.insert(channel, text.clone());
                                    yield Ok(TranscriptionStreamPart::TranscriptPartial {
                                        id: channel_id(channel_index),
                                        text,
                                        start_second: number("start"),
                                        duration_in_seconds: number("duration"),
                                        channel_index,
                                        provider_metadata: None,
                                    });
                                }
                            }
                            Some("transcript.done") => {
                                // The done text is empty: fall back to the accumulated
                                // utterances plus any trailing unfinalized text.
                                let mut parts = finalized.get(&channel).cloned().unwrap_or_default();
                                parts.extend(pending.get(&channel).filter(|t| !t.is_empty()).cloned());
                                let text = Some(text_of()).filter(|t| !t.is_empty()).unwrap_or_else(|| parts.join(" "));
                                done_texts.insert(channel, text);
                                done_duration = number("duration").or(done_duration);
                                if done_texts.len() >= expected_done {
                                    yield Ok(TranscriptionStreamPart::Finish {
                                        text: done_texts.values().cloned().collect::<Vec<_>>().join("\n"),
                                        segments: Vec::new(),
                                        language: language.clone(),
                                        duration_in_seconds: done_duration,
                                        provider_metadata: None,
                                    });
                                    ws.close().await;
                                    break;
                                }
                            }
                            // xAI STT errors are terminal: surface the server message.
                            Some("error") => {
                                yield Err(AiMuxError::Other(
                                    raw.get("message").and_then(Value::as_str).unwrap_or("xAI STT error").to_owned(),
                                ));
                                ws.close().await;
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        };

        Ok(TranscriptionStreamResult {
            stream: Box::pin(stream),
            request: Some(TranscriptionRequest { body: Some(ws_url) }),
            response: Some(TranscriptionResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(String::new()),
                headers: None,
                body: None,
            }),
        })
    }
}
