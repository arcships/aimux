//! Cartesia speech (TTS) provider.
//!
//! Aligned with Vercel AI SDK `createCartesia`
//! (`reference/ai/packages/cartesia/src/cartesia-provider.ts`) and
//! `CartesiaSpeechModel`
//! (`reference/ai/packages/cartesia/src/cartesia-speech-model.ts`).
//!
//! Endpoint: `POST https://api.cartesia.ai/tts/bytes`
//!
//! Authentication: `Authorization: Bearer {api_key}` header, plus a
//! `Cartesia-Version` header.
//!
//! The Cartesia TTS API accepts `model_id`, `transcript`, `voice` (with
//! `mode: "id"` and an `id`), and `output_format` (a nested object with
//! `container`, `encoding`/`bit_rate`, and `sample_rate`). It returns raw
//! binary audio.
//!
//! `instructions` is not supported and emits a warning. `speed` must be
//! between 0.6 and 1.5 (inclusive) or it is ignored with a warning.

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

// ── Constants ────────────────────────────────────────────────────────────────

/// The Cartesia API version sent with every request via the `Cartesia-Version`
/// header.
const CARTESIA_API_VERSION: &str = "2026-03-01";

/// The valid sample rates for Cartesia output.
const SAMPLE_RATES: &[u32] = &[8000, 16000, 22050, 24000, 44100, 48000];

// ── Config ───────────────────────────────────────────────────────────────────

/// Configuration for the Cartesia provider.
#[derive(Debug, Clone)]
pub struct CartesiaConfig {
    pub api_key: String,
    /// Base URL for the Cartesia API (no trailing slash). Defaults to
    /// `https://api.cartesia.ai`.
    pub base_url: String,
    /// The Cartesia API version (sent via the `Cartesia-Version` header).
    pub version: String,
    /// Extra headers merged into every request.
    pub headers: Option<HashMap<String, String>>,
}

impl CartesiaConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.cartesia.ai".to_string(),
            version: CARTESIA_API_VERSION.to_string(),
            headers: None,
        }
    }

    /// Override the base URL (for testing or proxies).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_string();
        self
    }

    /// Override the Cartesia API version.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Attach extra headers merged into every request.
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Create from environment variable `CARTESIA_API_KEY`.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "CARTESIA_API_KEY", "Cartesia")?;
        Ok(Self::new(api_key))
    }
}

// ── Provider ─────────────────────────────────────────────────────────────────

/// Cartesia provider — creates `CartesiaSpeechModel` instances.
pub struct CartesiaProvider {
    config: CartesiaConfig,
}

impl CartesiaProvider {
    pub fn new(config: CartesiaConfig) -> Self {
        Self { config }
    }

    /// Create a speech (TTS) model instance for the given model name (e.g.
    /// `"sonic-3.5"`).
    pub fn speech(&self, model_id: &str) -> CartesiaSpeechModel {
        CartesiaSpeechModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a transcription (STT) model instance for the given model name
    /// (e.g. `"best"`). Uses the `/stt` endpoint.
    pub fn transcription(&self, model_id: &str) -> CartesiaTranscriptionModel {
        CartesiaTranscriptionModel::new(model_id.to_string(), self.config.clone())
    }
}

// ── Speech model ─────────────────────────────────────────────────────────────

/// A Cartesia speech (TTS) model.
pub struct CartesiaSpeechModel {
    model_id: String,
    config: CartesiaConfig,
}

impl CartesiaSpeechModel {
    pub fn new(model_id: String, config: CartesiaConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
        headers.insert("Cartesia-Version".to_string(), self.config.version.clone());
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
        format!("{}/tts/bytes", self.config.base_url)
    }
}

#[async_trait]
impl SpeechModel for CartesiaSpeechModel {
    fn provider(&self) -> &str {
        "cartesia.speech"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let (body, warnings) = build_request(options, &self.model_id)?;

        let headers = self.build_headers(options.headers.as_ref());

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: headers.into_iter().collect(),
                body: HttpBody::Json(Value::Object(body.clone())),

                abort_signal: options.abort_signal.clone(),
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

// ── Output format resolution ─────────────────────────────────────────────────

/// The resolved Cartesia output format.
///
/// Mirrors the TS `CartesiaSpeechOutputFormat` discriminated union: MP3 output
/// carries a `bit_rate`; raw/WAV output carries an `encoding`.
#[derive(Debug, Clone)]
enum CartesiaOutputFormat {
    Mp3 {
        sample_rate: u32,
        bit_rate: u32,
    },
    RawOrWav {
        container: String,
        encoding: String,
        sample_rate: u32,
    },
}

impl CartesiaOutputFormat {
    /// Convert to a JSON object for the request body.
    fn to_json(&self) -> Value {
        match self {
            CartesiaOutputFormat::Mp3 {
                sample_rate,
                bit_rate,
            } => json!({
                "container": "mp3",
                "sample_rate": sample_rate,
                "bit_rate": bit_rate,
            }),
            CartesiaOutputFormat::RawOrWav {
                container,
                encoding,
                sample_rate,
            } => json!({
                "container": container,
                "encoding": encoding,
                "sample_rate": sample_rate,
            }),
        }
    }
}

/// The default output format: MP3 at 44100 Hz, 128000 bit/s.
fn default_output_format() -> CartesiaOutputFormat {
    CartesiaOutputFormat::Mp3 {
        sample_rate: 44100,
        bit_rate: 128000,
    }
}

/// Lookup table for known output format names.
fn lookup_format(name: &str) -> Option<CartesiaOutputFormat> {
    match name {
        "alaw" => Some(CartesiaOutputFormat::RawOrWav {
            container: "raw".to_string(),
            encoding: "pcm_alaw".to_string(),
            sample_rate: 8000,
        }),
        "mp3" => Some(default_output_format()),
        "mulaw" => Some(CartesiaOutputFormat::RawOrWav {
            container: "raw".to_string(),
            encoding: "pcm_mulaw".to_string(),
            sample_rate: 8000,
        }),
        "pcm" | "raw" => Some(CartesiaOutputFormat::RawOrWav {
            container: "raw".to_string(),
            encoding: "pcm_f32le".to_string(),
            sample_rate: 44100,
        }),
        "wav" => Some(CartesiaOutputFormat::RawOrWav {
            container: "wav".to_string(),
            encoding: "pcm_s16le".to_string(),
            sample_rate: 44100,
        }),
        _ => None,
    }
}

/// Resolve the output format from the `outputFormat` string and provider
/// options, collecting warnings for unsupported values.
///
/// Mirrors the TS `resolveOutputFormat` function.
fn resolve_output_format(
    output_format: &str,
    provider_options: Option<&CartesiaSpeechProviderOptions>,
    warnings: &mut Vec<Warning>,
) -> CartesiaOutputFormat {
    let lower = output_format.to_lowercase();
    let parts: Vec<&str> = lower.split('_').collect();
    let format_name = parts.first().copied().unwrap_or("");
    let sample_rate_text = parts.get(1).copied();
    let extra_parts: &[&str] = if parts.len() > 2 { &parts[2..] } else { &[] };

    let mapped = lookup_format(format_name);
    let mut resolved = mapped.clone().unwrap_or_else(|| {
        warnings.push(Warning::Unsupported {
            feature: "outputFormat".to_string(),
            details: Some(format!(
                "Unknown output format \"{}\". Falling back to mp3. Use providerOptions.cartesia to configure container, encoding, and sampleRate directly.",
                output_format
            )),
        });
        default_output_format()
    });

    // If the format was known and a sample rate suffix was provided, try to
    // parse it.
    if mapped.is_some()
        && let Some(srt) = sample_rate_text
    {
        let parsed_rate: Option<u32> = srt.parse().ok();
        if extra_parts.is_empty() {
            if let Some(rate) = parsed_rate {
                if SAMPLE_RATES.contains(&rate) {
                    set_sample_rate(&mut resolved, rate);
                } else {
                    warnings.push(Warning::Unsupported {
                            feature: "outputFormat".to_string(),
                            details: Some(format!(
                                "Unsupported Cartesia sample rate in output format \"{}\". Using {} Hz instead.",
                                output_format,
                                get_sample_rate(&resolved)
                            )),
                        });
                }
            } else {
                warnings.push(Warning::Unsupported {
                        feature: "outputFormat".to_string(),
                        details: Some(format!(
                            "Unsupported Cartesia sample rate in output format \"{}\". Using {} Hz instead.",
                            output_format,
                            get_sample_rate(&resolved)
                        )),
                    });
            }
        } else {
            warnings.push(Warning::Unsupported {
                    feature: "outputFormat".to_string(),
                    details: Some(format!(
                        "Unsupported Cartesia sample rate in output format \"{}\". Using {} Hz instead.",
                        output_format,
                        get_sample_rate(&resolved)
                    )),
                });
        }
    }

    let provider_container = provider_options.and_then(|o| o.container.as_deref());
    let provider_sample_rate = provider_options.and_then(|o| o.sample_rate);
    let provider_encoding = provider_options.and_then(|o| o.encoding.clone());
    let provider_bit_rate = provider_options.and_then(|o| o.bit_rate);

    let container = provider_container.unwrap_or_else(|| get_container(&resolved));
    let sample_rate = provider_sample_rate.unwrap_or_else(|| get_sample_rate(&resolved));

    if container == "mp3" {
        if provider_encoding.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "providerOptions.cartesia.encoding".to_string(),
                details: Some(
                    "Cartesia MP3 output does not accept an encoding. The encoding option was ignored."
                        .to_string(),
                ),
            });
        }

        let bit_rate = provider_bit_rate.unwrap_or_else(|| {
            if get_container(&resolved) == "mp3" {
                get_bit_rate(&resolved)
            } else {
                128000
            }
        });

        return CartesiaOutputFormat::Mp3 {
            sample_rate,
            bit_rate,
        };
    }

    // raw or wav
    if provider_bit_rate.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "providerOptions.cartesia.bitRate".to_string(),
            details: Some(
                "Cartesia raw and WAV output do not accept a bit rate. The bitRate option was ignored."
                    .to_string(),
            ),
        });
    }

    let encoding = provider_encoding.unwrap_or_else(|| {
        if get_container(&resolved) == "mp3" {
            if container == "wav" {
                "pcm_s16le".to_string()
            } else {
                "pcm_f32le".to_string()
            }
        } else {
            get_encoding(&resolved)
        }
    });

    CartesiaOutputFormat::RawOrWav {
        container: container.to_string(),
        encoding,
        sample_rate,
    }
}

fn get_container(fmt: &CartesiaOutputFormat) -> &str {
    match fmt {
        CartesiaOutputFormat::Mp3 { .. } => "mp3",
        CartesiaOutputFormat::RawOrWav { container, .. } => container,
    }
}

fn get_sample_rate(fmt: &CartesiaOutputFormat) -> u32 {
    match fmt {
        CartesiaOutputFormat::Mp3 { sample_rate, .. } => *sample_rate,
        CartesiaOutputFormat::RawOrWav { sample_rate, .. } => *sample_rate,
    }
}

fn get_bit_rate(fmt: &CartesiaOutputFormat) -> u32 {
    match fmt {
        CartesiaOutputFormat::Mp3 { bit_rate, .. } => *bit_rate,
        CartesiaOutputFormat::RawOrWav { .. } => 128000,
    }
}

fn get_encoding(fmt: &CartesiaOutputFormat) -> String {
    match fmt {
        CartesiaOutputFormat::Mp3 { .. } => "pcm_f32le".to_string(),
        CartesiaOutputFormat::RawOrWav { encoding, .. } => encoding.clone(),
    }
}

fn set_sample_rate(fmt: &mut CartesiaOutputFormat, rate: u32) {
    match fmt {
        CartesiaOutputFormat::Mp3 { sample_rate, .. } => *sample_rate = rate,
        CartesiaOutputFormat::RawOrWav { sample_rate, .. } => *sample_rate = rate,
    }
}

// ── Request builder ──────────────────────────────────────────────────────────

/// Build the Cartesia TTS request body and collect warnings.
///
/// Mirrors the TS `CartesiaSpeechModel.getArgs`:
/// - `voice` is required (returns an error if not set).
/// - `outputFormat` defaults to `"mp3"` and is resolved via
///   [`resolve_output_format`].
/// - `language` is forwarded when present (provider options override).
/// - `speed` must be between 0.6 and 1.5 (inclusive); out-of-range values emit
///   a warning. Provider options `speed` takes precedence.
/// - `instructions` is not supported and emits a warning.
fn build_request(
    options: &SpeechCallOptions,
    model_id: &str,
) -> Result<(Map<String, Value>, Vec<Warning>), AiMuxError> {
    let mut warnings = Vec::new();

    let voice = options.voice.as_deref().ok_or_else(|| {
        AiMuxError::InvalidArgument(
            "Cartesia speech models require a `voice` to be set.".to_string(),
        )
    })?;

    let cartesia_options = parse_cartesia_provider_options(options.provider_options.as_ref());

    let output_format_str = options.output_format.as_deref().unwrap_or("mp3");
    let output_format =
        resolve_output_format(output_format_str, cartesia_options.as_ref(), &mut warnings);

    let mut body = Map::new();
    body.insert("model_id".to_string(), json!(model_id));
    body.insert("transcript".to_string(), json!(options.text));
    body.insert("voice".to_string(), json!({ "mode": "id", "id": voice }));
    body.insert("output_format".to_string(), output_format.to_json());

    // Map generic language.
    if let Some(ref language) = options.language {
        body.insert("language".to_string(), json!(language));
    }

    // Provider-specific options override generic ones.
    if let Some(ref opts) = cartesia_options
        && let Some(ref language) = opts.language
    {
        body.insert("language".to_string(), json!(language));
    }

    // Speed: provider options take precedence over generic speed.
    let resolved_speed = cartesia_options
        .as_ref()
        .and_then(|o| o.speed)
        .or(options.speed);

    if let Some(speed) = resolved_speed {
        if (0.6..=1.5).contains(&speed) {
            let mut gen_config = Map::new();
            gen_config.insert("speed".to_string(), json!(speed));
            body.insert("generation_config".to_string(), Value::Object(gen_config));
        } else {
            warnings.push(Warning::Unsupported {
                feature: "speed".to_string(),
                details: Some(
                    "Cartesia speed must be between 0.6 and 1.5. The speed option was ignored."
                        .to_string(),
                ),
            });
        }
    }

    if options.instructions.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "instructions".to_string(),
            details: Some(
                "Cartesia speech models do not support instructions. Instructions parameter was ignored."
                    .to_string(),
            ),
        });
    }

    Ok((body, warnings))
}

// ── Provider options parsing ─────────────────────────────────────────────────

/// Parsed `cartesia` speech provider options.
#[derive(Debug, Default)]
struct CartesiaSpeechProviderOptions {
    container: Option<String>,
    encoding: Option<String>,
    sample_rate: Option<u32>,
    bit_rate: Option<u32>,
    speed: Option<f64>,
    language: Option<String>,
}

/// Extract Cartesia-specific speech options from the shared provider options.
fn parse_cartesia_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Option<CartesiaSpeechProviderOptions> {
    let provider_opts = options.and_then(|opts| opts.get("cartesia"))?;
    let opts = provider_opts.as_object()?;

    Some(CartesiaSpeechProviderOptions {
        container: opts
            .get("container")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        encoding: opts
            .get("encoding")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        sample_rate: opts
            .get("sampleRate")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32),
        bit_rate: opts
            .get("bitRate")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32),
        speed: opts.get("speed").and_then(|v| v.as_f64()),
        language: opts
            .get("language")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

// ════════════════════════════════════════════════════════════════════════════
// Transcription (STT) model
// ════════════════════════════════════════════════════════════════════════════

use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionRequest,
    TranscriptionResponse, TranscriptionResult, TranscriptionSegment,
};
use aimux_provider_utils::{MultipartForm, media_type_to_extension};
use serde::Deserialize;

/// Streaming transcription model IDs start with `ink-2` and only support the
/// WebSocket streaming endpoint, not the REST batch endpoint.
fn is_streaming_transcription_model_id(model_id: &str) -> bool {
    model_id == "ink-2" || model_id.starts_with("ink-2-")
}

/// Cartesia transcription response word.
#[derive(Debug, Deserialize)]
struct CartesiaTranscriptionWord {
    word: String,
    start: f64,
    end: f64,
}

/// Cartesia transcription API response body.
#[derive(Debug, Deserialize)]
struct CartesiaTranscriptionResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    words: Option<Vec<CartesiaTranscriptionWord>>,
}

/// Cartesia transcription (STT) model — implements `TranscriptionModel`.
///
/// Aligned with Vercel AI SDK `CartesiaTranscriptionModel`
/// (`reference/ai/packages/cartesia/src/cartesia-transcription-model.ts`).
///
/// Endpoint: `POST {base_url}/stt` (multipart form-data)
pub struct CartesiaTranscriptionModel {
    model_id: String,
    config: CartesiaConfig,
}

impl CartesiaTranscriptionModel {
    pub fn new(model_id: String, config: CartesiaConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
        headers.insert("Cartesia-Version".to_string(), self.config.version.clone());
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
        format!("{}/stt", self.config.base_url)
    }
}

fn audio_input_to_bytes_stt(audio: &AudioInput) -> Result<Vec<u8>, AiMuxError> {
    match audio {
        AudioInput::Binary(bytes) => Ok(bytes.clone()),
        AudioInput::Base64(b64) => {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                .map_err(|e| AiMuxError::InvalidArgument(format!("invalid base64: {e}")))
        }
    }
}

#[async_trait]
impl TranscriptionModel for CartesiaTranscriptionModel {
    fn provider(&self) -> &str {
        "cartesia"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(
        &self,
        options: &TranscriptionCallOptions,
    ) -> Result<TranscriptionResult, AiMuxError> {
        if is_streaming_transcription_model_id(&self.model_id) {
            return Err(AiMuxError::Unsupported(format!(
                "non-streaming transcription with {}",
                self.model_id
            )));
        }

        let mut warnings: Vec<Warning> = Vec::new();

        // Parse provider options.
        let mut language: Option<String> = None;
        let mut timestamp_granularities: Option<Vec<String>> = None;
        if let Some(ref po) = options.provider_options
            && let Some(cartesia) = po.get("cartesia")
        {
            if let Some(l) = cartesia.get("language").and_then(|v| v.as_str()) {
                language = Some(l.to_string());
            }
            if let Some(tg) = cartesia
                .get("timestampGranularities")
                .and_then(|v| v.as_array())
            {
                timestamp_granularities = Some(
                    tg.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect(),
                );
            }
            if cartesia.get("streaming").is_some() {
                warnings.push(Warning::Unsupported {
                    feature: "providerOptions.cartesia.streaming".to_string(),
                    details: Some(
                        "Cartesia batch transcription does not support streaming options."
                            .to_string(),
                    ),
                });
            }
        }

        let audio_bytes = audio_input_to_bytes_stt(&options.audio)?;
        let file_extension = media_type_to_extension(&options.media_type);
        let filename = format!("audio.{file_extension}");

        let mut form = MultipartForm::new();
        form.text("model", &self.model_id)?;
        form.file("file", &filename, &options.media_type, &audio_bytes)?;

        if let Some(ref lang) = language {
            form.text("language", lang)?;
        }
        if let Some(ref tg) = timestamp_granularities {
            for g in tg {
                form.text("timestamp_granularities[]", g)?;
            }
        }

        let (body_bytes, content_type) = form.finish();

        let headers = self.build_headers(options.headers.as_ref());

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: headers.into_iter().collect(),
                body: HttpBody::Bytes(body_bytes, content_type),

                abort_signal: options.abort_signal.clone(),
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;

        let raw_body: Value = serde_json::from_slice(&resp.body).unwrap_or(Value::Null);

        let parsed: CartesiaTranscriptionResponse = serde_json::from_value(raw_body.clone())
            .map_err(|e| AiMuxError::Json(e.to_string()))?;

        let segments: Vec<TranscriptionSegment> = parsed
            .words
            .as_ref()
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

        let timestamp = chrono::Utc::now().to_rfc3339();

        Ok(TranscriptionResult {
            text: parsed.text,
            segments,
            language: parsed.language,
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
