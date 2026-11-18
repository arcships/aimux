//! Gladia transcription (STT) model.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderMetadata, Warning, provider_namespace};
use aimux_core::transcription_model::{
    TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest, TranscriptionResult,
    TranscriptionSegment,
};
use aimux_provider_utils::{HttpBody, MultipartForm, media_type_to_extension};

use crate::shared::{
    EndpointConfig, POLL_INTERVAL_MS_KEY, PollStep, audio_input_to_bytes, is_poll_control_key,
    namespaced_options, poll_interval_ms, poll_until, task_failed, transcription_response,
};

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "gladia";

/// Milliseconds between two polls of a transcription job
/// (`providerOptions.gladia.pollIntervalMs` overrides it for one call).
const POLL_INTERVAL_MS: u64 = 100;
/// How many times a transcription job is polled before the call gives up (ten
/// minutes at the default interval).
const MAX_POLL_ATTEMPTS: u32 = 6_000;

fn gladia_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    crate::shared::nested_error_response_handler("Gladia request failed")
}

// ── Response schema ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GladiaUploadResponse {
    audio_url: String,
}

#[derive(Debug, Deserialize)]
struct GladiaInitResponse {
    result_url: String,
}

#[derive(Debug, Deserialize)]
struct GladiaUtterance {
    text: String,
    start: f64,
    end: f64,
}

#[derive(Debug, Deserialize)]
struct GladiaTranscription {
    full_transcript: String,
    languages: Vec<String>,
    utterances: Vec<GladiaUtterance>,
}

#[derive(Debug, Deserialize)]
struct GladiaMetadata {
    audio_duration: f64,
}

#[derive(Debug, Deserialize)]
struct GladiaResult {
    metadata: GladiaMetadata,
    transcription: GladiaTranscription,
}

#[derive(Debug, Deserialize)]
struct GladiaResultResponse {
    status: String,
    #[serde(default)]
    result: Option<GladiaResult>,
}

// ── Model ───────────────────────────────────────────────────────────────────

/// A Gladia transcription (STT) model.
pub struct GladiaTranscriptionModel {
    model_id: String,
    config: EndpointConfig,
}

impl GladiaTranscriptionModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl TranscriptionModel for GladiaTranscriptionModel {
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

        let audio_bytes = audio_input_to_bytes(&options.audio)?;
        let file_extension = media_type_to_extension(&options.media_type);
        let filename = format!("audio.{file_extension}");

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let gladia_options = namespaced_options(options.provider_options.as_ref(), NAMESPACE);

        // Step 1: Upload audio. One attempt: an upload is never replayed
        // implicitly.
        let mut form = MultipartForm::new();
        form.file("audio", &filename, &options.media_type, &audio_bytes)?;
        let (body_bytes, content_type) = form.finish();

        let resp = aimux_provider_utils::post_to_api(
            exchange.request(exchange.url("/v2/upload"), options),
            HttpBody::Bytes(body_bytes, content_type),
            aimux_provider_utils::create_json_response_handler(),
            gladia_failed_response_handler(),
        )
        .await?;

        let upload: GladiaUploadResponse = resp.value;

        // Step 2: Initiate transcription. This creates the job and is sent
        // exactly once; only the status poll below repeats.
        let mut body = Map::new();
        body.insert("audio_url".to_string(), json!(upload.audio_url));

        // Forward all provider options as-is (the API uses snake_case), except
        // the poll interval, which is read here.
        if let Some(obj) = gladia_options {
            for (k, v) in obj {
                if !is_poll_control_key(k) {
                    body.insert(k.clone(), v.clone());
                }
            }
        }

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/v2/pre-recorded"), options),
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            gladia_failed_response_handler(),
        )
        .await?;

        let init: GladiaInitResponse = resp.value;

        // Step 3: Poll for the result.
        //
        // AI SDK polls result_url with validateUrl: true and credentialedOrigin
        // = the API origin: the target is validated and headers survive only
        // while it stays on base_url's origin.
        let resp = poll_until(
            "gladia transcription",
            options.abort_signal.as_ref(),
            Duration::from_millis(poll_interval_ms(
                gladia_options,
                POLL_INTERVAL_MS_KEY,
                POLL_INTERVAL_MS,
            )),
            MAX_POLL_ATTEMPTS,
            || async {
                let mut request = exchange.request(init.result_url.clone(), options);
                request.validate_url = true;
                request.trusted_origin = Some(exchange.base_url().to_string());
                let resp = aimux_provider_utils::get_from_api(
                    request,
                    aimux_provider_utils::create_json_response_handler::<GladiaResultResponse>(),
                    gladia_failed_response_handler(),
                )
                .await?;
                match resp.value.status.clone().as_str() {
                    "done" => Ok(PollStep::Ready(resp)),
                    "error" => Err(task_failed(
                        "Transcription job failed",
                        init.result_url.clone(),
                        resp.value.status.clone(),
                        Some(resp.raw_value.clone().unwrap_or(Value::Null).to_string()),
                    )),
                    _ => Ok(PollStep::Pending),
                }
            },
        )
        .await?;

        let response_headers = resp.response_headers;
        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let parsed = resp.value;

        let result = parsed.result.ok_or_else(|| {
            AiMuxError::invalid_response_data(
                "Gladia transcription completed without a result".to_string(),
            )
        })?;

        let segments: Vec<TranscriptionSegment> = result
            .transcription
            .utterances
            .iter()
            .map(|u| TranscriptionSegment {
                text: u.text.clone(),
                start_second: u.start,
                end_second: u.end,
            })
            .collect();

        let language = result.transcription.languages.first().cloned();
        let duration_in_seconds = Some(result.metadata.audio_duration);

        let timestamp = aimux_core::Timestamp::now();

        let provider_metadata: Option<SharedProviderMetadata> =
            Some(provider_namespace(NAMESPACE, raw_body.clone())?);
        Ok(TranscriptionResult {
            text: result.transcription.full_transcript,
            segments,
            language,
            duration_in_seconds,
            warnings,
            request: Some(TranscriptionRequest { body: None }),
            response: transcription_response(timestamp, &self.model_id, response_headers, raw_body),
            provider_metadata,
        })
    }
}
