//! AssemblyAI transcription (STT) provider.
//!
//! Aligned with Vercel AI SDK `createAssemblyAI` / `AssemblyAITranscriptionModel`
//! (`reference/ai/packages/assemblyai/src/assemblyai-transcription-model.ts`).
//!
//! AssemblyAI uses a three-step async pattern:
//! 1. POST `/v2/upload` with raw audio bytes → returns `upload_url`
//! 2. POST `/v2/transcript` with JSON body (audio_url + options) → returns transcript `id`
//! 3. GET `/v2/transcript/{id}` polling until status is `completed`

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::error::ApiCallError;
use aimux_core::retry;
use aimux_core::shared::{SharedProviderMetadata, Warning};
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::{
    HttpBody, HttpRequest, load_api_key, sleep_or_abort, without_trailing_slash,
};

/// AssemblyAI errors: `{"error": "..."}` where `error` is a plain string
/// (e.g. `"Authentication error, API token missing/invalid."`). No machine
/// code is documented.
fn assemblyai_error_parts(data: &Value) -> aimux_provider_utils::ProviderErrorParts {
    let error = data.get("error");
    let message = error
        .and_then(Value::as_str)
        .or_else(|| {
            error
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        })
        .or_else(|| data.get("message").and_then(Value::as_str))
        .unwrap_or("AssemblyAI request failed")
        .to_string();
    aimux_provider_utils::ProviderErrorParts {
        message,
        provider_code: error
            .and_then(|value| value.get("code"))
            .and_then(|value| match value {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            }),
    }
}

fn assemblyai_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(assemblyai_error_parts)
}

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AssemblyAIConfig {
    pub api_key: String,
    pub base_url: String,
    pub headers: Option<HashMap<String, String>>,
}

impl AssemblyAIConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.assemblyai.com".to_string(),
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

    /// Create from the `ASSEMBLYAI_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when the environment variable is not
    /// set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "ASSEMBLYAI_API_KEY", "AssemblyAI")?;
        Ok(Self::new(api_key))
    }
}

pub struct AssemblyAIProvider {
    config: AssemblyAIConfig,
}

impl AssemblyAIProvider {
    #[must_use]
    pub fn new(config: AssemblyAIConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn transcription(&self, model_id: &str) -> AssemblyAITranscriptionModel {
        AssemblyAITranscriptionModel::new(model_id.to_string(), self.config.clone())
    }
}

// ── Response schema ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AssemblyAIUploadResponse {
    upload_url: String,
}

#[derive(Debug, Deserialize)]
struct AssemblyAISubmitResponse {
    id: String,
}

#[derive(Debug, Deserialize)]
struct AssemblyAIWord {
    text: String,
    start: f64,
    end: f64,
}

#[derive(Debug, Deserialize)]
struct AssemblyAITranscriptResponse {
    #[allow(dead_code)]
    id: String,
    status: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default, rename = "language_code")]
    language_code: Option<String>,
    #[serde(default)]
    words: Option<Vec<AssemblyAIWord>>,
    #[serde(default)]
    utterances: Option<Vec<Value>>,
    #[serde(default, rename = "sentiment_analysis_results")]
    sentiment_analysis_results: Option<Vec<Value>>,
    #[serde(default, rename = "entities")]
    entities: Option<Vec<Value>>,
    #[serde(default, rename = "content_safety_labels")]
    content_safety_labels: Option<Value>,
    #[serde(default, rename = "iab_categories_result")]
    iab_categories_result: Option<Value>,
    #[serde(default, rename = "auto_highlights_result")]
    auto_highlights_result: Option<Value>,
    #[serde(default, rename = "audio_duration")]
    audio_duration: Option<f64>,
    #[serde(default)]
    error: Option<String>,
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

pub struct AssemblyAITranscriptionModel {
    model_id: String,
    config: AssemblyAIConfig,
}

impl AssemblyAITranscriptionModel {
    #[must_use]
    pub fn new(model_id: String, config: AssemblyAIConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), self.config.api_key.clone());
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

    fn transcript_url(&self) -> String {
        format!("{}/v2/transcript", self.config.base_url)
    }

    fn transcript_status_url(&self, id: &str) -> String {
        format!("{}/v2/transcript/{}", self.config.base_url, id)
    }
}

#[async_trait]
impl TranscriptionModel for AssemblyAITranscriptionModel {
    fn provider(&self) -> &str {
        "assemblyai"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(
        &self,
        options: &TranscriptionCallOptions,
    ) -> Result<TranscriptionResult, AiMuxError> {
        let mut warnings: Vec<Warning> = Vec::new();

        let audio_bytes = audio_input_to_bytes(&options.audio)?;

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers.into_iter().collect();

        // Step 1: Upload audio.
        let resp = aimux_provider_utils::post_to_api(
            HttpRequest {
                url: self.upload_url(),
                headers: header_list.clone(),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
                response_timeout: None,
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
            HttpBody::Bytes(audio_bytes, "application/octet-stream".to_string()),
            aimux_provider_utils::create_json_response_handler(),
            assemblyai_failed_response_handler(),
        )
        .await?;

        let upload: AssemblyAIUploadResponse = resp.value;

        // Step 2: Submit transcript request.
        let mut body = Map::new();

        // Model selection.
        if self.model_id == "best" {
            body.insert("speech_model".to_string(), json!(self.model_id));
            warnings.push(Warning::Unsupported {
                feature: "model 'best'".to_string(),
                details: Some(
                    "The 'best' model is a legacy AssemblyAI model. Use 'universal-3-5-pro' instead."
                        .to_string(),
                ),
            });
        } else {
            body.insert("speech_models".to_string(), json!([self.model_id]));
        }

        body.insert("audio_url".to_string(), json!(upload.upload_url));

        // Parse and forward provider options.
        if let Some(ref po) = options.provider_options
            && let Some(aai) = po.get("assemblyai")
            && let Some(obj) = aai.as_object()
        {
            for (k, v) in obj {
                body.insert(k.clone(), v.clone());
            }
        }

        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: self.transcript_url(),
                headers: header_list.clone(),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
                response_timeout: None,
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            assemblyai_failed_response_handler(),
        )
        .await?;

        let submit: AssemblyAISubmitResponse = resp.value;
        let retries = retry::prepare_retries(
            options.max_retries,
            self.retry_config(),
            options.abort_signal.clone(),
        );

        // Step 3: Poll for completion.
        let mut raw_body: Value;
        let mut response_headers: HashMap<String, String>;
        loop {
            sleep_or_abort(
                std::time::Duration::from_millis(100),
                options.abort_signal.as_ref(),
            )
            .await?;

            let poll_url = self.transcript_status_url(&submit.id);
            let resp = retries
                .retry(|| {
                    aimux_provider_utils::get_from_api(
                        HttpRequest {
                            url: poll_url.clone(),
                            headers: header_list.clone(),

                            abort_signal: options.abort_signal.clone(),
                            call_id: None,
                            recording_context: None,
                            response_timeout: None,
                            validate_url: false,
                            trusted_origin: None,
                            credentialed_origin: None,
                        },
                        aimux_provider_utils::create_json_response_handler::<
                            AssemblyAITranscriptResponse,
                        >(),
                        assemblyai_failed_response_handler(),
                    )
                })
                .await?;

            response_headers = resp.response_headers.unwrap_or_default();

            raw_body = resp.raw_value.unwrap_or(Value::Null);
            let parsed = resp.value;

            if parsed.status == "completed" {
                // Build segments from words (timestamps are in milliseconds).
                let segments: Vec<TranscriptionSegment> = parsed
                    .words
                    .as_ref()
                    .map(|words| {
                        words
                            .iter()
                            .map(|w| TranscriptionSegment {
                                text: w.text.clone(),
                                start_second: w.start / 1000.0,
                                end_second: w.end / 1000.0,
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let language = parsed.language_code.clone();
                let duration_in_seconds = parsed.audio_duration.or_else(|| {
                    parsed
                        .words
                        .as_ref()
                        .and_then(|w| w.last())
                        .map(|w| w.end / 1000.0)
                });

                let text = parsed.text.unwrap_or_default();

                // Build provider metadata from extra fields.
                let mut provider_metadata: Option<SharedProviderMetadata> = None;
                let mut md = HashMap::new();
                let mut aai_meta = Map::new();
                if parsed.utterances.is_some() {
                    aai_meta.insert(
                        "utterances".to_string(),
                        raw_body.get("utterances").cloned().unwrap_or(Value::Null),
                    );
                }
                if parsed.sentiment_analysis_results.is_some() {
                    aai_meta.insert(
                        "sentimentAnalysisResults".to_string(),
                        raw_body
                            .get("sentiment_analysis_results")
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                }
                if parsed.entities.is_some() {
                    aai_meta.insert(
                        "entities".to_string(),
                        raw_body.get("entities").cloned().unwrap_or(Value::Null),
                    );
                }
                if parsed.content_safety_labels.is_some() {
                    aai_meta.insert(
                        "contentSafetyLabels".to_string(),
                        raw_body
                            .get("content_safety_labels")
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                }
                if parsed.iab_categories_result.is_some() {
                    aai_meta.insert(
                        "iabCategoriesResult".to_string(),
                        raw_body
                            .get("iab_categories_result")
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                }
                if parsed.auto_highlights_result.is_some() {
                    aai_meta.insert(
                        "autoHighlightsResult".to_string(),
                        raw_body
                            .get("auto_highlights_result")
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                }
                if !aai_meta.is_empty() {
                    md.insert("assemblyai".to_string(), Value::Object(aai_meta));
                    provider_metadata = Some(md);
                }

                let timestamp = chrono::Utc::now().to_rfc3339();

                return Ok(TranscriptionResult {
                    text,
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
                    provider_code: Some("error".to_string()),
                    response_body: Some(raw_body.to_string()),
                    ..ApiCallError::new(
                        format!(
                            "Transcription failed: {}",
                            parsed.error.unwrap_or_else(|| "Unknown error".to_string())
                        ),
                        poll_url,
                        serde_json::json!({}),
                    )
                })));
            }
        }
    }
}
