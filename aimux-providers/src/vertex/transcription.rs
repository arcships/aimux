//! Google Vertex AI transcription (STT) model — implements `TranscriptionModel`.
//!
//! Aligned with Vercel AI SDK `GoogleVertexTranscriptionModel`
//! (`reference/ai/packages/google-vertex/src/google-vertex-transcription-model.ts`).
//!
//! Endpoint: `POST https://{host}/v2/projects/{project}/locations/{region}/recognizers/_:recognize`
//!
//! The Speech-to-Text v2 API accepts a JSON body with base64-encoded audio
//! `content` and a `config` object (model, language codes, auto decoding,
//! features). It returns `results[]` with `alternatives[]` containing
//! `transcript` and `words[]` with timing offsets.

use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::response::{ErrorStructure, parse_provider_error};

use super::VertexAuth;

/// Google-specific error structure: `{ "error": { "message": "..." } }`.
const GOOGLE_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["error", "message"],
    type_path: &["error", "status"],
};

// ── Response schema ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GoogleVertexWord {
    #[serde(default)]
    word: Option<String>,
    #[serde(default, rename = "startOffset")]
    start_offset: Option<String>,
    #[serde(default, rename = "endOffset")]
    end_offset: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleVertexAlternative {
    #[serde(default)]
    transcript: Option<String>,
    #[serde(default)]
    words: Option<Vec<GoogleVertexWord>>,
}

#[derive(Debug, Deserialize)]
struct GoogleVertexResult {
    #[serde(default)]
    alternatives: Option<Vec<GoogleVertexAlternative>>,
    #[serde(default, rename = "languageCode")]
    language_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleVertexMetadata {
    #[serde(default, rename = "totalBilledDuration")]
    total_billed_duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleVertexResponse {
    #[serde(default)]
    results: Option<Vec<GoogleVertexResult>>,
    #[serde(default)]
    metadata: Option<GoogleVertexMetadata>,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Parse a Speech-to-Text duration string like `"1.200s"` into seconds.
fn parse_duration_seconds(value: &Option<String>) -> Option<f64> {
    value
        .as_ref()
        .and_then(|s| s.trim_end_matches('s').parse::<f64>().ok())
}

/// Convert a BCP 47 language tag (e.g. `"en-US"`) to an ISO 639-1 code
/// (e.g. `"en"`).
fn convert_bcp47_to_iso6391(value: &Option<String>) -> Option<String> {
    value
        .as_ref()
        .and_then(|s| s.split('-').next())
        .filter(|s| s.len() == 2)
        .map(|s| s.to_string())
}

fn audio_input_to_base64(audio: &AudioInput) -> Result<String, AiMuxError> {
    match audio {
        AudioInput::Base64(s) => Ok(s.clone()),
        AudioInput::Binary(bytes) => Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            bytes,
        )),
    }
}

// ── Model ───────────────────────────────────────────────────────────────────

pub struct VertexTranscriptionModel {
    model_id: String,
    project: String,
    location: String,
    auth: VertexAuth,
    base_url: String,
    client: Client,
}

impl VertexTranscriptionModel {
    pub fn new(
        model_id: String,
        project: String,
        location: String,
        auth: VertexAuth,
        base_url: String,
        client: Client,
    ) -> Self {
        Self {
            model_id,
            project,
            location,
            auth,
            base_url,
            client,
        }
    }

    fn auth_header(&self) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        match &self.auth {
            VertexAuth::BearerToken(token) => {
                headers.insert("Authorization".to_string(), format!("Bearer {token}"));
            }
            VertexAuth::ApiKey(key) => {
                headers.insert("x-goog-api-key".to_string(), key.clone());
            }
        }
        headers
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = self.auth_header();
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self, region: &str) -> String {
        // When base_url is a test server (localhost / 127.0.0.1), use it
        // directly so wiremock can intercept the request.
        if self.base_url.starts_with("http://127.0.0.1")
            || self.base_url.starts_with("http://localhost")
        {
            return format!(
                "{}/v2/projects/{}/locations/{}/recognizers/_:recognize",
                self.base_url, self.project, region
            );
        }
        let host = if region == "global" {
            "speech.googleapis.com".to_string()
        } else {
            format!("{region}-speech.googleapis.com")
        };
        format!(
            "https://{host}/v2/projects/{}/locations/{}/recognizers/_:recognize",
            self.project, region
        )
    }
}

#[async_trait]
impl TranscriptionModel for VertexTranscriptionModel {
    fn provider(&self) -> &str {
        "google.vertex"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(
        &self,
        options: &TranscriptionCallOptions,
    ) -> Result<TranscriptionResult, AiMuxError> {
        let warnings: Vec<Warning> = Vec::new();

        // Parse provider options (may be under googleVertex, vertex, or google).
        let mut region = self.location.clone();
        let mut language_codes: Vec<String> = vec!["auto".to_string()];
        let mut enable_word_time_offsets = true;
        let mut enable_automatic_punctuation = true;

        if let Some(ref po) = options.provider_options {
            for key in &["googleVertex", "vertex", "google"] {
                if let Some(gv) = po.get(*key) {
                    if let Some(r) = gv.get("region").and_then(|v| v.as_str()) {
                        region = r.to_string();
                    }
                    if let Some(lc) = gv.get("languageCodes").and_then(|v| v.as_array()) {
                        language_codes = lc
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect();
                    }
                    if let Some(v) = gv.get("enableWordTimeOffsets").and_then(|v| v.as_bool()) {
                        enable_word_time_offsets = v;
                    }
                    if let Some(v) = gv
                        .get("enableAutomaticPunctuation")
                        .and_then(|v| v.as_bool())
                    {
                        enable_automatic_punctuation = v;
                    }
                    break;
                }
            }
        }

        let content = audio_input_to_base64(&options.audio)?;

        let request_body = json!({
            "config": {
                "model": self.model_id,
                "languageCodes": language_codes,
                "autoDecodingConfig": {},
                "features": {
                    "enableWordTimeOffsets": enable_word_time_offsets,
                    "enableAutomaticPunctuation": enable_automatic_punctuation,
                }
            },
            "content": content,
        });

        let headers = self.build_headers(options.headers.as_ref());
        let url = self.endpoint(&region);

        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .headers(reqwest::header::HeaderMap::from_iter(
                headers.iter().filter_map(|(k, v)| {
                    reqwest::header::HeaderName::try_from(k)
                        .ok()
                        .zip(reqwest::header::HeaderValue::try_from(v).ok())
                }),
            ))
            .json(&request_body)
            .send()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        let status = resp.status();
        let response_headers: HashMap<String, String> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        let raw_body: Value = if status.is_success() {
            let text = resp
                .text()
                .await
                .map_err(|e| AiMuxError::Http(e.to_string()))?;
            serde_json::from_str(&text).unwrap_or(Value::Null)
        } else {
            let text = resp.text().await.unwrap_or_default();
            return Err(parse_provider_error(
                status.as_u16(),
                &text,
                &GOOGLE_ERROR_STRUCTURE,
            ));
        };

        let parsed: GoogleVertexResponse =
            serde_json::from_value(raw_body.clone()).map_err(AiMuxError::Json)?;

        let results = parsed.results.unwrap_or_default();

        // Concatenate transcript from all results.
        let text: String = results
            .iter()
            .filter_map(|r| {
                r.alternatives
                    .as_ref()
                    .and_then(|a| a.first())
                    .and_then(|alt| alt.transcript.clone())
            })
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();

        // Collect word-level segments from all results.
        let segments: Vec<TranscriptionSegment> = results
            .iter()
            .flat_map(|r| {
                r.alternatives
                    .as_ref()
                    .into_iter()
                    .flatten()
                    .flat_map(|alt| alt.words.as_deref().unwrap_or(&[]))
                    .filter_map(|w| {
                        let word = w.word.as_ref()?;
                        let start = parse_duration_seconds(&w.start_offset)?;
                        let end = parse_duration_seconds(&w.end_offset)?;
                        Some(TranscriptionSegment {
                            text: word.clone(),
                            start_second: start,
                            end_second: end,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        let language =
            convert_bcp47_to_iso6391(&results.first().and_then(|r| r.language_code.clone()));

        let duration_in_seconds = parse_duration_seconds(
            &parsed
                .metadata
                .as_ref()
                .and_then(|m| m.total_billed_duration.clone()),
        );

        let timestamp = chrono::Utc::now().to_rfc3339();

        Ok(TranscriptionResult {
            text,
            segments,
            language,
            duration_in_seconds,
            warnings,
            request: Some(TranscriptionRequest {
                body: Some(request_body.to_string()),
            }),
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
