//! Hume speech (TTS) provider.
//!
//! Aligned with Vercel AI SDK `createHume`
//! (`reference/ai/packages/hume/src/hume-provider.ts`) and `HumeSpeechModel`
//! (`reference/ai/packages/hume/src/hume-speech-model.ts`).
//!
//! Endpoint: `POST https://api.hume.ai/v0/tts/file`
//!
//! Authentication: `X-Hume-Api-Key` header.
//!
//! The Hume TTS API accepts `utterances` (an array of `{ text, voice, speed,
//! description }` objects) and `format` (with a `type` field) in the request
//! body, and returns raw binary audio.
//!
//! `language` is not supported and emits a warning. The model ID is always an
//! empty string (Hume does not use model IDs for TTS).

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, RetryConfig, load_api_key, send};

// ── Config ───────────────────────────────────────────────────────────────────

/// Configuration for the Hume provider.
#[derive(Debug, Clone)]
pub struct HumeConfig {
    pub api_key: String,
    /// Base URL for the Hume API (no trailing slash). Defaults to
    /// `https://api.hume.ai`.
    pub base_url: String,
    /// Extra headers merged into every request.
    pub headers: Option<HashMap<String, String>>,
}

impl HumeConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.hume.ai".to_string(),
            headers: None,
        }
    }

    /// Override the base URL (for testing or proxies).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_string();
        self
    }

    /// Attach extra headers merged into every request.
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Create from environment variable `HUME_API_KEY`.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "HUME_API_KEY", "Hume")?;
        Ok(Self::new(api_key))
    }
}

// ── Provider ─────────────────────────────────────────────────────────────────

/// Hume provider — creates `HumeSpeechModel` instances.
pub struct HumeProvider {
    config: HumeConfig,
}

impl HumeProvider {
    pub fn new(config: HumeConfig) -> Self {
        Self { config }
    }

    /// Create a speech (TTS) model instance. Hume does not use model IDs for
    /// TTS, so the model ID is always an empty string.
    pub fn speech(&self) -> HumeSpeechModel {
        HumeSpeechModel::new(self.config.clone())
    }
}

// ── Speech model ─────────────────────────────────────────────────────────────

/// The default voice ID used when no voice is provided.
const DEFAULT_VOICE_ID: &str = "d8ab67c6-953d-4bd8-9370-8fa53a0f1453";

/// The output formats accepted by the Hume TTS API.
const SUPPORTED_OUTPUT_FORMATS: &[&str] = &["mp3", "pcm", "wav"];

/// A Hume speech (TTS) model.
pub struct HumeSpeechModel {
    /// Hume does not use model IDs for TTS; this is always an empty string.
    model_id: String,
    config: HumeConfig,
}

impl HumeSpeechModel {
    pub fn new(config: HumeConfig) -> Self {
        Self {
            model_id: String::new(),
            config,
        }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("X-Hume-Api-Key".to_string(), self.config.api_key.clone());
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
        format!("{}/v0/tts/file", self.config.base_url)
    }
}

#[async_trait]
impl SpeechModel for HumeSpeechModel {
    fn provider(&self) -> &str {
        "hume.speech"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let (body, warnings) = build_request(options)?;

        let headers: Vec<(String, String)> = self
            .build_headers(options.headers.as_ref())
            .into_iter()
            .collect();

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers,
                body: HttpBody::Json(Value::Object(body.clone())),
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;

        let audio_bytes = resp.body.to_vec();

        let timestamp = chrono::Utc::now().to_rfc3339();

        Ok(SpeechResult {
            audio: AudioData::Binary(audio_bytes),
            warnings,
            request: Some(SpeechRequest {
                body: Some(Value::Object(body)),
            }),
            response: SpeechResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
                body: None,
            },
            provider_metadata: None,
        })
    }
}

// ── Request builder ──────────────────────────────────────────────────────────

/// Build the Hume TTS request body and collect warnings.
///
/// Mirrors the TS `HumeSpeechModel.getArgs`:
/// - `voice` defaults to `"d8ab67c6-953d-4bd8-9370-8fa53a0f1453"`.
/// - `outputFormat` defaults to `"mp3"`; unsupported formats emit a warning.
/// - `speed` and `instructions` (as `description`) are placed inside the
///   utterance.
/// - `language` is not supported and emits a warning.
/// - Provider options (`hume` key) support a `context` field with either a
///   `generationId` or a list of `utterances`.
fn build_request(
    options: &SpeechCallOptions,
) -> Result<(Map<String, Value>, Vec<Warning>), AiMuxError> {
    let mut warnings = Vec::new();

    let voice = options.voice.as_deref().unwrap_or(DEFAULT_VOICE_ID);
    let output_format = options.output_format.as_deref().unwrap_or("mp3");

    // Build the utterance.
    let mut utterance = Map::new();
    utterance.insert("text".to_string(), json!(options.text));
    if let Some(speed) = options.speed {
        utterance.insert("speed".to_string(), json!(speed));
    }
    if let Some(ref instructions) = options.instructions {
        utterance.insert("description".to_string(), json!(instructions));
    }
    utterance.insert(
        "voice".to_string(),
        json!({ "id": voice, "provider": "HUME_AI" }),
    );

    let mut body = Map::new();
    body.insert("utterances".to_string(), json!([Value::Object(utterance)]));

    // Format.
    let mut format = Map::new();
    format.insert("type".to_string(), json!("mp3"));
    if SUPPORTED_OUTPUT_FORMATS.contains(&output_format) {
        format.insert("type".to_string(), json!(output_format));
    } else {
        warnings.push(Warning::Unsupported {
            feature: "outputFormat".to_string(),
            details: Some(format!(
                "Unsupported output format: {}. Using mp3 instead.",
                output_format
            )),
        });
    }
    body.insert("format".to_string(), Value::Object(format));

    // Add provider-specific options (context).
    let hume_options = parse_hume_provider_options(options.provider_options.as_ref());
    if let Some(ref opts) = hume_options
        && let Some(ref context) = opts.context
    {
        body.insert("context".to_string(), json!(context));
    }

    if let Some(ref language) = options.language {
        warnings.push(Warning::Unsupported {
            feature: "language".to_string(),
            details: Some(format!(
                "Hume speech models do not support language selection. Language parameter \"{}\" was ignored.",
                language
            )),
        });
    }

    Ok((body, warnings))
}

// ── Provider options parsing ─────────────────────────────────────────────────

/// Parsed `hume` speech provider options.
#[derive(Debug, Default)]
struct HumeSpeechProviderOptions {
    /// Context for the speech synthesis request — either a `generation_id` or
    /// a list of `utterances`. Stored as a pre-built JSON value ready for the
    /// request body.
    context: Option<Value>,
}

/// Extract Hume-specific speech options from the shared provider options.
fn parse_hume_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Option<HumeSpeechProviderOptions> {
    let provider_opts = options.and_then(|opts| opts.get("hume"))?;
    let opts = provider_opts.as_object()?;
    let context = opts.get("context").and_then(|c| c.as_object())?;

    // The context can be either { generationId: "..." } or { utterances: [...] }.
    // We map it to the API shape: { generation_id: "..." } or { utterances: [...] }.
    let resolved_context =
        if let Some(gen_id) = context.get("generationId").and_then(|v| v.as_str()) {
            json!({ "generation_id": gen_id })
        } else {
            let utterances = context.get("utterances").and_then(|v| v.as_array())?;
            let mapped: Vec<Value> = utterances
                .iter()
                .filter_map(|u| u.as_object())
                .map(|u| {
                    let mut m = Map::new();
                    if let Some(t) = u.get("text").and_then(|v| v.as_str()) {
                        m.insert("text".to_string(), json!(t));
                    }
                    if let Some(d) = u.get("description").and_then(|v| v.as_str()) {
                        m.insert("description".to_string(), json!(d));
                    }
                    if let Some(s) = u.get("speed").and_then(|v| v.as_f64()) {
                        m.insert("speed".to_string(), json!(s));
                    }
                    if let Some(ts) = u.get("trailingSilence").and_then(|v| v.as_f64()) {
                        m.insert("trailing_silence".to_string(), json!(ts));
                    }
                    if let Some(v) = u.get("voice") {
                        m.insert("voice".to_string(), v.clone());
                    }
                    Value::Object(m)
                })
                .collect();
            json!({ "utterances": mapped })
        };

    Some(HumeSpeechProviderOptions {
        context: Some(resolved_context),
    })
}
