//! The Deepgram transcription model (`deepgram-transcription-model.ts`) and
//! its response schema (`deepgram-api-types.ts`).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::HttpBody;

use crate::shared::EndpointConfig;

use super::error::deepgram_failed_response_handler;
use super::options::{parse_deepgram_options, query_params};

// ── Response schema ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct DeepgramWord {
    word: String,
    start: f64,
    end: f64,
}

#[derive(Debug, Deserialize)]
struct DeepgramAlternative {
    transcript: String,
    #[serde(default)]
    words: Option<Vec<DeepgramWord>>,
}

#[derive(Debug, Deserialize)]
struct DeepgramChannel {
    #[serde(default, rename = "detected_language")]
    detected_language: Option<String>,
    alternatives: Vec<DeepgramAlternative>,
}

#[derive(Debug, Deserialize)]
struct DeepgramResults {
    #[serde(default)]
    channels: Option<Vec<DeepgramChannel>>,
}

#[derive(Debug, Deserialize)]
struct DeepgramMetadata {
    #[serde(default)]
    duration: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DeepgramResponse {
    #[serde(default)]
    metadata: Option<DeepgramMetadata>,
    #[serde(default)]
    results: Option<DeepgramResults>,
}

// ── Model ───────────────────────────────────────────────────────────────────

fn audio_input_to_bytes(audio: &AudioInput) -> Result<Vec<u8>, AiMuxError> {
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

pub struct DeepgramTranscriptionModel {
    model_id: String,
    config: EndpointConfig,
}

impl DeepgramTranscriptionModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl TranscriptionModel for DeepgramTranscriptionModel {
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
        let dg_options = parse_deepgram_options(options.provider_options.as_ref())?;
        let warnings: Vec<Warning> = Vec::new();

        let params = query_params(&self.model_id, &dg_options);
        // `new URLSearchParams(...).toString()`: form-encoded.
        let query_string = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params.iter().map(|(key, value)| (*key, value.as_str())))
            .finish();

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let url = exchange.url(&format!("/v1/listen?{query_string}"));

        let audio_bytes = audio_input_to_bytes(&options.audio)?;

        let resp = aimux_provider_utils::post_to_api(
            exchange.request(url, options),
            HttpBody::Bytes(audio_bytes, options.media_type.clone()),
            aimux_provider_utils::create_json_response_handler::<DeepgramResponse>(),
            deepgram_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let parsed = resp.value;

        let channel = parsed
            .results
            .as_ref()
            .and_then(|r| r.channels.as_ref())
            .and_then(|c| c.first());

        let text = channel
            .and_then(|ch| ch.alternatives.first())
            .map(|alt| alt.transcript.clone())
            .unwrap_or_default();

        let segments: Vec<TranscriptionSegment> = channel
            .and_then(|ch| ch.alternatives.first())
            .and_then(|alt| alt.words.as_ref())
            .map(|words| {
                words
                    .iter()
                    .map(|w| TranscriptionSegment {
                        text: w.word.clone(),
                        start_second: w.start,
                        end_second: w.end,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let language = channel.and_then(|ch| ch.detected_language.clone());

        let duration_in_seconds = parsed.metadata.as_ref().and_then(|m| m.duration);

        let timestamp = aimux_core::Timestamp::now();

        Ok(TranscriptionResult {
            text,
            segments,
            language,
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
}
