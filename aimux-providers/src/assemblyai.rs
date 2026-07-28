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
use aimux_provider_utils::{load_api_key, without_trailing_slash};

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

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "ASSEMBLYAI_API_KEY", "AssemblyAI")?;
        Ok(Self::new(api_key))
    }
}

pub struct AssemblyAIProvider {
    config: AssemblyAIConfig,
    client: Client,
}

impl AssemblyAIProvider {
    pub fn new(config: AssemblyAIConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    pub fn transcription(&self, model_id: &str) -> AssemblyAITranscriptionModel {
        AssemblyAITranscriptionModel::new(
            model_id.to_string(),
            self.config.clone(),
            self.client.clone(),
        )
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
    client: Client,
}

impl AssemblyAITranscriptionModel {
    pub fn new(model_id: String, config: AssemblyAIConfig, client: Client) -> Self {
        Self {
            model_id,
            config,
            client,
        }
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
        let header_map =
            reqwest::header::HeaderMap::from_iter(headers.iter().filter_map(|(k, v)| {
                reqwest::header::HeaderName::try_from(k)
                    .ok()
                    .zip(reqwest::header::HeaderValue::try_from(v).ok())
            }));

        // Step 1: Upload audio.
        let resp = self
            .client
            .post(self.upload_url())
            .header("Content-Type", "application/octet-stream")
            .headers(header_map.clone())
            .body(audio_bytes)
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

        let upload: AssemblyAIUploadResponse =
            serde_json::from_str(&text).map_err(AiMuxError::Json)?;

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

        let resp = self
            .client
            .post(self.transcript_url())
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

        let submit: AssemblyAISubmitResponse =
            serde_json::from_str(&text).map_err(AiMuxError::Json)?;

        // Step 3: Poll for completion.
        let mut raw_body: Value;
        let mut response_headers: HashMap<String, String>;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            let resp = self
                .client
                .get(self.transcript_status_url(&submit.id))
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
            let parsed: AssemblyAITranscriptResponse =
                serde_json::from_value(raw_body.clone()).map_err(AiMuxError::Json)?;

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
                return Err(AiMuxError::Provider(format!(
                    "Transcription failed: {}",
                    parsed.error.unwrap_or_else(|| "Unknown error".to_string())
                )));
            }
        }
    }
}
