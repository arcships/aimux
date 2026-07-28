//! Deepgram transcription (STT) provider.
//!
//! Aligned with Vercel AI SDK `createDeepgram` / `DeepgramTranscriptionModel`
//! (`reference/ai/packages/deepgram/src/deepgram-transcription-model.ts`).
//!
//! Endpoint: `POST https://api.deepgram.com/v1/listen?{query_params}`
//!
//! The Deepgram API accepts raw audio bytes in the request body (with the
//! `Content-Type` header set to the audio media type) and query parameters for
//! model configuration. It returns a JSON body with `results.channels[0]`
//! containing the transcript, words, and detected language.

use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::response::{DEFAULT_ERROR_STRUCTURE, parse_provider_error};
use aimux_provider_utils::{load_api_key, without_trailing_slash};

// ── Config ──────────────────────────────────────────────────────────────────

/// Configuration for the Deepgram provider.
#[derive(Debug, Clone)]
pub struct DeepgramConfig {
    pub api_key: String,
    pub base_url: String,
    pub headers: Option<HashMap<String, String>>,
}

impl DeepgramConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.deepgram.com".to_string(),
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
        let api_key = load_api_key(None, "DEEPGRAM_API_KEY", "Deepgram")?;
        Ok(Self::new(api_key))
    }
}

/// Deepgram provider — creates `DeepgramTranscriptionModel` instances.
pub struct DeepgramProvider {
    config: DeepgramConfig,
    client: Client,
}

impl DeepgramProvider {
    pub fn new(config: DeepgramConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    pub fn transcription(&self, model_id: &str) -> DeepgramTranscriptionModel {
        DeepgramTranscriptionModel::new(
            model_id.to_string(),
            self.config.clone(),
            self.client.clone(),
        )
    }
}

// ── Provider options ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct DeepgramOptions {
    detect_entities: Option<bool>,
    detect_language: Option<bool>,
    filler_words: Option<bool>,
    language: Option<String>,
    punctuate: Option<bool>,
    redact: Option<Value>,
    search: Option<Value>,
    smart_format: Option<bool>,
    summarize: Option<bool>,
    topics: Option<Value>,
    utterances: Option<bool>,
    utt_split: Option<f64>,
    diarize: Option<bool>,
}

fn parse_deepgram_options(provider_options: Option<&HashMap<String, Value>>) -> DeepgramOptions {
    let mut opts = DeepgramOptions::default();
    if let Some(po) = provider_options
        && let Some(dg) = po.get("deepgram")
    {
        opts.detect_entities = dg.get("detectEntities").and_then(|v| v.as_bool());
        opts.detect_language = dg.get("detectLanguage").and_then(|v| v.as_bool());
        opts.filler_words = dg.get("fillerWords").and_then(|v| v.as_bool());
        opts.language = dg
            .get("language")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        opts.punctuate = dg.get("punctuate").and_then(|v| v.as_bool());
        opts.redact = dg.get("redact").cloned();
        opts.search = dg.get("search").cloned();
        opts.smart_format = dg.get("smartFormat").and_then(|v| v.as_bool());
        opts.summarize = dg.get("summarize").and_then(|v| v.as_bool());
        opts.topics = dg.get("topics").cloned();
        opts.utterances = dg.get("utterances").and_then(|v| v.as_bool());
        opts.utt_split = dg.get("uttSplit").and_then(|v| v.as_f64());
        opts.diarize = dg.get("diarize").and_then(|v| v.as_bool());
    }
    opts
}

// ── Response schema ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct DeepgramWord {
    word: String,
    start: f64,
    end: f64,
}

#[derive(Debug, Deserialize)]
struct DeepgramAlternative {
    transcript: String,
    #[serde(default)]
    words: Option<Vec<DeepgramWord>>,
}

#[derive(Debug, Deserialize)]
struct DeepgramChannel {
    #[serde(default, rename = "detected_language")]
    detected_language: Option<String>,
    alternatives: Vec<DeepgramAlternative>,
}

#[derive(Debug, Deserialize)]
struct DeepgramResults {
    #[serde(default)]
    channels: Option<Vec<DeepgramChannel>>,
}

#[derive(Debug, Deserialize)]
struct DeepgramMetadata {
    #[serde(default)]
    duration: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DeepgramResponse {
    #[serde(default)]
    metadata: Option<DeepgramMetadata>,
    #[serde(default)]
    results: Option<DeepgramResults>,
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

pub struct DeepgramTranscriptionModel {
    model_id: String,
    config: DeepgramConfig,
    client: Client,
}

impl DeepgramTranscriptionModel {
    pub fn new(model_id: String, config: DeepgramConfig, client: Client) -> Self {
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
            format!("Token {}", self.config.api_key),
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
}

#[async_trait]
impl TranscriptionModel for DeepgramTranscriptionModel {
    fn provider(&self) -> &str {
        "deepgram"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(
        &self,
        options: &TranscriptionCallOptions,
    ) -> Result<TranscriptionResult, AiMuxError> {
        let dg_options = parse_deepgram_options(options.provider_options.as_ref());
        let warnings: Vec<Warning> = Vec::new();

        // Build query parameters.
        let mut params = vec![("model".to_string(), self.model_id.clone())];

        // diarize defaults to true.
        let diarize = dg_options.diarize.unwrap_or(true);
        params.push(("diarize".to_string(), diarize.to_string()));

        if let Some(v) = dg_options.detect_entities {
            params.push(("detect_entities".to_string(), v.to_string()));
        }
        if let Some(v) = dg_options.detect_language {
            params.push(("detect_language".to_string(), v.to_string()));
        }
        if let Some(v) = dg_options.filler_words {
            params.push(("filler_words".to_string(), v.to_string()));
        }
        if let Some(ref v) = dg_options.language {
            params.push(("language".to_string(), v.clone()));
        }
        if let Some(v) = dg_options.punctuate {
            params.push(("punctuate".to_string(), v.to_string()));
        }
        if let Some(v) = dg_options.smart_format {
            params.push(("smart_format".to_string(), v.to_string()));
        }
        if let Some(v) = dg_options.summarize {
            params.push(("summarize".to_string(), v.to_string()));
        }
        if let Some(v) = dg_options.utterances {
            params.push(("utterances".to_string(), v.to_string()));
        }
        if let Some(v) = dg_options.utt_split {
            params.push(("utt_split".to_string(), v.to_string()));
        }
        if let Some(ref v) = dg_options.redact {
            params.push(("redact".to_string(), v.to_string()));
        }
        if let Some(ref v) = dg_options.search {
            params.push(("search".to_string(), v.to_string()));
        }
        if let Some(ref v) = dg_options.topics {
            params.push(("topics".to_string(), v.to_string()));
        }

        let query_string = params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");

        let url = format!("{}/v1/listen?{query_string}", self.config.base_url);

        let audio_bytes = audio_input_to_bytes(&options.audio)?;

        let headers = self.build_headers(options.headers.as_ref());

        let resp = self
            .client
            .post(&url)
            .header("Content-Type", &options.media_type)
            .headers(reqwest::header::HeaderMap::from_iter(
                headers.iter().filter_map(|(k, v)| {
                    reqwest::header::HeaderName::try_from(k)
                        .ok()
                        .zip(reqwest::header::HeaderValue::try_from(v).ok())
                }),
            ))
            .body(audio_bytes)
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
                &DEFAULT_ERROR_STRUCTURE,
            ));
        };

        let parsed: DeepgramResponse =
            serde_json::from_value(raw_body.clone()).map_err(AiMuxError::Json)?;

        let channel = parsed
            .results
            .as_ref()
            .and_then(|r| r.channels.as_ref())
            .and_then(|c| c.first());

        let text = channel
            .and_then(|ch| ch.alternatives.first())
            .map(|alt| alt.transcript.clone())
            .unwrap_or_default();

        let segments: Vec<TranscriptionSegment> = channel
            .and_then(|ch| ch.alternatives.first())
            .and_then(|alt| alt.words.as_ref())
            .map(|words| {
                words
                    .iter()
                    .map(|w| TranscriptionSegment {
                        text: w.word.clone(),
                        start_second: w.start,
                        end_second: w.end,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let language = channel.and_then(|ch| ch.detected_language.clone());

        let duration_in_seconds = parsed.metadata.as_ref().and_then(|m| m.duration);

        let timestamp = chrono::Utc::now().to_rfc3339();

        Ok(TranscriptionResult {
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
            provider_metadata: None,
        })
    }
}
