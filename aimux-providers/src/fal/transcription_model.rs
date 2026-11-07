//! The fal transcription model (`fal-transcription-model.ts`), its options
//! (`fal-transcription-model-options.ts`, `fal-transcription-options.ts`)
//! and response schema (`fal-api-types.ts`).

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};

use crate::shared::{EndpointConfig, POLL_INTERVAL_MS_KEY, PollStep, poll_interval_ms, poll_until};

use super::error::{fal_failed_response_handler, is_in_progress};
use super::options;

/// How many times a queued transcription request is polled before the call
/// gives up (ten minutes at the default interval).
const MAX_POLL_ATTEMPTS: u32 = 6_000;

/// Milliseconds between two polls of a queued transcription request
/// (`providerOptions.fal.pollIntervalMs` overrides it for one call).
const POLL_INTERVAL_MS: u64 = 100;

// ── Response schema ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct FalJobResponse {
    request_id: String,
}

#[derive(Debug, Deserialize)]
struct FalChunk {
    text: String,
    #[serde(default)]
    timestamp: Option<Vec<f64>>,
}

#[derive(Debug, Deserialize)]
struct FalTranscriptionResponse {
    text: String,
    #[serde(default)]
    chunks: Option<Vec<FalChunk>>,
    #[serde(default, rename = "inferred_languages")]
    inferred_languages: Option<Vec<String>>,
}

/// `falTranscriptionModelOptionsSchema` (fal-transcription-model-options.ts),
/// a plain `z.object`: unknown keys are dropped. Every key is `.nullish()`;
/// a `.default()` fills a missing key and keeps an explicit `null`, as
/// `#[serde(default = ...)]` on an `Option` does.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FalTranscriptionOptions {
    #[serde(default = "FalTranscriptionOptions::default_language")]
    language: Option<String>,
    #[serde(default = "FalTranscriptionOptions::default_diarize")]
    diarize: Option<bool>,
    #[serde(default = "FalTranscriptionOptions::default_chunk_level")]
    chunk_level: Option<FalChunkLevel>,
    #[serde(default = "FalTranscriptionOptions::default_version")]
    version: Option<FalWhisperVersion>,
    #[serde(default = "FalTranscriptionOptions::default_batch_size")]
    batch_size: Option<serde_json::Number>,
    #[serde(default)]
    num_speakers: Option<serde_json::Number>,
}

impl FalTranscriptionOptions {
    fn default_language() -> Option<String> {
        Some("en".to_string())
    }

    fn default_diarize() -> Option<bool> {
        Some(true)
    }

    fn default_chunk_level() -> Option<FalChunkLevel> {
        Some(FalChunkLevel::Segment)
    }

    fn default_version() -> Option<FalWhisperVersion> {
        Some(FalWhisperVersion::Three)
    }

    fn default_batch_size() -> Option<serde_json::Number> {
        Some(64.into())
    }
}

/// `chunkLevel`: `z.enum(['segment', 'word'])`.
#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum FalChunkLevel {
    Segment,
    Word,
}

/// `version`: `z.enum(['3'])`.
#[derive(Debug, Deserialize, serde::Serialize)]
enum FalWhisperVersion {
    #[serde(rename = "3")]
    Three,
}

// ── Model ───────────────────────────────────────────────────────────────────

fn audio_input_to_base64(audio: &AudioInput) -> Result<String, AiMuxError> {
    match audio {
        AudioInput::Base64(s) => Ok(s.clone()),
        AudioInput::Binary(bytes) => Ok(aimux_provider_utils::convert_uint8_array_to_base64(bytes)),
        AudioInput::Url(_) => Err(AudioInput::url_error()),
    }
}

pub struct FalTranscriptionModel {
    model_id: String,
    config: EndpointConfig,
}

impl FalTranscriptionModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }

    fn submit_path(&self) -> String {
        format!("/fal-ai/{}", self.model_id)
    }

    fn poll_path(&self, request_id: &str) -> String {
        format!("/fal-ai/{}/requests/{request_id}", self.model_id)
    }
}

#[async_trait]
impl TranscriptionModel for FalTranscriptionModel {
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
        let warnings: Vec<Warning> = Vec::new();

        let base64_audio = audio_input_to_base64(&options.audio)?;
        let audio_url = format!("data:{};base64,{}", options.media_type, base64_audio);

        let mut body = Map::new();
        body.insert("task".to_string(), json!("transcribe"));
        body.insert("diarize".to_string(), json!(true));
        body.insert("chunk_level".to_string(), json!("word"));
        body.insert("audio_url".to_string(), json!(audio_url));

        // fal-transcription-model.ts getArgs: the parsed options, defaults
        // included, go into the body only when `providerOptions.fal` is set.
        if let Some(fal) = aimux_provider_utils::parse_provider_options::<FalTranscriptionOptions>(
            options::NAMESPACE,
            options.provider_options.as_ref(),
        )? {
            body.insert("language".to_string(), json!(fal.language));
            if let Some(version) = fal.version {
                body.insert("version".to_string(), json!(version));
            }
            if let Some(batch_size) = fal.batch_size {
                body.insert("batch_size".to_string(), json!(batch_size));
            }
            if let Some(num_speakers) = fal.num_speakers {
                body.insert("num_speakers".to_string(), json!(num_speakers));
            }
            if let Some(diarize) = fal.diarize {
                body.insert("diarize".to_string(), json!(diarize));
            }
            if let Some(chunk_level) = fal.chunk_level {
                body.insert("chunk_level".to_string(), json!(chunk_level));
            }
        }
        let fal_options = options::fal_options(options.provider_options.as_ref());

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        // Submit job.
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url(&self.submit_path()), options),
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            fal_failed_response_handler(),
        )
        .await?;

        let job: FalJobResponse = resp.value;

        // Poll for the result. Submitting is over: only this status request
        // repeats.
        let poll_url = exchange.url(&self.poll_path(&job.request_id));
        let resp = poll_until(
            &format!("fal request {}", job.request_id),
            options.abort_signal.as_ref(),
            Duration::from_millis(poll_interval_ms(
                fal_options,
                POLL_INTERVAL_MS_KEY,
                POLL_INTERVAL_MS,
            )),
            MAX_POLL_ATTEMPTS,
            || async {
                match aimux_provider_utils::get_from_api(
                    exchange.request(poll_url.clone(), options),
                    aimux_provider_utils::create_json_response_handler::<FalTranscriptionResponse>(
                    ),
                    fal_failed_response_handler(),
                )
                .await
                {
                    Ok(response) => Ok(PollStep::Ready(response)),
                    Err(error) if is_in_progress(&error) => Ok(PollStep::Pending),
                    Err(error) => Err(error),
                }
            },
        )
        .await?;
        let response_headers = resp.response_headers;
        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let parsed = resp.value;

        let segments: Vec<TranscriptionSegment> = parsed
            .chunks
            .as_ref()
            .map(|chunks| {
                chunks
                    .iter()
                    .map(|c| TranscriptionSegment {
                        text: c.text.clone(),
                        start_second: c
                            .timestamp
                            .as_ref()
                            .and_then(|t| t.first().copied())
                            .unwrap_or(0.0),
                        end_second: c
                            .timestamp
                            .as_ref()
                            .and_then(|t| t.get(1).copied())
                            .unwrap_or(0.0),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let language = parsed
            .inferred_languages
            .as_ref()
            .and_then(|l| l.first().cloned());

        let duration_in_seconds = parsed
            .chunks
            .as_ref()
            .and_then(|c| c.last())
            .and_then(|c| c.timestamp.as_ref())
            .and_then(|t| t.get(1).copied());

        let timestamp = aimux_core::Timestamp::now();

        Ok(TranscriptionResult {
            text: parsed.text,
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
