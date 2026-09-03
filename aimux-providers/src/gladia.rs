//! Gladia transcription (STT) provider.
//!
//! Aligned with Vercel AI SDK `createGladia` / `GladiaTranscriptionModel`
//! (`reference/ai/packages/gladia/src/gladia-transcription-model.ts`).
//!
//! Gladia uses a three-step async pattern:
//! 1. POST `/v2/upload` with multipart form (audio file) → returns `audio_url`
//! 2. POST `/v2/pre-recorded` with JSON body (audio_url + options) → returns `result_url`
//! 3. GET `result_url` polling until status is `done`

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::retry;
use aimux_core::shared::{SharedProviderMetadata, Warning};
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::{
    HttpBody, HttpRequest, MultipartForm, load_api_key, media_type_to_extension, sleep_or_abort,
    without_trailing_slash,
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

#[derive(Debug, Clone)]
pub struct GladiaConfig {
    pub api_key: String,
    pub base_url: String,
    pub headers: Option<HashMap<String, String>>,
}

impl GladiaConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.gladia.io".to_string(),
            headers: None,
        }
    }

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    #[must_use]
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Create from the `GLADIA_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when the environment variable is not
    /// set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "GLADIA_API_KEY", "Gladia")?;
        Ok(Self::new(api_key))
    }
}

pub struct GladiaProvider {
    config: GladiaConfig,
}

impl GladiaProvider {
    #[must_use]
    pub fn new(config: GladiaConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn transcription(&self, model_id: &str) -> GladiaTranscriptionModel {
        GladiaTranscriptionModel::new(model_id.to_string(), self.config.clone())
    }
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
    config: GladiaConfig,
}

impl GladiaTranscriptionModel {
    #[must_use]
    pub fn new(model_id: String, config: GladiaConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
        if let Some(ref config_headers) = self.config.headers {
            for (k, v) in config_headers {
                headers.insert(k.clone(), v.clone());
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn upload_url(&self) -> String {
        format!("{}/v2/upload", self.config.base_url)
    }

    fn initiate_url(&self) -> String {
        format!("{}/v2/pre-recorded", self.config.base_url)
    }
}

#[async_trait]
impl TranscriptionModel for GladiaTranscriptionModel {
    fn provider(&self) -> &str {
        "gladia"
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

        let headers = self.build_headers(options.headers.as_ref());

        // Core wraps the whole `do_generate` in one retry (RFC-0031 §6.2), so
        // without per-stage retries here, a transient failure in a later
        // stage would replay every earlier stage — re-uploading the audio to
        // retry an initiate or poll failure. Each stage gets its own retry
        // against the provider's configured retry settings instead; an
        // exhausted inner retry returns `AiMuxError::Retry`, which the outer
        // Core retry passes through unchanged rather than re-wrapping
        // (`retry_with_exponential_backoff`'s `Err(AiMuxError::Retry(_))`
        // arm), so `do_generate` is never replayed either.
        let retries = retry::prepare_retries(
            options.max_retries,
            self.retry_config(),
            options.abort_signal.clone(),
        );

        // Step 1: Upload audio.
        let mut form = MultipartForm::new();
        form.file("audio", &filename, &options.media_type, &audio_bytes)?;
        let (body_bytes, content_type) = form.finish();

        let resp = retries
            .retry(|| {
                aimux_provider_utils::post_to_api(
                    HttpRequest::new(
                        self.upload_url(),
                        headers
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                        options,
                    ),
                    HttpBody::Bytes(body_bytes.clone(), content_type.clone()),
                    aimux_provider_utils::create_json_response_handler(),
                    gladia_failed_response_handler(),
                )
            })
            .await?;

        let upload: GladiaUploadResponse = resp.value;

        // Step 2: Initiate transcription.
        let mut body = Map::new();
        body.insert("audio_url".to_string(), json!(upload.audio_url));

        // Parse provider options.
        if let Some(ref po) = options.provider_options
            && let Some(gladia) = po.get("gladia")
        {
            // Forward all provider options as-is (the API uses snake_case).
            if let Some(obj) = gladia.as_object() {
                for (k, v) in obj {
                    body.insert(k.clone(), v.clone());
                }
            }
        }

        let body = Value::Object(body);
        let resp = retries
            .retry(|| {
                aimux_provider_utils::post_json_to_api(
                    HttpRequest::new(
                        self.initiate_url(),
                        headers
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                        options,
                    ),
                    body.clone(),
                    aimux_provider_utils::create_json_response_handler(),
                    gladia_failed_response_handler(),
                )
            })
            .await?;

        let init: GladiaInitResponse = resp.value;

        // Step 3: Poll for result.
        let mut raw_body: Value;
        let mut response_headers: HashMap<String, String>;
        loop {
            sleep_or_abort(
                std::time::Duration::from_millis(100),
                options.abort_signal.as_ref(),
            )
            .await?;

            // AI SDK polls result_url with validateUrl: true and
            // credentialedOrigin = the API origin: the target is validated
            // and headers survive only while it stays on base_url's origin.
            let resp = retries
                .retry(|| {
                    aimux_provider_utils::get_from_api(
                        HttpRequest {
                            url: init.result_url.clone(),
                            headers: headers
                                .iter()
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect(),

                            abort_signal: options.abort_signal.clone(),
                            call_id: None,
                            recording_context: None,
                            response_timeout: None,
                            max_json_response_bytes: None,
                            validate_url: true,
                            trusted_origin: Some(self.config.base_url.clone()),
                            credentialed_origin: Some(self.config.base_url.clone()),
                        },
                        aimux_provider_utils::create_json_response_handler::<GladiaResultResponse>(
                        ),
                        gladia_failed_response_handler(),
                    )
                })
                .await?;

            response_headers = resp.response_headers;
            raw_body = resp.raw_value.unwrap_or(Value::Null);
            let parsed = resp.value;

            if parsed.status == "done" {
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
                    md.insert("gladia".to_string(), raw_body.clone());
                    Some(md)
                };
                return Ok(TranscriptionResult {
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
                });
            }

            if parsed.status == "error" {
                return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                    status_code: Some(200),
                    provider_code: Some(parsed.status.clone()),
                    response_body: Some(raw_body.to_string()),
                    ..ApiCallError::new(
                        "Transcription job failed",
                        init.result_url.clone(),
                        serde_json::json!({}),
                    )
                })));
            }
        }
    }
}
