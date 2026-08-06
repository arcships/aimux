//! Rev.ai transcription (STT) provider.
//!
//! Aligned with Vercel AI SDK `createRevai` / `RevaiTranscriptionModel`
//! (`reference/ai/packages/revai/src/revai-transcription-model.ts`).
//!
//! Rev.ai uses an async job pattern:
//! 1. POST `/speechtotext/v1/jobs` with multipart form (media file + config JSON)
//! 2. GET `/speechtotext/v1/jobs/{id}` to poll until status is `transcribed`
//! 3. GET `/speechtotext/v1/jobs/{id}/transcript` to fetch the final transcript

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, MultipartForm, RetryConfig, load_api_key,
    media_type_to_extension, send, without_trailing_slash,
};

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RevaiConfig {
    pub api_key: String,
    pub base_url: String,
    pub headers: Option<HashMap<String, String>>,
}

impl RevaiConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.rev.ai".to_string(),
            headers: None,
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "REVAI_API_KEY", "Rev.ai")?;
        Ok(Self::new(api_key))
    }
}

pub struct RevaiProvider {
    config: RevaiConfig,
}

impl RevaiProvider {
    pub fn new(config: RevaiConfig) -> Self {
        Self { config }
    }

    pub fn transcription(&self, model_id: &str) -> RevaiTranscriptionModel {
        RevaiTranscriptionModel::new(model_id.to_string(), self.config.clone())
    }
}

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
    config: RevaiConfig,
}

impl RevaiTranscriptionModel {
    pub fn new(model_id: String, config: RevaiConfig) -> Self {
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

    fn jobs_url(&self) -> String {
        format!("{}/speechtotext/v1/jobs", self.config.base_url)
    }

    fn job_status_url(&self, job_id: &str) -> String {
        format!("{}/speechtotext/v1/jobs/{}", self.config.base_url, job_id)
    }

    fn transcript_url(&self, job_id: &str) -> String {
        format!(
            "{}/speechtotext/v1/jobs/{}/transcript",
            self.config.base_url, job_id
        )
    }
}

#[async_trait]
impl TranscriptionModel for RevaiTranscriptionModel {
    fn provider(&self) -> &str {
        "revai"
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
        let (body_bytes, content_type) = form.finish();

        let headers = self.build_headers(options.headers.as_ref());

        // Submit job.
        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.jobs_url(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                body: HttpBody::Bytes(body_bytes, content_type),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let submit_response: RevaiJobResponse =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

        if submit_response.status.as_deref() == Some("failed") {
            return Err(AiMuxError::Provider(
                "Failed to submit transcription job to Rev.ai".to_string(),
            ));
        }

        let job_id = submit_response.id.ok_or_else(|| {
            AiMuxError::Provider("Rev.ai job submission did not return an id".to_string())
        })?;
        let submission_language = submit_response.language;

        // Poll for completion.
        let job_status: RevaiJobResponse;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            let resp = send(
                HttpRequest {
                    method: HttpMethod::Get,
                    url: self.job_status_url(&job_id),
                    headers: headers
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    body: HttpBody::Empty,

                    abort_signal: options.abort_signal.clone(),
                    call_id: None,
                    recording_context: None,
                },
                RetryConfig::default(),
                &DEFAULT_ERROR_STRUCTURE,
            )
            .await?;

            let poll: RevaiJobResponse =
                serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

            if poll.status.as_deref() == Some("transcribed") {
                job_status = poll;
                break;
            }
            if poll.status.as_deref() == Some("failed") {
                return Err(AiMuxError::Provider("Transcription job failed".to_string()));
            }
        }
        let _ = job_status;

        // Fetch transcript.
        let resp = send(
            HttpRequest {
                method: HttpMethod::Get,
                url: self.transcript_url(&job_id),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                body: HttpBody::Empty,

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;
        let raw_body: Value = serde_json::from_slice(&resp.body).unwrap_or(Value::Null);
        let parsed: RevaiTranscriptResponse = serde_json::from_value(raw_body.clone())
            .map_err(|e| AiMuxError::Json(e.to_string()))?;

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
