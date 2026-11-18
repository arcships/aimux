//! Cartesia transcription (STT) model.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::transcription_model::{
    TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest, TranscriptionResult,
    TranscriptionSegment,
};
use aimux_provider_utils::{HttpBody, MultipartForm, media_type_to_extension};

use crate::shared::{
    EndpointConfig, audio_input_to_bytes, namespaced_options, transcription_response, unsupported,
};

use super::provider::{NAMESPACE, cartesia_failed_response_handler};

/// Streaming transcription model IDs start with `ink-2` and only support the
/// WebSocket streaming endpoint, not the REST batch endpoint.
///
/// `do_stream` for these IDs is intentionally NOT implemented — deferred
/// with explicit triggers (RFC-0034 §4/D6: docs behind a login wall, young
/// turns API, schema inferable from the SDK only). Until it lands, both
/// paths return `UnsupportedFunctionality`.
fn is_streaming_transcription_model_id(model_id: &str) -> bool {
    model_id == "ink-2" || model_id.starts_with("ink-2-")
}

/// Cartesia transcription response word.
#[derive(Debug, Deserialize)]
struct CartesiaTranscriptionWord {
    word: String,
    start: f64,
    end: f64,
}

/// Cartesia transcription API response body.
#[derive(Debug, Deserialize)]
struct CartesiaTranscriptionResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    words: Option<Vec<CartesiaTranscriptionWord>>,
}

/// Cartesia transcription (STT) model — implements `TranscriptionModel`.
///
/// Aligned with Vercel AI SDK `CartesiaTranscriptionModel`
/// (`reference/ai/packages/cartesia/src/cartesia-transcription-model.ts`).
///
/// Endpoint: `POST {base_url}/stt` (multipart form-data)
pub struct CartesiaTranscriptionModel {
    model_id: String,
    config: EndpointConfig,
}

impl CartesiaTranscriptionModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl TranscriptionModel for CartesiaTranscriptionModel {
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
        if is_streaming_transcription_model_id(&self.model_id) {
            return Err(AiMuxError::UnsupportedFunctionality(format!(
                "non-streaming transcription with {}",
                self.model_id
            )));
        }

        let mut warnings: Vec<Warning> = Vec::new();

        // Parse provider options.
        let mut language: Option<String> = None;
        let mut timestamp_granularities: Option<Vec<String>> = None;
        if let Some(cartesia) = namespaced_options(options.provider_options.as_ref(), NAMESPACE) {
            if let Some(l) = cartesia.get("language").and_then(|v| v.as_str()) {
                language = Some(l.to_string());
            }
            if let Some(tg) = cartesia
                .get("timestampGranularities")
                .and_then(|v| v.as_array())
            {
                timestamp_granularities = Some(
                    tg.iter()
                        .filter_map(|v| v.as_str().map(std::string::ToString::to_string))
                        .collect(),
                );
            }
            if cartesia.get("streaming").is_some() {
                warnings.push(unsupported(
                    "providerOptions.cartesia.streaming",
                    "Cartesia batch transcription does not support streaming options.",
                ));
            }
        }

        let audio_bytes = audio_input_to_bytes(&options.audio)?;
        let file_extension = media_type_to_extension(&options.media_type);
        let filename = format!("audio.{file_extension}");

        let mut form = MultipartForm::new();
        form.text("model", &self.model_id)?;
        form.file("file", &filename, &options.media_type, &audio_bytes)?;

        if let Some(ref lang) = language {
            form.text("language", lang)?;
        }
        if let Some(ref tg) = timestamp_granularities {
            for g in tg {
                form.text("timestamp_granularities[]", g)?;
            }
        }

        let (body_bytes, content_type) = form.finish();

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_to_api(
            exchange.request(exchange.url("/stt"), options),
            HttpBody::Bytes(body_bytes, content_type),
            aimux_provider_utils::create_json_response_handler::<CartesiaTranscriptionResponse>(),
            cartesia_failed_response_handler(),
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
                        text: w.word.clone(),
                        start_second: w.start,
                        end_second: w.end,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let timestamp = aimux_core::Timestamp::now();

        Ok(TranscriptionResult {
            text: parsed.text,
            segments,
            language: parsed.language,
            duration_in_seconds: parsed.duration,
            warnings,
            request: Some(TranscriptionRequest { body: None }),
            response: transcription_response(timestamp, &self.model_id, response_headers, raw_body),
            provider_metadata: None,
        })
    }
}
