//! Gladia transcription (STT) provider.
//!
//! Aligned with Vercel AI SDK `createGladia` / `GladiaTranscriptionModel`
//! (`reference/ai/packages/gladia/src/gladia-transcription-model.ts`).
//!
//! Gladia uses a three-step async pattern:
//! 1. POST `/v2/upload` with multipart form (audio file) → returns `audio_url`
//! 2. POST `/v2/pre-recorded` with JSON body (audio_url + options) → returns `result_url`
//! 3. GET `result_url` polling until status is `done`
//!
//! [`create_gladia`] takes [`GladiaProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`GladiaProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `GLADIA_API_KEY`.
//! [`gladia()`] is the default instance; it reads nothing and cannot fail.

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
use aimux_provider_utils::{AuthScheme, HttpBody, MultipartForm, media_type_to_extension};

use crate::shared::{
    EndpointConfig, POLL_INTERVAL_MS_KEY, PollStep, audio_input_to_bytes, define_vendor_provider,
    is_poll_control_key, namespaced_options, poll_interval_ms, poll_until, task_failed,
    transcription_response,
};

fn gladia_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    crate::shared::nested_error_response_handler("Gladia request failed")
}

// ── Config ──────────────────────────────────────────────────────────────────

/// Milliseconds between two polls of a transcription job
/// (`providerOptions.gladia.pollIntervalMs` overrides it for one call).
const POLL_INTERVAL_MS: u64 = 100;
/// How many times a transcription job is polled before the call gives up (ten
/// minutes at the default interval).
const MAX_POLL_ATTEMPTS: u32 = 6_000;

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "gladia";

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.gladia.io";
pub(crate) const API_KEY_ENV_VAR: &str = "GLADIA_API_KEY";
const DEFAULT_NAME: &str = "gladia";

define_vendor_provider! {
    settings: GladiaProviderSettings,
    provider: GladiaProvider,
    create: create_gladia,
    singleton: gladia,
    display: "Gladia",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: None,
}

impl GladiaProvider {
    /// A transcription (STT) model; `provider()` is `"{name}.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> GladiaTranscriptionModel {
        GladiaTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        )
    }
}

crate::impl_single_modality_provider!(GladiaProvider, transcription_model, |p, id| p
    .transcription(id));

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
