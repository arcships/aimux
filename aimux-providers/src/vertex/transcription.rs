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

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::google::options::Namespace;
use crate::shared::EndpointConfig;
use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};

use super::ProjectLocationFn;

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
        .filter(|value| value.is_finite())
}

/// Convert a BCP 47 language tag (e.g. `"en-US"`) to an ISO 639-1 code
/// (e.g. `"en"`).
fn convert_bcp47_to_iso6391(value: &Option<String>) -> Option<String> {
    value
        .as_ref()
        .and_then(|s| s.split('-').next())
        .map(str::to_ascii_lowercase)
        .map(|s| match s.as_str() {
            "cmn" => "zh".to_string(),
            "iw" => "he".to_string(),
            "in" => "id".to_string(),
            "ji" => "yi".to_string(),
            _ => s,
        })
        .filter(|s| s.len() == 2 && s.bytes().all(|b| b.is_ascii_alphabetic()))
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
    config: EndpointConfig,
    project_location: ProjectLocationFn,
}

impl VertexTranscriptionModel {
    pub(crate) fn from_config(
        model_id: String,
        config: EndpointConfig,
        project_location: ProjectLocationFn,
    ) -> Self {
        Self {
            model_id,
            config,
            project_location,
        }
    }

    /// The Speech-to-Text v2 recognizer endpoint. A local `base_url` (a test
    /// server or a proxy on the loopback interface) is used directly; the
    /// real service has its own regional host.
    fn endpoint(&self, base_url: &str, project: &str, region: &str) -> String {
        if base_url.starts_with("http://127.0.0.1") || base_url.starts_with("http://localhost") {
            return format!(
                "{base_url}/v2/projects/{project}/locations/{region}/recognizers/_:recognize"
            );
        }
        let host = if region == "global" {
            "speech.googleapis.com".to_string()
        } else {
            format!("{region}-speech.googleapis.com")
        };
        format!("https://{host}/v2/projects/{project}/locations/{region}/recognizers/_:recognize")
    }
}

#[async_trait]
impl TranscriptionModel for VertexTranscriptionModel {
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

        // Parse provider options (`googleVertex`, then `google`).
        let target = (self.project_location)().await?;
        let mut region = target.location.clone();
        let mut language_codes: Vec<String> = vec!["auto".to_string()];
        let mut enable_word_time_offsets = true;
        let mut enable_automatic_punctuation = true;

        if let Some(ref po) = options.provider_options {
            for key in Namespace::Vertex.read_keys() {
                if let Some(gv) = po.get(*key) {
                    if let Some(r) = gv.get("region").and_then(|v| v.as_str()) {
                        region = r.to_string();
                    }
                    if let Some(lc) = gv.get("languageCodes").and_then(|v| v.as_array()) {
                        language_codes = lc
                            .iter()
                            .filter_map(|v| v.as_str().map(std::string::ToString::to_string))
                            .collect();
                    }
                    if let Some(v) = gv
                        .get("enableWordTimeOffsets")
                        .and_then(serde_json::Value::as_bool)
                    {
                        enable_word_time_offsets = v;
                    }
                    if let Some(v) = gv
                        .get("enableAutomaticPunctuation")
                        .and_then(serde_json::Value::as_bool)
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

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let url = self.endpoint(exchange.base_url(), &target.project, &region);

        // The recognizer lives on the Speech-to-Text host, not on the Vertex
        // host the endpoint resolves to; it is the host these credentials
        // are meant for.
        let mut request = exchange.request(url.clone(), options);
        request.credentialed_origin = Some(url);
        let resp = aimux_provider_utils::post_json_to_api(
            request,
            request_body.clone(),
            aimux_provider_utils::create_json_response_handler::<GoogleVertexResponse>(),
            crate::google::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let parsed = resp.value;

        let results = parsed.results.unwrap_or_default();

        // Concatenate transcript from all results.
        let text: String = results
            .iter()
            .map(|r| {
                r.alternatives
                    .as_ref()
                    .and_then(|a| a.first())
                    .and_then(|alt| alt.transcript.clone())
                    .unwrap_or_default()
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
                    .and_then(|alternatives| alternatives.first())
                    .into_iter()
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
