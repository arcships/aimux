//! LMNT speech (TTS) provider.
//!
//! Aligned with Vercel AI SDK `createLMNT`
//! (`reference/ai/packages/lmnt/src/lmnt-provider.ts`) and `LMNTSpeechModel`
//! (`reference/ai/packages/lmnt/src/lmnt-speech-model.ts`).
//!
//! Endpoint: `POST https://api.lmnt.com/v1/ai/speech/bytes`
//!
//! Authentication: `x-api-key` header.
//!
//! The LMNT TTS API accepts `model`, `text`, `voice`, `response_format`,
//! `speed`, and `language` in the request body, and returns raw binary audio.
//! Provider options (`lmnt` key) support `conversational`, `length`, `seed`,
//! `speed`, `temperature`, `topP`, `sampleRate`, and `format`.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};

use aimux_provider_utils::{HttpRequest, load_api_key};

fn lmnt_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error");
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("LMNT request failed")
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

// ── Config ───────────────────────────────────────────────────────────────────

/// Configuration for the LMNT provider.
#[derive(Debug, Clone)]
pub struct LMNTConfig {
    pub api_key: String,
    /// Base URL for the LMNT API (no trailing slash). Defaults to
    /// `https://api.lmnt.com`.
    pub base_url: String,
    /// Extra headers merged into every request.
    pub headers: Option<HashMap<String, String>>,
}

impl LMNTConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.lmnt.com".to_string(),
            headers: None,
        }
    }

    /// Override the base URL (for testing or proxies).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_string();
        self
    }

    /// Attach extra headers merged into every request.
    #[must_use]
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Create from environment variable `LMNT_API_KEY`.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when `LMNT_API_KEY` is not set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "LMNT_API_KEY", "LMNT")?;
        Ok(Self::new(api_key))
    }
}

// ── Provider ─────────────────────────────────────────────────────────────────

/// LMNT provider — creates `LMNTSpeechModel` instances.
pub struct LMNTProvider {
    config: LMNTConfig,
}

impl LMNTProvider {
    #[must_use]
    pub fn new(config: LMNTConfig) -> Self {
        Self { config }
    }

    /// Create a speech (TTS) model instance for the given model name (e.g.
    /// `"aurora"`).
    #[must_use]
    pub fn speech(&self, model_id: &str) -> LMNTSpeechModel {
        LMNTSpeechModel::new(model_id.to_string(), self.config.clone())
    }
}

// ── Speech model ─────────────────────────────────────────────────────────────

/// The default voice ID used when no voice is provided.
const DEFAULT_VOICE_ID: &str = "ava";

/// The output formats accepted by the LMNT TTS API.
const SUPPORTED_OUTPUT_FORMATS: &[&str] = &["mp3", "aac", "mulaw", "raw", "wav"];

/// An LMNT speech (TTS) model.
pub struct LMNTSpeechModel {
    model_id: String,
    config: LMNTConfig,
}

impl LMNTSpeechModel {
    #[must_use]
    pub fn new(model_id: String, config: LMNTConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("x-api-key".to_string(), self.config.api_key.clone());
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
        format!("{}/v1/ai/speech/bytes", self.config.base_url)
    }
}

#[async_trait]
impl SpeechModel for LMNTSpeechModel {
    fn provider(&self) -> &str {
        "lmnt.speech"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let (body, warnings) = build_request(options, &self.model_id)?;

        let headers = self.build_headers(options.headers.as_ref());

        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: self.endpoint(),
                headers: headers.into_iter().collect(),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            Value::Object(body.clone()),
            aimux_provider_utils::create_binary_response_handler(),
            lmnt_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers.unwrap_or_default();

        let audio_bytes = resp.value.to_vec();

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

/// Build the LMNT TTS request body and collect warnings.
///
/// Mirrors the TS `LMNTSpeechModel.getArgs`:
/// - `voice` defaults to `"ava"`.
/// - `response_format` defaults to `"mp3"`; unsupported formats emit a warning.
/// - `speed` and `language` are forwarded when present.
/// - Provider options (`lmnt` key) override the corresponding fields:
///   `conversational`, `length`, `seed`, `speed`, `temperature`, `topP`
///   (mapped to `top_p`), `sampleRate` (mapped to `sample_rate`), and `format`.
fn build_request(
    options: &SpeechCallOptions,
    model_id: &str,
) -> Result<(Map<String, Value>, Vec<Warning>), AiMuxError> {
    let mut warnings = Vec::new();

    let voice = options.voice.as_deref().unwrap_or(DEFAULT_VOICE_ID);
    let output_format = options.output_format.as_deref().unwrap_or("mp3");

    let mut body = Map::new();
    body.insert("model".to_string(), json!(model_id));
    body.insert("text".to_string(), json!(options.text));
    body.insert("voice".to_string(), json!(voice));
    body.insert("response_format".to_string(), json!("mp3"));
    if let Some(speed) = options.speed {
        body.insert("speed".to_string(), json!(speed));
    }

    if SUPPORTED_OUTPUT_FORMATS.contains(&output_format) {
        body.insert("response_format".to_string(), json!(output_format));
    } else {
        warnings.push(Warning::Unsupported {
            feature: "outputFormat".to_string(),
            details: Some(format!(
                "Unsupported output format: {output_format}. Using mp3 instead."
            )),
        });
    }

    // Add provider-specific options.
    let lmnt_options = parse_lmnt_provider_options(options.provider_options.as_ref());
    if let Some(ref opts) = lmnt_options {
        if let Some(conversational) = opts.conversational {
            body.insert("conversational".to_string(), json!(conversational));
        }
        if let Some(length) = opts.length {
            body.insert("length".to_string(), json!(length));
        }
        if let Some(seed) = opts.seed {
            body.insert("seed".to_string(), json!(seed));
        }
        if let Some(speed) = opts.speed {
            body.insert("speed".to_string(), json!(speed));
        }
        if let Some(temperature) = opts.temperature {
            body.insert("temperature".to_string(), json!(temperature));
        }
        if let Some(top_p) = opts.top_p {
            body.insert("top_p".to_string(), json!(top_p));
        }
        if let Some(sample_rate) = opts.sample_rate {
            body.insert("sample_rate".to_string(), json!(sample_rate));
        }
        // The TS test passes `providerOptions.lmnt.format` but the schema also
        // has a `format` field. If present, override `response_format`.
        if let Some(ref format) = opts.format {
            body.insert("response_format".to_string(), json!(format));
        }
    }

    if let Some(ref language) = options.language {
        body.insert("language".to_string(), json!(language));
    }

    Ok((body, warnings))
}

// ── Provider options parsing ─────────────────────────────────────────────────

/// Parsed `lmnt` speech provider options.
#[derive(Debug, Default)]
struct LMNTSpeechProviderOptions {
    conversational: Option<bool>,
    length: Option<f64>,
    seed: Option<u64>,
    speed: Option<f64>,
    temperature: Option<f64>,
    top_p: Option<f64>,
    sample_rate: Option<u32>,
    format: Option<String>,
}

/// Extract LMNT-specific speech options from the shared provider options.
fn parse_lmnt_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Option<LMNTSpeechProviderOptions> {
    let provider_opts = options.and_then(|opts| opts.get("lmnt"))?;
    let opts = provider_opts.as_object()?;

    Some(LMNTSpeechProviderOptions {
        conversational: opts
            .get("conversational")
            .and_then(serde_json::Value::as_bool),
        length: opts.get("length").and_then(serde_json::Value::as_f64),
        seed: opts.get("seed").and_then(serde_json::Value::as_u64),
        speed: opts.get("speed").and_then(serde_json::Value::as_f64),
        temperature: opts.get("temperature").and_then(serde_json::Value::as_f64),
        top_p: opts.get("topP").and_then(serde_json::Value::as_f64),
        sample_rate: opts
            .get("sampleRate")
            .and_then(serde_json::Value::as_u64)
            .map(|n| n as u32),
        format: opts
            .get("format")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
    })
}
