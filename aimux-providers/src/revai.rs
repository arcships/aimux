//! Rev.ai transcription (STT) provider.
//!
//! Aligned with Vercel AI SDK `createRevai` / `RevaiTranscriptionModel`
//! (`reference/ai/packages/revai/src/revai-transcription-model.ts`).
//!
//! Rev.ai uses an async job pattern:
//! 1. POST `/speechtotext/v1/jobs` with multipart form (media file + config JSON)
//! 2. GET `/speechtotext/v1/jobs/{id}` to poll until status is `transcribed`
//! 3. GET `/speechtotext/v1/jobs/{id}/transcript` to fetch the final transcript
//!
//! [`create_revai`] takes [`RevaiProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`RevaiProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `REVAI_API_KEY`.
//! [`revai()`] is the default instance; it reads nothing and cannot fail.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::transcription_model::{
    TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest, TranscriptionResult,
    TranscriptionSegment,
};
use aimux_provider_utils::{AuthScheme, MultipartForm, media_type_to_extension};

use crate::shared::{
    EndpointConfig, POLL_INTERVAL_MS_KEY, PollStep, audio_input_to_bytes, define_vendor_provider,
    namespaced_options, poll_interval_ms, poll_until, retry_download, task_failed,
    transcription_response,
};

fn revai_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    crate::shared::nested_error_response_handler("Rev.ai request failed")
}

// ── Config ──────────────────────────────────────────────────────────────────

/// Milliseconds between two polls of a job
/// (`providerOptions.revai.pollIntervalMs` overrides it for one call).
const POLL_INTERVAL_MS: u64 = 100;
/// How many times a job is polled before the call gives up (ten minutes at the
/// default interval).
const MAX_POLL_ATTEMPTS: u32 = 6_000;

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "revai";

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.rev.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "REVAI_API_KEY";
const DEFAULT_NAME: &str = "revai";

define_vendor_provider! {
    settings: RevaiProviderSettings,
    provider: RevaiProvider,
    create: create_revai,
    singleton: revai,
    display: "Rev.ai",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: None,
}

impl RevaiProvider {
    /// A transcription (STT) model (e.g. `"machine"`); `provider()` is `"{name}.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> RevaiTranscriptionModel {
        RevaiTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        )
    }
}

crate::impl_single_modality_provider!(RevaiProvider, transcription_model, |p, id| p
    .transcription(id));

// ── Response schema ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RevaiJobResponse {
    id: Option<String>,
    status: Option<String>,
    #[serde(default)]
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RevaiElement {
    #[serde(default, rename = "type")]
    elem_type: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    ts: Option<f64>,
    #[serde(default, rename = "end_ts")]
    end_ts: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RevaiMonologue {
    #[serde(default)]
    elements: Option<Vec<RevaiElement>>,
}

#[derive(Debug, Deserialize)]
struct RevaiTranscriptResponse {
    #[serde(default)]
    monologues: Option<Vec<RevaiMonologue>>,
}

// ── Model ───────────────────────────────────────────────────────────────────

pub struct RevaiTranscriptionModel {
    model_id: String,
    config: EndpointConfig,
}

impl RevaiTranscriptionModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl TranscriptionModel for RevaiTranscriptionModel {
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

        // Build config JSON.
        let config = json!({ "transcriber": self.model_id }).to_string();

        // Build multipart form.
        let mut form = MultipartForm::new();
        form.file("media", &filename, &options.media_type, &audio_bytes)?;
        form.text("config", &config)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let submit_url = exchange.url("/speechtotext/v1/jobs");

        // Submit job. This creates the job and is sent exactly once; only the
        // status poll and the transcript fetch below repeat.
        let resp = aimux_provider_utils::post_form_data_to_api(
            exchange.request(submit_url.clone(), options),
            form,
            aimux_provider_utils::create_json_response_handler::<RevaiJobResponse>(),
            revai_failed_response_handler(),
        )
        .await?;

        let response_body = resp.raw_value.as_ref().map(ToString::to_string);
        let submit_response: RevaiJobResponse = resp.value;

        if submit_response.status.as_deref() == Some("failed") {
            return Err(task_failed(
                "Failed to submit transcription job to Rev.ai",
                submit_url,
                "failed",
                response_body,
            ));
        }

        let job_id = submit_response.id.ok_or_else(|| {
            AiMuxError::invalid_response_data(
                "Rev.ai job submission did not return an id".to_string(),
            )
        })?;
        let submission_language = submit_response.language;
        let poll_interval = Duration::from_millis(poll_interval_ms(
            namespaced_options(options.provider_options.as_ref(), NAMESPACE),
            POLL_INTERVAL_MS_KEY,
            POLL_INTERVAL_MS,
        ));

        // Poll for completion.
        let poll_url = exchange.url(&format!("/speechtotext/v1/jobs/{job_id}"));
        poll_until(
            &format!("revai job {job_id}"),
            options.abort_signal.as_ref(),
            poll_interval,
            MAX_POLL_ATTEMPTS,
            || async {
                let resp = aimux_provider_utils::get_from_api(
                    exchange.request(poll_url.clone(), options),
                    aimux_provider_utils::create_json_response_handler::<RevaiJobResponse>(),
                    revai_failed_response_handler(),
                )
                .await?;
                let response_body = resp.raw_value.as_ref().map(ToString::to_string);
                match resp.value.status.as_deref() {
                    Some("transcribed") => Ok(PollStep::Ready(())),
                    Some("failed") => Err(task_failed(
                        "Transcription job failed",
                        poll_url.clone(),
                        "failed",
                        response_body,
                    )),
                    _ => Ok(PollStep::Pending),
                }
            },
        )
        .await?;

        // Fetch transcript. A transient failure repeats this request, never the
        // submit.
        let transcript_url = exchange.url(&format!("/speechtotext/v1/jobs/{job_id}/transcript"));
        let resp = retry_download(options.abort_signal.as_ref(), poll_interval, || {
            aimux_provider_utils::get_from_api(
                exchange.request(transcript_url.clone(), options),
                aimux_provider_utils::create_json_response_handler::<RevaiTranscriptResponse>(),
                revai_failed_response_handler(),
            )
        })
        .await?;

        let response_headers = resp.response_headers;
        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let parsed = resp.value;

        // Process monologues to extract segments and text.
        let mut segments: Vec<TranscriptionSegment> = Vec::new();
        let mut full_text_parts: Vec<String> = Vec::new();
        let mut duration_in_seconds = 0.0_f64;

        for monologue in parsed.monologues.as_deref().unwrap_or(&[]) {
            let mut current_segment_text = String::new();
            let mut segment_start = 0.0_f64;
            let mut has_started = false;

            let mut monologue_text = String::new();

            for element in monologue.elements.as_deref().unwrap_or(&[]) {
                let value = element.value.as_deref().unwrap_or("");
                current_segment_text.push_str(value);
                monologue_text.push_str(value);

                if element.elem_type.as_deref() == Some("text") {
                    if let Some(end_ts) = element.end_ts
                        && end_ts > duration_in_seconds
                    {
                        duration_in_seconds = end_ts;
                    }

                    if !has_started && let Some(ts) = element.ts {
                        segment_start = ts;
                        has_started = true;
                    }

                    if let Some(end_ts) = element.end_ts {
                        if has_started && !current_segment_text.trim().is_empty() {
                            segments.push(TranscriptionSegment {
                                text: current_segment_text.trim().to_string(),
                                start_second: segment_start,
                                end_second: end_ts,
                            });
                        }
                        current_segment_text.clear();
                        has_started = false;
                    }
                }
            }

            if has_started && !current_segment_text.trim().is_empty() {
                let end = if duration_in_seconds > segment_start {
                    duration_in_seconds
                } else {
                    segment_start + 1.0
                };
                segments.push(TranscriptionSegment {
                    text: current_segment_text.trim().to_string(),
                    start_second: segment_start,
                    end_second: end,
                });
            }

            full_text_parts.push(monologue_text);
        }

        let full_text = full_text_parts.join(" ");
        let language = submission_language;

        let timestamp = aimux_core::Timestamp::now();

        Ok(TranscriptionResult {
            text: full_text,
            segments,
            language,
            duration_in_seconds: if duration_in_seconds > 0.0 {
                Some(duration_in_seconds)
            } else {
                None
            },
            warnings,
            request: Some(TranscriptionRequest { body: None }),
            response: transcription_response(timestamp, &self.model_id, response_headers, raw_body),
            provider_metadata: None,
        })
    }
}
