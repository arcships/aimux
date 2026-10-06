//! The Groq transcription model (`{name}.transcription`).
//!
//! Mirrors `groq-transcription-model.ts`: a multipart `POST
//! {base_url}/audio/transcriptions`. With `responseFormat: "text"` the body is
//! plain text and becomes the transcript; otherwise it is the JSON of
//! `groqTranscriptionResponseSchema`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::{HttpBody, MultipartForm, media_type_to_extension};

use crate::shared::EndpointConfig;

use super::error::groq_failed_response_handler;
use super::options::parse_groq_transcription_options;

/// `groqTranscriptionResponseSchema`. Every field the schema requires is
/// required here; the ones the model does not read are only validated.
#[derive(Deserialize)]
#[allow(dead_code, reason = "validated against the upstream response schema")]
struct GroqTranscriptionResponse {
    text: String,
    x_groq: XGroq,
    task: Option<String>,
    language: Option<String>,
    duration: Option<f64>,
    segments: Option<Vec<Segment>>,
    words: Option<Vec<Word>>,
}

#[derive(Deserialize)]
#[allow(dead_code, reason = "validated against the upstream response schema")]
struct XGroq {
    id: String,
}

#[derive(Deserialize)]
#[allow(dead_code, reason = "validated against the upstream response schema")]
struct Segment {
    id: f64,
    seek: f64,
    start: f64,
    end: f64,
    text: String,
    tokens: Vec<f64>,
    temperature: f64,
    avg_logprob: f64,
    compression_ratio: f64,
    no_speech_prob: f64,
}

#[derive(Deserialize)]
struct Word {
    word: String,
    start: f64,
    end: f64,
}

/// A Groq transcription (STT) model.
pub struct GroqTranscriptionModel {
    model_id: String,
    config: EndpointConfig,
}

impl GroqTranscriptionModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl TranscriptionModel for GroqTranscriptionModel {
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
        // `getArgs`.
        let groq_options = parse_groq_transcription_options(options.provider_options.as_ref())?;
        let audio = match &options.audio {
            AudioInput::Binary(bytes) => bytes.clone(),
            AudioInput::Base64(data) => {
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data).map_err(
                    |error| AiMuxError::InvalidArgument(format!("invalid base64: {error}")),
                )?
            }
        };

        let mut form = MultipartForm::new();
        form.text("model", &self.model_id)?;
        form.file(
            "file",
            &format!("audio.{}", media_type_to_extension(&options.media_type)),
            &options.media_type,
            &audio,
        )?;
        let response_format = groq_options
            .as_ref()
            .and_then(|o| o.response_format.clone());
        if let Some(groq) = &groq_options {
            if let Some(language) = &groq.language {
                form.text("language", language)?;
            }
            if let Some(prompt) = &groq.prompt {
                form.text("prompt", prompt)?;
            }
            if let Some(response_format) = &groq.response_format {
                form.text("response_format", response_format)?;
            }
            if let Some(temperature) = groq.temperature {
                form.text("temperature", &temperature.to_string())?;
            }
            for item in groq.timestamp_granularities.iter().flatten() {
                form.text("timestamp_granularities[]", item)?;
            }
        }
        let (body, content_type) = form.finish();

        // `doGenerate`.
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let request = exchange.request(exchange.url("/audio/transcriptions"), options);
        let body = HttpBody::Bytes(body, content_type);
        let (text, segments, language, duration, headers, raw_body) = if response_format.as_deref()
            == Some("text")
        {
            let response = aimux_provider_utils::post_to_api(
                request,
                body,
                aimux_provider_utils::create_binary_response_handler(),
                groq_failed_response_handler(),
            )
            .await?;
            let text = String::from_utf8_lossy(&response.value).into_owned();
            (
                text.clone(),
                Vec::new(),
                None,
                None,
                response.response_headers,
                Value::String(text),
            )
        } else {
            let response = aimux_provider_utils::post_to_api(
                request,
                body,
                aimux_provider_utils::create_json_response_handler::<GroqTranscriptionResponse>(),
                groq_failed_response_handler(),
            )
            .await?;
            let parsed = response.value;
            let segments = match (parsed.segments, parsed.words) {
                (Some(segments), _) => segments
                    .into_iter()
                    .map(|s| TranscriptionSegment {
                        text: s.text,
                        start_second: s.start,
                        end_second: s.end,
                    })
                    .collect(),
                (None, Some(words)) => words
                    .into_iter()
                    .map(|w| TranscriptionSegment {
                        text: w.word,
                        start_second: w.start,
                        end_second: w.end,
                    })
                    .collect(),
                (None, None) => Vec::new(),
            };
            (
                parsed.text,
                segments,
                parsed.language,
                parsed.duration,
                response.response_headers,
                response.raw_value.unwrap_or(Value::Null),
            )
        };

        Ok(TranscriptionResult {
            text,
            segments,
            language,
            duration_in_seconds: duration,
            warnings: Vec::new(),
            request: Some(TranscriptionRequest { body: None }),
            response: TranscriptionResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(headers),
                body: Some(raw_body),
            },
            provider_metadata: None,
        })
    }
}
