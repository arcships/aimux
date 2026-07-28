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
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderMetadata, Warning};
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::response::{DEFAULT_ERROR_STRUCTURE, parse_provider_error};
use aimux_provider_utils::{
    MultipartForm, load_api_key, media_type_to_extension, without_trailing_slash,
};

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

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "GLADIA_API_KEY", "Gladia")?;
        Ok(Self::new(api_key))
    }
}

pub struct GladiaProvider {
    config: GladiaConfig,
    client: Client,
}

impl GladiaProvider {
    pub fn new(config: GladiaConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    pub fn transcription(&self, model_id: &str) -> GladiaTranscriptionModel {
        GladiaTranscriptionModel::new(
            model_id.to_string(),
            self.config.clone(),
            self.client.clone(),
        )
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
    client: Client,
}

impl GladiaTranscriptionModel {
    pub fn new(model_id: String, config: GladiaConfig, client: Client) -> Self {
        Self {
            model_id,
            config,
            client,
        }
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
        let header_map =
            reqwest::header::HeaderMap::from_iter(headers.iter().filter_map(|(k, v)| {
                reqwest::header::HeaderName::try_from(k)
                    .ok()
                    .zip(reqwest::header::HeaderValue::try_from(v).ok())
            }));

        // Step 1: Upload audio.
        let mut form = MultipartForm::new();
        form.file("audio", &filename, &options.media_type, &audio_bytes);
        let (body_bytes, content_type) = form.finish();

        let resp = self
            .client
            .post(self.upload_url())
            .header("Content-Type", &content_type)
            .headers(header_map.clone())
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(parse_provider_error(
                status.as_u16(),
                &text,
                &DEFAULT_ERROR_STRUCTURE,
            ));
        }

        let upload: GladiaUploadResponse = serde_json::from_str(&text).map_err(AiMuxError::Json)?;

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

        let resp = self
            .client
            .post(self.initiate_url())
            .header("Content-Type", "application/json")
            .headers(header_map.clone())
            .json(&Value::Object(body))
            .send()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(parse_provider_error(
                status.as_u16(),
                &text,
                &DEFAULT_ERROR_STRUCTURE,
            ));
        }

        let init: GladiaInitResponse = serde_json::from_str(&text).map_err(AiMuxError::Json)?;

        // Step 3: Poll for result.
        let mut raw_body: Value;
        let mut response_headers: HashMap<String, String>;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            let resp = self
                .client
                .get(&init.result_url)
                .headers(header_map.clone())
                .send()
                .await
                .map_err(|e| AiMuxError::Http(e.to_string()))?;

            let status = resp.status();
            response_headers = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();

            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                return Err(parse_provider_error(
                    status.as_u16(),
                    &text,
                    &DEFAULT_ERROR_STRUCTURE,
                ));
            }

            raw_body = serde_json::from_str(&text).unwrap_or(Value::Null);
            let parsed: GladiaResultResponse =
                serde_json::from_value(raw_body.clone()).map_err(AiMuxError::Json)?;

            if parsed.status == "done" {
                let result = parsed.result.ok_or_else(|| {
                    AiMuxError::Provider("Transcription result is empty".to_string())
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
                return Err(AiMuxError::Provider("Transcription job failed".to_string()));
            }
        }
    }
}
