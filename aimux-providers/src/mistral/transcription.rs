//! Mistral transcription model — implements the `TranscriptionModel` trait.
//!
//! Aligned with Vercel AI SDK `MistralTranscriptionModel`
//! (`reference/ai/packages/mistral/src/mistral-transcription-model.ts`).
//!
//! Endpoint: `POST {base_url}/audio/transcriptions` (multipart form-data).

use async_trait::async_trait;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Map, Number, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderMetadata;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionResponse,
    TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::{MultipartForm, media_type_to_extension};

use crate::shared::EndpointConfig;

#[derive(Deserialize)]
struct ResponseBody {
    model: String,
    text: String,
    language: Option<String>,
    segments: Option<Vec<ResponseSegment>>,
    usage: Option<ResponseUsage>,
}

#[derive(Deserialize)]
struct ResponseSegment {
    r#type: Option<String>,
    text: String,
    start: Number,
    end: Number,
    score: Option<Number>,
    speaker_id: Option<String>,
}

#[derive(Deserialize)]
struct ResponseUsage {
    prompt_tokens: Option<Number>,
    completion_tokens: Option<Number>,
    total_tokens: Option<Number>,
    prompt_audio_seconds: Option<Number>,
    request_count: Option<Number>,
}

/// A Mistral transcription (STT) model (e.g. `"voxtral-mini-latest"`).
pub struct MistralTranscriptionModel {
    model_id: String,
    config: EndpointConfig,
}

impl MistralTranscriptionModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

/// `appendFormValue` for a list: one field per item.
fn append_all(form: &mut MultipartForm, key: &str, items: &[&str]) -> Result<(), AiMuxError> {
    for item in items {
        form.text(key, item)?;
    }
    Ok(())
}

/// Insert `key` only when the number is present (the `...(x != null && {..})`
/// spreads of the upstream metadata).
fn put(map: &mut Map<String, Value>, key: &str, value: &Option<Number>) {
    if let Some(value) = value {
        map.insert(key.to_string(), Value::Number(value.clone()));
    }
}

#[async_trait]
impl TranscriptionModel for MistralTranscriptionModel {
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
        let mistral = super::options::transcription_options(options.provider_options.as_ref())?;

        // Mistral documents these options as mutually incompatible. Rejecting
        // the combination locally provides a stable SDK error instead of an
        // API 4xx.
        if mistral.language.is_some() && mistral.timestamp_granularities.is_some() {
            return Err(AiMuxError::InvalidArgument(
                "providerOptions.mistral.language cannot be combined with \
                 providerOptions.mistral.timestampGranularities"
                    .into(),
            ));
        }

        let audio = match &options.audio {
            AudioInput::Binary(bytes) => bytes.clone(),
            AudioInput::Base64(data) => base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|error| AiMuxError::InvalidArgument(format!("invalid base64: {error}")))?,
        };
        let mut form = MultipartForm::new();
        form.text("model", &self.model_id)?;
        form.file(
            "file",
            &format!("audio.{}", media_type_to_extension(&options.media_type)),
            &options.media_type,
            &audio,
        )?;
        if let Some(language) = mistral.language {
            form.text("language", language)?;
        }
        if let Some(temperature) = mistral.temperature {
            form.text("temperature", &temperature.to_string())?;
        }
        append_all(
            &mut form,
            "timestamp_granularities",
            mistral.timestamp_granularities.as_deref().unwrap_or(&[]),
        )?;
        if let Some(diarize) = mistral.diarize {
            form.text("diarize", &diarize.to_string())?;
        }
        append_all(
            &mut form,
            "context_bias",
            mistral.context_bias.as_deref().unwrap_or(&[]),
        )?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let resp = aimux_provider_utils::post_form_data_to_api(
            exchange.request(exchange.url("/audio/transcriptions"), options),
            form,
            aimux_provider_utils::create_json_response_handler::<ResponseBody>(),
            super::mistral_failed_response_handler(),
        )
        .await?;
        let response = resp.value;
        // z.literal('transcription_segment').nullish()
        if response.segments.iter().flatten().any(|segment| {
            segment
                .r#type
                .as_deref()
                .is_some_and(|kind| kind != "transcription_segment")
        }) {
            return Err(AiMuxError::InvalidResponseData(
                "Invalid transcription segment type".into(),
            ));
        }

        let segments: Vec<TranscriptionSegment> = response
            .segments
            .iter()
            .flatten()
            .map(|segment| TranscriptionSegment {
                text: segment.text.clone(),
                start_second: segment.start.as_f64().unwrap_or_default(),
                end_second: segment.end.as_f64().unwrap_or_default(),
            })
            .collect();

        let mut metadata = Map::new();
        if let Some(usage) = &response.usage {
            let mut map = Map::new();
            put(&mut map, "promptTokens", &usage.prompt_tokens);
            put(&mut map, "completionTokens", &usage.completion_tokens);
            put(&mut map, "totalTokens", &usage.total_tokens);
            put(&mut map, "promptAudioSeconds", &usage.prompt_audio_seconds);
            put(&mut map, "requestCount", &usage.request_count);
            metadata.insert("usage".into(), Value::Object(map));
        }
        let provider_segments: Vec<Value> = response
            .segments
            .iter()
            .flatten()
            .filter(|s| s.r#type.is_some() || s.score.is_some() || s.speaker_id.is_some())
            .map(|segment| {
                let mut map = Map::new();
                map.insert("text".into(), json!(segment.text));
                map.insert("startSecond".into(), Value::Number(segment.start.clone()));
                map.insert("endSecond".into(), Value::Number(segment.end.clone()));
                if let Some(kind) = &segment.r#type {
                    map.insert("type".into(), json!(kind));
                }
                put(&mut map, "score", &segment.score);
                if let Some(speaker_id) = &segment.speaker_id {
                    map.insert("speakerId".into(), json!(speaker_id));
                }
                Value::Object(map)
            })
            .collect();
        if !provider_segments.is_empty() {
            metadata.insert("segments".into(), Value::Array(provider_segments));
        }

        let duration_in_seconds = response
            .usage
            .as_ref()
            .and_then(|usage| usage.prompt_audio_seconds.as_ref()?.as_f64())
            .or_else(|| segments.last().map(|segment| segment.end_second));
        let provider_metadata = (!metadata.is_empty()).then(|| {
            SharedProviderMetadata::from([(super::options::NAMESPACE.to_string(), metadata)])
        });

        Ok(TranscriptionResult {
            text: response.text,
            segments,
            language: response.language,
            duration_in_seconds,
            warnings: Vec::new(),
            request: None,
            response: TranscriptionResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(response.model),
                headers: Some(resp.response_headers),
                body: resp.raw_value,
            },
            provider_metadata,
        })
    }
}
