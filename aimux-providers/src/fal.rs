//! Fal transcription (STT) provider.
//!
//! Aligned with Vercel AI SDK `createFal` / `FalTranscriptionModel`
//! (`reference/ai/packages/fal/src/fal-transcription-model.ts`).
//!
//! Fal uses an async queue pattern: POST to submit, then GET to poll until
//! the result is ready. The audio is sent as a base64 data URL.
//!
//! [`create_fal`] takes [`FalProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`FalProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `FAL_KEY`.
//! [`fal()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;
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
use aimux_provider_utils::HttpRequest;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::shared::{
    EndpointConfig, POLL_INTERVAL_MS_KEY, PollStep, is_poll_control_key, poll_interval_ms,
    poll_until, retry_download,
};
use aimux_provider_utils::{AuthScheme, Credential, ProviderHeaders};

/// fal errors are FastAPI-style: `{"detail": "..."}` or
/// `{"detail": [{"loc": [...], "msg": "...", "type": "..."}]}` where `type`
/// is the machine code (https://fal.ai/docs/model-apis/errors).
///
/// fal-image-model.ts reads a validation error as one `loc.join('.'): msg`
/// line per entry, and any other error as its `message` (fal-error.ts); the
/// two are handled the same way here, so a validation error keeps every
/// entry instead of the first.
fn fal_error_parts(data: &Value) -> aimux_provider_utils::ProviderErrorParts {
    let detail = data.get("detail");
    let entries = detail.and_then(Value::as_array);
    let message = entries
        .filter(|entries| !entries.is_empty())
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    let loc = entry
                        .get("loc")
                        .and_then(Value::as_array)
                        .map(|loc| {
                            loc.iter()
                                .map(|part| match part {
                                    Value::String(part) => part.clone(),
                                    other => other.to_string(),
                                })
                                .collect::<Vec<_>>()
                                .join(".")
                        })
                        .unwrap_or_default();
                    let msg = entry.get("msg").and_then(Value::as_str).unwrap_or_default();
                    format!("{loc}: {msg}")
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .or_else(|| detail.and_then(Value::as_str).map(str::to_owned))
        .or_else(|| {
            data.get("error")
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| {
            data.get("message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "Unknown fal error".to_string());
    aimux_provider_utils::ProviderErrorParts {
        message,
        provider_code: entries
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.get("type"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

fn fal_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(fal_error_parts)
}

/// Whether a failed status poll only says the job has not finished: its body
/// is `{"detail": "Request is still in progress"}`, whatever the status code
/// (the status `failedResponseHandler` of fal-transcription-model.ts and
/// fal-video-model.ts).
fn is_in_progress(error: &AiMuxError) -> bool {
    let AiMuxError::ApiCall(detail) = error else {
        return false;
    };
    detail
        .response_body
        .as_deref()
        .and_then(|body| serde_json::from_str::<Value>(body).ok())
        .is_some_and(|body| body["detail"] == "Request is still in progress")
}

// ── Config ──────────────────────────────────────────────────────────────────

/// Milliseconds between two polls of a queued transcription request
/// (`providerOptions.fal.pollIntervalMs` overrides it for one call).
const POLL_INTERVAL_MS: u64 = 100;
/// How many times a queued transcription request is polled before the call
/// gives up (ten minutes at the default interval).
const MAX_POLL_ATTEMPTS: u32 = 6_000;
/// Milliseconds between two attempts to download a generated image
/// (`providerOptions.fal.pollIntervalMs` overrides it for one call).
const DOWNLOAD_RETRY_INTERVAL_MS: u64 = 1_000;

pub(crate) mod options;

/// The default `base_url` (`createFal`'s `defaultBaseURL`). Upstream uses it
/// for the image model; the transcription and video models hard-code the
/// queue host themselves (see [`DEFAULT_QUEUE_BASE_URL`]).
pub(crate) const DEFAULT_BASE_URL: &str = "https://fal.run";
/// The host the transcription and video models talk to, which upstream
/// hard-codes in `fal-transcription-model.ts` and `fal-video-model.ts`
/// rather than reading `baseURL`.
pub(crate) const DEFAULT_QUEUE_BASE_URL: &str = "https://queue.fal.run";
pub(crate) const API_KEY_ENV_VAR: &str = "FAL_KEY";
const DEFAULT_NAME: &str = "fal";

/// Settings of [`create_fal`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct FalProviderSettings {
    /// Base URL for the API calls. Default `https://fal.run` for images and
    /// `https://queue.fal.run` for transcription and video, as upstream; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `FAL_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.transcription"`, `"{name}.video"`, `"{name}.image"`).
    /// Default `"fal"`. The providerOptions key stays `fal`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for FalProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FalProviderSettings")
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

/// Create a Fal provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_fal(settings: FalProviderSettings) -> Result<FalProvider, AiMuxError> {
    let explicit_base_url = match settings.base_url.as_deref() {
        Some(url) => Some(validate_base_url(url)?),
        None => None,
    };
    Ok(FalProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url: explicit_base_url,
        headers: ProviderHeaders::new(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Fal"),
            AuthScheme::Scheme("Key"),
            Vec::new(),
            settings.headers,
        )
        .with_user_agent(
            crate::shared::aisdk_pins::FAL.name,
            crate::shared::aisdk_pins::FAL.version,
        ),
        fetch: Some(crate::shared::package_fetch(
            settings.fetch,
            crate::shared::aisdk_pins::FAL,
        )),
    })
}

/// The default provider: `create_fal` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn fal() -> &'static FalProvider {
    static DEFAULT: OnceLock<FalProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_fal(FalProviderSettings::default()).expect("default Fal settings are always valid")
    })
}

/// A Fal provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct FalProvider {
    name: String,
    /// The caller's `base_url`; `None` leaves each model on its own default.
    base_url: Option<String>,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
}

impl FalProvider {
    /// The endpoint of one model method: the caller's `base_url`, or the
    /// host that model defaults to (upstream reads `baseURL` for images
    /// only and hard-codes the queue host in the other two).
    fn model_config(&self, method: &str) -> EndpointConfig {
        let default = match method {
            "image" => DEFAULT_BASE_URL,
            _ => DEFAULT_QUEUE_BASE_URL,
        };
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.as_deref().unwrap_or(default).to_string(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// A transcription (STT) model (e.g. `"wizper"`); `provider()` is `"{name}.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> FalTranscriptionModel {
        FalTranscriptionModel::from_config(model_id.to_string(), self.model_config("transcription"))
    }

    /// A video model (e.g. `"fal-ai/kling-video"`); `provider()` is `"{name}.video"`.
    #[must_use]
    pub fn video(&self, model_id: &str) -> FalVideoModel {
        FalVideoModel::from_config(model_id.to_string(), self.model_config("video"))
    }

    /// An image model (e.g. `"fal-ai/flux/schnell"`); `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> FalImageModel {
        FalImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }
}

impl ::aimux_core::Provider for FalProvider {
    fn language_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::LanguageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "languageModel"))
    }

    fn embedding_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::ImageModel>, AiMuxError> {
        Ok(::std::sync::Arc::new(self.image(model_id)))
    }

    fn transcription_model(
        &self,
        model_id: &str,
    ) -> Option<Result<::std::sync::Arc<dyn ::aimux_core::TranscriptionModel>, AiMuxError>> {
        Some(Ok(::std::sync::Arc::new(self.transcription(model_id))))
    }

    fn video_model(
        &self,
        model_id: &str,
    ) -> Option<Result<::std::sync::Arc<dyn ::aimux_core::VideoModel>, AiMuxError>> {
        Some(Ok(::std::sync::Arc::new(self.video(model_id))))
    }
}

// ── Response schema ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct FalJobResponse {
    request_id: String,
}

/// The video queue's answer (fal-video-model.ts `falJobResponseSchema`):
/// both fields nullish, and only `response_url` is used.
#[derive(Debug, Deserialize)]
struct FalVideoJobResponse {
    #[serde(default)]
    response_url: Option<String>,
}

/// The status poll of a video (fal-video-model.ts fetchStatus). Its URL comes
/// from the queue response body, so it goes through the SSRF guard, and the
/// caller headers (the `Key` credential among them) are sent only while it
/// stays on the origin that was submitted to: `credentialedOrigin` and
/// `trustedOrigin` are both the submit URL.
fn status_request(mut request: HttpRequest, submit_url: &str) -> HttpRequest {
    request.validate_url = true;
    request.credentialed_origin = Some(submit_url.to_string());
    request.trusted_origin = Some(submit_url.to_string());
    request
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

// ── Image model ──────────────────────────────────────────────────────────────

use aimux_core::image_model::{
    ImageCallOptions, ImageModel, ImageOutputs, ImageResponse, ImageResult,
};

/// `falImageModelOptionsSchema` (fal-image-model-options.ts), checked
/// only: a loose object whose known keys, camelCase and their deprecated
/// snake_case forms, are `.nullish()` with the ranges and enums below. The
/// request is built from the raw namespace, as upstream's transform keeps
/// every unknown key.
#[derive(Debug, Deserialize)]
#[allow(
    dead_code,
    reason = "validated only; the body is built from the raw namespace"
)]
struct FalImageOptions {
    #[serde(default, rename = "imageUrl")]
    image_url: Option<String>,
    #[serde(default, rename = "maskUrl")]
    mask_url: Option<String>,
    #[serde(default, rename = "guidanceScale", deserialize_with = "guidance_scale")]
    guidance_scale: Option<f64>,
    #[serde(
        default,
        rename = "numInferenceSteps",
        deserialize_with = "num_inference_steps"
    )]
    num_inference_steps: Option<f64>,
    #[serde(default, rename = "enableSafetyChecker")]
    enable_safety_checker: Option<bool>,
    #[serde(default, rename = "outputFormat")]
    output_format: Option<FalOutputFormat>,
    #[serde(default, rename = "syncMode")]
    sync_mode: Option<bool>,
    #[serde(default)]
    strength: Option<f64>,
    #[serde(default)]
    acceleration: Option<FalAcceleration>,
    #[serde(
        default,
        rename = "safetyTolerance",
        deserialize_with = "safety_tolerance"
    )]
    safety_tolerance: Option<f64>,
    #[serde(default, rename = "useMultipleImages")]
    use_multiple_images: Option<bool>,
    #[serde(default, rename = "image_url")]
    image_url_snake: Option<String>,
    #[serde(default, rename = "mask_url")]
    mask_url_snake: Option<String>,
    #[serde(
        default,
        rename = "guidance_scale",
        deserialize_with = "guidance_scale"
    )]
    guidance_scale_snake: Option<f64>,
    #[serde(
        default,
        rename = "num_inference_steps",
        deserialize_with = "num_inference_steps"
    )]
    num_inference_steps_snake: Option<f64>,
    #[serde(default, rename = "enable_safety_checker")]
    enable_safety_checker_snake: Option<bool>,
    #[serde(default, rename = "output_format")]
    output_format_snake: Option<FalOutputFormat>,
    #[serde(default, rename = "sync_mode")]
    sync_mode_snake: Option<bool>,
    #[serde(
        default,
        rename = "safety_tolerance",
        deserialize_with = "safety_tolerance"
    )]
    safety_tolerance_snake: Option<f64>,
}

/// `outputFormat`: `z.enum(['jpeg', 'png'])`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FalOutputFormat {
    Jpeg,
    Png,
}

/// `acceleration`: `z.enum(['none', 'regular', 'high'])`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FalAcceleration {
    #[serde(rename = "none")]
    Off,
    Regular,
    High,
}

/// A nullish number within `min..=max`.
fn number_in<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
    min: f64,
    max: f64,
) -> Result<Option<f64>, D::Error> {
    match Option::<f64>::deserialize(deserializer)? {
        Some(value) if !(min..=max).contains(&value) => Err(serde::de::Error::custom(format!(
            "expected a number from {min} to {max}"
        ))),
        value => Ok(value),
    }
}

/// `guidanceScale`: `z.number().min(1).max(20)`.
fn guidance_scale<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    number_in(deserializer, 1.0, 20.0)
}

/// `numInferenceSteps`: `z.number().min(1).max(50)`.
fn num_inference_steps<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    number_in(deserializer, 1.0, 50.0)
}

/// `safetyTolerance`: `z.enum(['1', ..., '6']).or(z.number().min(1).max(6))`.
fn safety_tolerance<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(level)) if ["1", "2", "3", "4", "5", "6"].contains(&level.as_str()) => {
            Ok(level.parse().ok())
        }
        Some(Value::Number(level))
            if level
                .as_f64()
                .is_some_and(|level| (1.0..=6.0).contains(&level)) =>
        {
            Ok(level.as_f64())
        }
        Some(_) => Err(serde::de::Error::custom(
            "expected '1' to '6' or a number from 1 to 6",
        )),
    }
}

/// The image options with a deprecated snake_case form, in the order the
/// schema's transform maps them: the camelCase key and the body key, which
/// is the snake_case one.
const FAL_IMAGE_RENAMED_KEYS: [(&str, &str); 8] = [
    ("imageUrl", "image_url"),
    ("maskUrl", "mask_url"),
    ("guidanceScale", "guidance_scale"),
    ("numInferenceSteps", "num_inference_steps"),
    ("enableSafetyChecker", "enable_safety_checker"),
    ("outputFormat", "output_format"),
    ("syncMode", "sync_mode"),
    ("safetyTolerance", "safety_tolerance"),
];

/// Models with an inpainting endpoint, which take a mask
/// (fal-image-model.ts `supportsMaskInputs`).
const FAL_MASK_MODELS: [&str; 2] = [
    "fal-ai/flux-general/inpainting",
    "fal-ai/flux-lora/inpainting",
];

/// Image-to-image models, which take an input file (fal-image-model.ts
/// `supportsFileInputs`).
const FAL_FILE_MODELS: [&str; 10] = [
    "fal-ai/flux-2/edit",
    "fal-ai/flux-pro/kontext",
    "fal-ai/flux-pro/kontext/max",
    "fal-ai/flux-general/image-to-image",
    "fal-ai/flux-general/inpainting",
    "fal-ai/flux-lora/image-to-image",
    "fal-ai/flux-lora/inpainting",
    "fal-ai/flux/dev/image-to-image",
    "fal-ai/flux/krea/image-to-image",
    "fal-ai/recraft/v3/image-to-image",
];

/// Text-to-image models that do not take an input file
/// (fal-image-model.ts `supportsFileInputs`).
const FAL_TEXT_ONLY_MODELS: [&str; 9] = [
    "bria/text-to-image/3.2",
    "fal-ai/bria/text-to-image/base",
    "fal-ai/bria/text-to-image/fast",
    "fal-ai/bria/text-to-image/hd",
    "fal-ai/bytedance/dreamina/v3.1/text-to-image",
    "fal-ai/flux-kontext-lora/text-to-image",
    "fal-ai/recraft/v3/text-to-image",
    "fal-ai/wan/v2.2-5b/text-to-image",
    "fal-ai/wan/v2.2-a14b/text-to-image",
];

/// An fal.ai image generation model.
pub struct FalImageModel {
    model_id: String,
    config: EndpointConfig,
}

impl FalImageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }

    fn endpoint_path(&self) -> String {
        format!("/{}", self.model_id)
    }

    fn convert_aspect_ratio_to_size(ar: &aimux_core::shared::AspectRatio) -> Option<Value> {
        Some(match (ar.width(), ar.height()) {
            (1, 1) => json!("square_hd"),
            (16, 9) => json!("landscape_16_9"),
            (9, 16) => json!("portrait_16_9"),
            (4, 3) => json!("landscape_4_3"),
            (3, 4) => json!("portrait_4_3"),
            (16, 10) => json!({ "width": 1280, "height": 800 }),
            (10, 16) => json!({ "width": 800, "height": 1280 }),
            (21, 9) => json!({ "width": 2560, "height": 1080 }),
            (9, 21) => json!({ "width": 1080, "height": 2560 }),
            _ => return None,
        })
    }
}

#[async_trait]
impl ImageModel for FalImageModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(1)
    }

    /// Whether the model takes an input file; `None` for an unknown model.
    fn supports_file_inputs(&self) -> Option<bool> {
        if FAL_FILE_MODELS.contains(&self.model_id.as_str()) {
            Some(true)
        } else if FAL_TEXT_ONLY_MODELS.contains(&self.model_id.as_str()) {
            Some(false)
        } else {
            None
        }
    }

    /// Whether the model takes a mask: the inpainting models do, a model
    /// known not to take files does not, an unknown one says nothing.
    fn supports_mask_inputs(&self) -> Option<bool> {
        if FAL_MASK_MODELS.contains(&self.model_id.as_str()) {
            Some(true)
        } else if self.supports_file_inputs().is_none() {
            None
        } else {
            Some(false)
        }
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let mut warnings: Vec<Warning> = Vec::new();
        // `image_size` is undefined, so not sent, without a size or for an
        // aspect ratio `convertAspectRatioToSize` does not map.
        let image_size = if let Some(s) = options.size {
            Some(json!({ "width": s.width(), "height": s.height() }))
        } else {
            options
                .aspect_ratio
                .and_then(|ar| Self::convert_aspect_ratio_to_size(&ar))
        };

        let mut body = Map::new();
        if let Some(ref p) = options.prompt {
            body.insert("prompt".into(), json!(p));
        }
        if let Some(seed) = options.seed {
            body.insert("seed".into(), json!(seed));
        }
        if let Some(image_size) = image_size {
            body.insert("image_size".into(), image_size);
        }
        body.insert("num_images".into(), json!(options.n));

        // fal-image-model.ts doGenerate: parseProviderOptions with
        // falImageModelOptionsSchema before the body is built.
        aimux_provider_utils::parse_provider_options::<FalImageOptions>(
            options::NAMESPACE,
            Some(&options.provider_options),
        )?;
        let fal_opts = options::fal_options(Some(&options.provider_options));

        if let Some(ref files) = options.files
            && !files.is_empty()
        {
            let multi = fal_opts
                .and_then(|o| o.get("useMultipleImages"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if multi {
                let uris: Vec<String> = files
                    .iter()
                    .map(aimux_provider_utils::convert_image_model_file_to_data_uri)
                    .collect();
                body.insert("image_urls".into(), json!(uris));
            } else {
                body.insert(
                    "image_url".into(),
                    json!(aimux_provider_utils::convert_image_model_file_to_data_uri(
                        &files[0]
                    )),
                );
                if files.len() > 1 {
                    warnings.push(Warning::Other {
                        message: "Multiple input images provided but useMultipleImages is not enabled. Only the first image will be used. Set providerOptions.fal.useMultipleImages to true for models that support multiple images (e.g., fal-ai/flux-2/edit)."
                            .into(),
                    });
                }
            }
        }
        if let Some(ref mask) = options.mask {
            body.insert(
                "mask_url".into(),
                json!(aimux_provider_utils::convert_image_model_file_to_data_uri(
                    mask
                )),
            );
        }

        if let Some(fal) = fal_opts {
            // The schema's transform (fal-image-model-options.ts): a
            // non-null snake_case key wins over its camelCase twin and is
            // reported as deprecated; a null known key is dropped. Every
            // other key passes through, `useMultipleImages` excepted.
            let set = |key: &str| fal.get(key).filter(|value| !value.is_null());
            let mut deprecated_keys = Vec::new();
            for (camel_key, snake_key) in FAL_IMAGE_RENAMED_KEYS {
                if let Some(value) = set(snake_key) {
                    deprecated_keys.push(snake_key);
                    body.insert(snake_key.to_string(), value.clone());
                } else if let Some(value) = set(camel_key) {
                    body.insert(snake_key.to_string(), value.clone());
                }
            }
            for key in ["strength", "acceleration"] {
                if let Some(value) = set(key) {
                    body.insert(key.to_string(), value.clone());
                }
            }
            for (key, value) in fal {
                let known = key == "strength"
                    || key == "acceleration"
                    || key == "useMultipleImages"
                    || FAL_IMAGE_RENAMED_KEYS
                        .iter()
                        .any(|(camel_key, snake_key)| key == camel_key || key == snake_key);
                if !known && key != "__deprecatedKeys" && !is_poll_control_key(key) {
                    body.insert(key.clone(), value.clone());
                }
            }
            if !deprecated_keys.is_empty() {
                let keys = deprecated_keys
                    .iter()
                    .map(|snake_key| {
                        let camel_key = FAL_IMAGE_RENAMED_KEYS
                            .iter()
                            .find(|(_, snake)| snake == snake_key)
                            .map_or(*snake_key, |(camel, _)| *camel);
                        format!("'{snake_key}' (use '{camel_key}')")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                warnings.push(Warning::Other {
                    message: format!(
                        "The following provider options use deprecated snake_case and will be removed in @ai-sdk/fal v2.0. Please use camelCase instead: {keys}"
                    ),
                });
            }
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url(&self.endpoint_path()), options),
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            fal_failed_response_handler(),
        )
        .await?;

        let rh = resp.response_headers;
        let rb: Value = resp.value;
        let target_images: Vec<Value> = if let Some(i) = rb.get("images").and_then(|v| v.as_array())
        {
            i.clone()
        } else if let Some(i) = rb.get("image") {
            vec![i.clone()]
        } else {
            vec![]
        };

        let mut downloaded: Vec<Vec<u8>> = Vec::new();
        for img in &target_images {
            if let Some(url) = img.get("url").and_then(|v| v.as_str()) {
                // images[].url comes from the queue result response body, so
                // it goes through the SSRF download guard.
                let ir = retry_download(
                    options.abort_signal.as_ref(),
                    Duration::from_millis(poll_interval_ms(
                        fal_opts,
                        POLL_INTERVAL_MS_KEY,
                        DOWNLOAD_RETRY_INTERVAL_MS,
                    )),
                    || {
                        aimux_provider_utils::get_from_api(
                            HttpRequest {
                                url: url.to_string(),
                                abort_signal: options.abort_signal.clone(),
                                validate_url: true,
                                trusted_origin: Some(exchange.base_url().to_string()),
                                credentialed_origin: Some(exchange.base_url().to_string()),
                                ..Default::default()
                            },
                            aimux_provider_utils::create_binary_response_handler(),
                            fal_failed_response_handler(),
                        )
                    },
                )
                .await?;
                downloaded.push(ir.value.to_vec());
            }
        }

        let mut metadata = aimux_core::shared::NamespaceMap::new();
        let mut fm = Map::new();
        // `nsfw` of image `index`: `has_nsfw_concepts?.[index] ??
        // nsfw_content_detected?.[index]`, left out when neither has it.
        let nsfw_flag = |key: &str, index: usize| {
            rb.get(key)
                .and_then(|flags| flags.get(index))
                .filter(|flag| !flag.is_null())
                .cloned()
        };
        let im: Vec<Value> = target_images
            .iter()
            .enumerate()
            .map(|(index, img)| {
                let mut e = Map::new();
                if let Some(o) = img.as_object() {
                    for (k, v) in o {
                        if !matches!(
                            k.as_str(),
                            "url" | "content_type" | "file_name" | "file_data" | "file_size"
                        ) {
                            e.insert(k.clone(), v.clone());
                        }
                    }
                }
                if let Some(v) = img.get("content_type") {
                    e.insert("contentType".into(), v.clone());
                }
                if let Some(v) = img.get("file_name") {
                    e.insert("fileName".into(), v.clone());
                }
                if let Some(v) = img.get("file_data") {
                    e.insert("fileData".into(), v.clone());
                }
                if let Some(v) = img.get("file_size") {
                    e.insert("fileSize".into(), v.clone());
                }
                if let Some(nsfw) = nsfw_flag("has_nsfw_concepts", index)
                    .or_else(|| nsfw_flag("nsfw_content_detected", index))
                {
                    e.insert("nsfw".into(), nsfw);
                }
                Value::Object(e)
            })
            .collect();
        fm.insert("images".into(), json!(im));
        if let Some(o) = rb.as_object() {
            for (k, v) in o {
                if !matches!(
                    k.as_str(),
                    "images" | "prompt" | "has_nsfw_concepts" | "nsfw_content_detected"
                ) {
                    fm.insert(k.clone(), v.clone());
                }
            }
        }
        metadata.insert(options::NAMESPACE.into(), fm);

        Ok(ImageResult {
            images: ImageOutputs::Binary(downloaded),
            warnings,
            provider_metadata: Some(metadata),
            response: ImageResponse {
                timestamp: Some(aimux_core::Timestamp::now()),
                model_id: Some(self.model_id.clone()),
                headers: Some(rh),
            },
            usage: None,
        })
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Video model
// ════════════════════════════════════════════════════════════════════════════

use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoModel, VideoOperationStart, VideoOperationStatus,
    VideoResponse, VideoResult,
};

/// Fal video generation model — implements `VideoModel`.
///
/// Aligned with Vercel AI SDK `FalVideoModel`
/// (`reference/ai/packages/fal/src/fal-video-model.ts`).
///
/// Uses the same async queue pattern as the transcription model; the status
/// polling is driven by Core via `do_status`.
pub struct FalVideoModel {
    model_id: String,
    config: EndpointConfig,
}

impl FalVideoModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }

    fn normalized_model_id(&self) -> String {
        self.model_id
            .strip_prefix("fal-ai/")
            .or_else(|| self.model_id.strip_prefix("fal/"))
            .unwrap_or(&self.model_id)
            .to_string()
    }

    fn submit_path(&self) -> String {
        format!("/fal-ai/{}", self.normalized_model_id())
    }
}

/// `falVideoModelOptionsSchema` (fal-video-model-options.ts), checked only:
/// a loose object whose five known keys are `.nullish()`. The body is built
/// from the raw namespace, which keeps every unknown key.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the body is built from the raw namespace"
)]
struct FalVideoOptions {
    #[serde(default, rename = "loop")]
    loop_video: Option<bool>,
    #[serde(default, deserialize_with = "motion_strength")]
    motion_strength: Option<f64>,
    #[serde(default)]
    resolution: Option<String>,
    #[serde(default)]
    negative_prompt: Option<String>,
    #[serde(default)]
    prompt_optimizer: Option<bool>,
}

/// `motionStrength`: `z.number().min(0).max(1)`.
fn motion_strength<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    number_in(deserializer, 0.0, 1.0)
}

/// The schema keys of [`FalVideoOptions`] and the body keys they are sent
/// as (fal-video-model.ts buildRequestBody).
const FAL_VIDEO_OPTION_KEYS: [(&str, &str); 5] = [
    ("loop", "loop"),
    ("motionStrength", "motion_strength"),
    ("resolution", "resolution"),
    ("negativePrompt", "negative_prompt"),
    ("promptOptimizer", "prompt_optimizer"),
];

fn video_file_to_data_uri(file: &VideoFile) -> Result<String, AiMuxError> {
    match file {
        VideoFile::Url { url, .. } => Ok(url.clone()),
        VideoFile::File { media_type, data } => Ok(format!(
            "data:{media_type};base64,{}",
            aimux_provider_utils::convert_to_base64(data)
        )),
    }
}

#[async_trait]
impl VideoModel for FalVideoModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_videos_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_start(
        &self,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStart, AiMuxError> {
        let warnings: Vec<Warning> = Vec::new();
        aimux_provider_utils::parse_provider_options::<FalVideoOptions>(
            options::NAMESPACE,
            Some(&options.provider_options),
        )?;
        let mut body = Map::new();

        if let Some(ref prompt) = options.prompt {
            body.insert("prompt".to_string(), json!(prompt));
        }
        if let Some(ref image) = options.image {
            body.insert(
                "image_url".to_string(),
                json!(video_file_to_data_uri(image)?),
            );
        }
        if let Some(ar) = options.aspect_ratio {
            body.insert("aspect_ratio".to_string(), json!(ar.to_string()));
        }
        // Upstream tests `if (options.duration)` and `if (options.seed)`: a 0
        // is not sent.
        if let Some(duration) = options.duration.filter(|&duration| duration != 0) {
            body.insert("duration".to_string(), json!(format!("{duration}s")));
        }
        if let Some(seed) = options.seed.filter(|&seed| seed != 0) {
            body.insert("seed".to_string(), json!(seed));
        }

        if let Some(fal) = options::fal_options(Some(&options.provider_options)) {
            // The schema keys first, a non-null value renamed; then every
            // other key as given, so an unknown key wins over a renamed one.
            for (key, wire_key) in FAL_VIDEO_OPTION_KEYS {
                if let Some(value) = fal.get(key).filter(|value| !value.is_null()) {
                    body.insert(wire_key.to_string(), value.clone());
                }
            }
            for (key, value) in fal {
                let known = FAL_VIDEO_OPTION_KEYS.iter().any(|(k, _)| k == key);
                if !known && !is_poll_control_key(key) {
                    body.insert(key.clone(), value.clone());
                }
            }
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        // Submit.
        let submit_url = exchange.url(&self.submit_path());
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(submit_url.clone(), options),
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            fal_failed_response_handler(),
        )
        .await?;

        // fal-video-model.ts submitToQueue: `if (!responseUrl)`, so an empty
        // URL fails like a missing one.
        let job: FalVideoJobResponse = resp.value;
        let response_url = job
            .response_url
            .filter(|url| !url.is_empty())
            .ok_or_else(|| {
                AiMuxError::invalid_response_data("No response URL returned from queue endpoint")
            })?;

        Ok(VideoOperationStart {
            operation: json!({ "responseUrl": response_url, "submitUrl": submit_url }),
            warnings,
            provider_metadata: None,
            response: VideoResponse {
                timestamp: Some(aimux_core::Timestamp::now()),
                model_id: Some(self.model_id.clone()),
                // fal-video-model.ts doStart reports `headers: undefined`:
                // the submission's headers are not carried.
                headers: None,
            },
        })
    }

    async fn do_status(
        &self,
        operation: &Value,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStatus, AiMuxError> {
        let response_url = operation
            .get("responseUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::invalid_argument(
                    "fal operation reference is missing responseUrl".to_string(),
                )
            })?;
        let submit_url = operation
            .get("submitUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::invalid_argument(
                    "fal operation reference is missing submitUrl".to_string(),
                )
            })?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::get_from_api(
            status_request(
                exchange.request(response_url.to_string(), options),
                submit_url,
            ),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            fal_failed_response_handler(),
        )
        .await;

        let resp = match resp {
            Ok(response) => response,
            Err(error) if is_in_progress(&error) => return Ok(VideoOperationStatus::Pending),
            Err(error) => return Err(error),
        };

        let response_headers = resp.response_headers;
        let raw_body = resp.value;

        // fal-video-model.ts buildResult reads `response.video.url` only: an
        // empty string counts as missing, and there is no top-level fallback.
        let video_url = raw_body
            .get("video")
            .and_then(|video| video.get("url"))
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty());
        let (Some(video), Some(video_url)) = (raw_body.get("video"), video_url) else {
            return Err(AiMuxError::invalid_response_data(
                "No video URL in response".to_string(),
            ));
        };

        // `mediaType` is the response's `content_type`, else `video/mp4`.
        let media_type = video
            .get("content_type")
            .and_then(Value::as_str)
            .filter(|content_type| !content_type.is_empty())
            .unwrap_or("video/mp4")
            .to_string();

        Ok(VideoOperationStatus::Completed(VideoResult {
            videos: vec![VideoData::Url {
                url: video_url.to_string(),
                media_type,
            }],
            warnings: Vec::new(),
            provider_metadata: Some(fal_video_metadata(&raw_body, video, video_url)),
            response: VideoResponse {
                timestamp: Some(aimux_core::Timestamp::now()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
        }))
    }
}

/// `providerMetadata.fal` of a completed video (`fal-video-model.ts`
/// buildResult): the video's fields under `videos`, then the job's `seed`,
/// `timings`, `has_nsfw_concepts` when present, and `prompt` when truthy.
fn fal_video_metadata(
    raw_body: &Value,
    video: &Value,
    video_url: &str,
) -> aimux_core::shared::SharedProviderMetadata {
    let field = |key: &str| video.get(key).cloned().unwrap_or(Value::Null);
    let mut fal = Map::new();
    fal.insert(
        "videos".into(),
        json!([{
            "url": video_url,
            "width": field("width"),
            "height": field("height"),
            "duration": field("duration"),
            "fps": field("fps"),
            "contentType": field("content_type"),
        }]),
    );
    if let Some(seed) = raw_body.get("seed").filter(|seed| !seed.is_null()) {
        fal.insert("seed".into(), seed.clone());
    }
    if let Some(timings) = raw_body.get("timings").filter(|timings| !timings.is_null()) {
        fal.insert("timings".into(), timings.clone());
    }
    if let Some(nsfw) = raw_body
        .get("has_nsfw_concepts")
        .filter(|nsfw| !nsfw.is_null())
    {
        fal.insert("has_nsfw_concepts".into(), nsfw.clone());
    }
    if let Some(prompt) = raw_body
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|prompt| !prompt.is_empty())
    {
        fal.insert("prompt".into(), json!(prompt));
    }

    let mut metadata = aimux_core::shared::NamespaceMap::new();
    metadata.insert(options::NAMESPACE.into(), fal);
    metadata
}

#[cfg(test)]
mod tests;
