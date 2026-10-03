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

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::shared::{SharedProviderMetadata, Warning};
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};
use aimux_provider_utils::{HttpBody, MultipartForm, media_type_to_extension};

use crate::shared::{
    Credential, EndpointConfig, POLL_INTERVAL_MS_KEY, PollStep, is_poll_control_key,
    poll_interval_ms, poll_until, provider_headers,
};

fn gladia_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error");
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Gladia request failed")
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

/// Milliseconds between two polls of a transcription job
/// (`providerOptions.gladia.pollIntervalMs` overrides it for one call).
const POLL_INTERVAL_MS: u64 = 100;
/// How many times a transcription job is polled before the call gives up (ten
/// minutes at the default interval).
const MAX_POLL_ATTEMPTS: u32 = 6_000;

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://api.gladia.io";
const API_KEY_ENV_VAR: &str = "GLADIA_API_KEY";
const DEFAULT_NAME: &str = "gladia";

/// Settings of [`create_gladia`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct GladiaProviderSettings {
    /// Base URL for the API calls. Default `https://api.gladia.io`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `GLADIA_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.transcription"`).
    /// Default `"gladia"`. The providerOptions key stays `gladia`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for GladiaProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GladiaProviderSettings")
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

/// Create a Gladia provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_gladia(settings: GladiaProviderSettings) -> Result<GladiaProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(GladiaProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Gladia"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_gladia` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn gladia() -> &'static GladiaProvider {
    static DEFAULT: OnceLock<GladiaProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_gladia(GladiaProviderSettings::default())
            .expect("default Gladia settings are always valid")
    })
}

/// A Gladia provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct GladiaProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl GladiaProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

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

fn audio_input_to_bytes(audio: &AudioInput) -> Result<Vec<u8>, AiMuxError> {
    match audio {
        AudioInput::Binary(bytes) => Ok(bytes.clone()),
        AudioInput::Base64(b64) => {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                .map_err(|e| AiMuxError::InvalidArgument(format!("invalid base64: {e}")))
        }
    }
}

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
        let gladia_options = options::gladia_options(options.provider_options.as_ref());

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
        if let Some(obj) = gladia_options.and_then(Value::as_object) {
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
                    "error" => Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                        status_code: Some(200),
                        provider_code: Some(resp.value.status.clone()),
                        response_body: Some(
                            resp.raw_value.clone().unwrap_or(Value::Null).to_string(),
                        ),
                        ..ApiCallError::new(
                            "Transcription job failed",
                            init.result_url.clone(),
                            serde_json::json!({}),
                        )
                    }))),
                    _ => Ok(PollStep::Pending),
                }
            },
        )
        .await?;

        let response_headers = resp.response_headers;
        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let parsed = resp.value;

        let result = parsed.result.ok_or_else(|| {
            AiMuxError::InvalidResponseData(
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

        let timestamp = chrono::Utc::now().to_rfc3339();

        let provider_metadata: Option<SharedProviderMetadata> = {
            let mut md = HashMap::new();
            md.insert(options::NAMESPACE.to_string(), raw_body.clone());
            Some(md)
        };
        Ok(TranscriptionResult {
            text: result.transcription.full_transcript,
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
            provider_metadata,
        })
    }
}
