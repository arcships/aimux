//! OpenAI transcription (STT) model — implements the `TranscriptionModel` trait.
//!
//! Aligned with Vercel AI SDK `OpenAITranscriptionModel`
//! (`reference/ai/packages/openai/src/transcription/openai-transcription-model.ts`).
//!
//! Endpoint: `POST {base_url}/audio/transcriptions` (multipart form-data)
//!
//! The OpenAI transcription API accepts `model`, `file`, `response_format`,
//! `language`, `prompt`, `temperature`, and `timestamp_granularities` as
//! multipart form fields and returns a JSON body with `text`, optional
//! `segments`, `words`, `language`, and `duration`.
//!
//! Realtime transcription models (`gpt-realtime-whisper*`) stream over
//! WebSocket and do not support the REST endpoint; `do_generate` returns
//! [`AiMuxError::Unsupported`] for those model IDs. Streaming (`do_stream`)
//! is not implemented in the Rust port.

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
use aimux_provider_utils::{MultipartForm, media_type_to_extension};

use super::OpenAIConfig;

// ── Language map ────────────────────────────────────────────────────────────

/// Maps OpenAI's full language names (e.g. `"english"`) to ISO 639-1 codes
/// (e.g. `"en"`). Source: OpenAI speech-to-text supported languages.
fn language_name_to_code(name: &str) -> Option<&'static str> {
    match name {
        "afrikaans" => Some("af"),
        "arabic" => Some("ar"),
        "armenian" => Some("hy"),
        "azerbaijani" => Some("az"),
        "belarusian" => Some("be"),
        "bosnian" => Some("bs"),
        "bulgarian" => Some("bg"),
        "catalan" => Some("ca"),
        "chinese" => Some("zh"),
        "croatian" => Some("hr"),
        "czech" => Some("cs"),
        "danish" => Some("da"),
        "dutch" => Some("nl"),
        "english" => Some("en"),
        "estonian" => Some("et"),
        "finnish" => Some("fi"),
        "french" => Some("fr"),
        "galician" => Some("gl"),
        "german" => Some("de"),
        "greek" => Some("el"),
        "hebrew" => Some("he"),
        "hindi" => Some("hi"),
        "hungarian" => Some("hu"),
        "icelandic" => Some("is"),
        "indonesian" => Some("id"),
        "italian" => Some("it"),
        "japanese" => Some("ja"),
        "kannada" => Some("kn"),
        "kazakh" => Some("kk"),
        "korean" => Some("ko"),
        "latvian" => Some("lv"),
        "lithuanian" => Some("lt"),
        "macedonian" => Some("mk"),
        "malay" => Some("ms"),
        "marathi" => Some("mr"),
        "maori" => Some("mi"),
        "nepali" => Some("ne"),
        "norwegian" => Some("no"),
        "persian" => Some("fa"),
        "polish" => Some("pl"),
        "portuguese" => Some("pt"),
        "romanian" => Some("ro"),
        "russian" => Some("ru"),
        "serbian" => Some("sr"),
        "slovak" => Some("sk"),
        "slovenian" => Some("sl"),
        "spanish" => Some("es"),
        "swahili" => Some("sw"),
        "swedish" => Some("sv"),
        "tagalog" => Some("tl"),
        "tamil" => Some("ta"),
        "thai" => Some("th"),
        "turkish" => Some("tr"),
        "ukrainian" => Some("uk"),
        "urdu" => Some("ur"),
        "vietnamese" => Some("vi"),
        "welsh" => Some("cy"),
        _ => None,
    }
}

// ── Provider options ────────────────────────────────────────────────────────

/// OpenAI-specific transcription options parsed from `providerOptions.openai`.
#[derive(Debug, Clone, Default)]
struct OpenAITranscriptionOptions {
    /// Additional information to include in the transcription response.
    include: Option<Vec<String>>,
    /// The language of the input audio in ISO-639-1 format.
    language: Option<String>,
    /// An optional text to guide the model's style.
    prompt: Option<String>,
    /// The sampling temperature, between 0 and 1.
    temperature: Option<f64>,
    /// The timestamp granularities to populate (`word` / `segment`).
    timestamp_granularities: Option<Vec<String>>,
}

/// Parse provider options from the shared options map.
fn parse_openai_options(
    provider_options: Option<&HashMap<String, Value>>,
) -> OpenAITranscriptionOptions {
    let mut opts = OpenAITranscriptionOptions::default();
    if let Some(po) = provider_options
        && let Some(openai) = po.get("openai")
    {
        if let Some(include) = openai.get("include").and_then(|v| v.as_array()) {
            opts.include = Some(
                include
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
            );
        }
        if let Some(language) = openai.get("language").and_then(|v| v.as_str()) {
            opts.language = Some(language.to_string());
        }
        if let Some(prompt) = openai.get("prompt").and_then(|v| v.as_str()) {
            opts.prompt = Some(prompt.to_string());
        }
        if let Some(temp) = openai.get("temperature").and_then(|v| v.as_f64()) {
            opts.temperature = Some(temp);
        }
        if let Some(tg) = openai
            .get("timestampGranularities")
            .and_then(|v| v.as_array())
        {
            opts.timestamp_granularities = Some(
                tg.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
            );
        }
    }
    opts
}

// ── Response schema ─────────────────────────────────────────────────────────

/// A word with timing information in the OpenAI transcription response.
#[derive(Debug, Deserialize)]
struct OpenAIWord {
    word: String,
    start: f64,
    end: f64,
}

/// A segment with timing information in the OpenAI transcription response.
#[derive(Debug, Deserialize)]
struct OpenAISegment {
    text: String,
    start: f64,
    end: f64,
}

/// The OpenAI transcription API response body.
#[derive(Debug, Deserialize)]
struct OpenAITranscriptionResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    words: Option<Vec<OpenAIWord>>,
    #[serde(default)]
    segments: Option<Vec<OpenAISegment>>,
}

// ── Model ───────────────────────────────────────────────────────────────────

/// Realtime transcription model IDs stream over the realtime WebSocket and do
/// not support the REST transcription endpoint. Prefix matching keeps dated
/// snapshots (e.g. `gpt-realtime-whisper-2026-01-01`) working.
fn is_realtime_transcription_model_id(model_id: &str) -> bool {
    model_id == "gpt-realtime-whisper" || model_id.starts_with("gpt-realtime-whisper-")
}

/// Convert `AudioInput` to raw bytes.
fn audio_input_to_bytes(audio: &AudioInput) -> Result<Vec<u8>, AiMuxError> {
    match audio {
        AudioInput::Binary(bytes) => Ok(bytes.clone()),
        AudioInput::Base64(b64) => {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                .map_err(|e| AiMuxError::InvalidArgument(format!("invalid base64: {e}")))
        }
    }
}

/// An OpenAI-compatible transcription (STT) model.
///
/// Works with any OpenAI-compatible `/audio/transcriptions` endpoint.
pub struct OpenAITranscriptionModel {
    model_id: String,
    config: OpenAIConfig,
    client: Client,
}

impl OpenAITranscriptionModel {
    pub fn new(model_id: String, config: OpenAIConfig, client: Client) -> Self {
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
        if let Some(ref org) = self.config.org_id {
            headers.insert("OpenAI-Organization".to_string(), org.clone());
        }
        if let Some(ref project) = self.config.project {
            headers.insert("OpenAI-Project".to_string(), project.clone());
        }
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

    fn endpoint(&self) -> String {
        format!("{}/audio/transcriptions", self.config.base_url)
    }
}

#[async_trait]
impl TranscriptionModel for OpenAITranscriptionModel {
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
        if is_realtime_transcription_model_id(&self.model_id) {
            return Err(AiMuxError::Unsupported(format!(
                "non-streaming transcription with {}",
                self.model_id
            )));
        }

        let openai_options = parse_openai_options(options.provider_options.as_ref());
        let warnings: Vec<Warning> = Vec::new();

        // Build multipart form.
        let audio_bytes = audio_input_to_bytes(&options.audio)?;
        let file_extension = media_type_to_extension(&options.media_type);
        let filename = format!("audio.{file_extension}");

        let mut form = MultipartForm::new();
        form.text("model", &self.model_id);
        form.file("file", &filename, &options.media_type, &audio_bytes);

        // whisper-1 defaults to verbose_json to get segments.
        if self.model_id == "whisper-1" {
            form.text("response_format", "verbose_json");
        }

        // Provider-specific options.
        let is_gpt4o_transcribe_model =
            self.model_id == "gpt-4o-transcribe" || self.model_id == "gpt-4o-mini-transcribe";

        // For non-whisper models, set response_format based on model type.
        if self.model_id != "whisper-1" {
            let response_format = if is_gpt4o_transcribe_model {
                "json"
            } else {
                "verbose_json"
            };
            form.text("response_format", response_format);
        }

        if let Some(ref include) = openai_options.include {
            for item in include {
                form.text("include[]", item);
            }
        }
        if let Some(ref language) = openai_options.language {
            form.text("language", language);
        }
        if let Some(ref prompt) = openai_options.prompt {
            form.text("prompt", prompt);
        }
        if let Some(temperature) = openai_options.temperature {
            form.text("temperature", &temperature.to_string());
        }
        if let Some(ref tg) = openai_options.timestamp_granularities {
            for item in tg {
                form.text("timestamp_granularities[]", item);
            }
        }

        // Temperature default is 0 when provider options are present (matching
        // the TS schema's `.default(0)`).
        if options.provider_options.is_some() && openai_options.temperature.is_none() {
            form.text("temperature", "0");
        }

        let (body_bytes, content_type) = form.finish();

        let headers = self.build_headers(options.headers.as_ref());

        let resp = self
            .client
            .post(self.endpoint())
            .header("Content-Type", &content_type)
            .headers(reqwest::header::HeaderMap::from_iter(
                headers.iter().filter_map(|(k, v)| {
                    reqwest::header::HeaderName::try_from(k)
                        .ok()
                        .zip(reqwest::header::HeaderValue::try_from(v).ok())
                }),
            ))
            .body(body_bytes)
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

        let parsed: OpenAITranscriptionResponse =
            serde_json::from_value(raw_body.clone()).map_err(|e| AiMuxError::Json(e.to_string()))?;

        // Map language name to ISO 639-1 code.
        let language = parsed
            .language
            .as_deref()
            .and_then(language_name_to_code)
            .map(|s| s.to_string());

        // Segments: prefer `segments`, fall back to `words`.
        let segments: Vec<TranscriptionSegment> = if let Some(segs) = parsed.segments {
            segs.into_iter()
                .map(|s| TranscriptionSegment {
                    text: s.text,
                    start_second: s.start,
                    end_second: s.end,
                })
                .collect()
        } else if let Some(words) = parsed.words {
            words
                .into_iter()
                .map(|w| TranscriptionSegment {
                    text: w.word,
                    start_second: w.start,
                    end_second: w.end,
                })
                .collect()
        } else {
            Vec::new()
        };

        let timestamp = chrono::Utc::now().to_rfc3339();

        Ok(TranscriptionResult {
            text: parsed.text,
            segments,
            language,
            duration_in_seconds: parsed.duration,
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
