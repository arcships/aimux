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

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::error::ApiCallError;
use aimux_core::shared::Warning;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};
use aimux_provider_utils::{MultipartForm, media_type_to_extension};

use crate::shared::{
    Credential, EndpointConfig, POLL_INTERVAL_MS_KEY, PollStep, poll_interval_ms, poll_until,
    provider_headers, retry_download,
};

fn revai_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error");
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Rev.ai request failed")
                .to_string(),
            provider_code: error.and_then(|value| value.get("code")).and_then(
                |value| match value {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                },
            ),
        }
    })
}

// ── Config ──────────────────────────────────────────────────────────────────

/// Milliseconds between two polls of a job
/// (`providerOptions.revai.pollIntervalMs` overrides it for one call).
const POLL_INTERVAL_MS: u64 = 100;
/// How many times a job is polled before the call gives up (ten minutes at the
/// default interval).
const MAX_POLL_ATTEMPTS: u32 = 6_000;

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://api.rev.ai";
const API_KEY_ENV_VAR: &str = "REVAI_API_KEY";
const DEFAULT_NAME: &str = "revai";

/// Settings of [`create_revai`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct RevaiProviderSettings {
    /// Base URL for the API calls. Default `https://api.rev.ai`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `REVAI_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.transcription"`).
    /// Default `"revai"`. The providerOptions key stays `revai`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for RevaiProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RevaiProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Create a Rev.ai provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_revai(settings: RevaiProviderSettings) -> Result<RevaiProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(RevaiProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Rev.ai"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_revai` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn revai() -> &'static RevaiProvider {
    static DEFAULT: OnceLock<RevaiProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_revai(RevaiProviderSettings::default())
            .expect("default Rev.ai settings are always valid")
    })
}

/// A Rev.ai provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct RevaiProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl RevaiProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
        )
    }

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

fn audio_input_to_bytes(audio: &AudioInput) -> Result<Vec<u8>, AiMuxError> {
    match audio {
        AudioInput::Binary(bytes) => Ok(bytes.clone()),
        AudioInput::Base64(b64) => {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                .map_err(|e| AiMuxError::InvalidArgument(format!("invalid base64: {e}")))
        }
    }
}

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
            return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(200),
                provider_code: Some("failed".to_string()),
                response_body,
                ..ApiCallError::new(
                    "Failed to submit transcription job to Rev.ai",
                    submit_url,
                    serde_json::json!({}),
                )
            })));
        }

        let job_id = submit_response.id.ok_or_else(|| {
            AiMuxError::InvalidResponseData(
                "Rev.ai job submission did not return an id".to_string(),
            )
        })?;
        let submission_language = submit_response.language;
        let poll_interval = Duration::from_millis(poll_interval_ms(
            options::revai_options(options.provider_options.as_ref()),
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
                    Some("failed") => Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                        status_code: Some(200),
                        provider_code: Some("failed".to_string()),
                        response_body,
                        ..ApiCallError::new(
                            "Transcription job failed",
                            poll_url.clone(),
                            serde_json::json!({}),
                        )
                    }))),
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

        let timestamp = chrono::Utc::now().to_rfc3339();

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
