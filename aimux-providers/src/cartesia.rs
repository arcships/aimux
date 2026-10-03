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
//!
//! [`create_cartesia`] takes [`CartesiaProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`CartesiaProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `CARTESIA_API_KEY`.
//! [`cartesia()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};
use aimux_provider_utils::HttpBody;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{Credential, EndpointConfig, provider_headers};

fn cartesia_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let title = data
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Cartesia error");
        let message = data
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("request failed");
        aimux_provider_utils::ProviderErrorParts {
            message: format!("{title}: {message}"),
            provider_code: data
                .get("error_code")
                .and_then(Value::as_str)
                .map(str::to_string),
        }
    })
}

// ── Constants ────────────────────────────────────────────────────────────────

/// The Cartesia API version sent with every request via the `Cartesia-Version`
/// header.
const CARTESIA_API_VERSION: &str = "2026-03-01";

/// The valid sample rates for Cartesia output.
const SAMPLE_RATES: &[u32] = &[8000, 16000, 22050, 24000, 44100, 48000];

// ── Config ───────────────────────────────────────────────────────────────────

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://api.cartesia.ai";
const API_KEY_ENV_VAR: &str = "CARTESIA_API_KEY";
const DEFAULT_NAME: &str = "cartesia";

/// Settings of [`create_cartesia`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct CartesiaProviderSettings {
    /// Base URL for the API calls. Default `https://api.cartesia.ai`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `CARTESIA_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.speech"`, `"{name}.transcription"`).
    /// Default `"cartesia"`. The providerOptions key stays `cartesia`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for CartesiaProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CartesiaProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Create a Cartesia provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_cartesia(settings: CartesiaProviderSettings) -> Result<CartesiaProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(CartesiaProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Cartesia"),
            vec![(
                "Cartesia-Version".to_string(),
                CARTESIA_API_VERSION.to_string(),
            )],
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_cartesia` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn cartesia() -> &'static CartesiaProvider {
    static DEFAULT: OnceLock<CartesiaProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_cartesia(CartesiaProviderSettings::default())
            .expect("default Cartesia settings are always valid")
    })
}

/// A Cartesia provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct CartesiaProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl CartesiaProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// A speech (TTS) model (e.g. `"sonic-3.5"`); `provider()` is `"{name}.speech"`.
    #[must_use]
    pub fn speech(&self, model_id: &str) -> CartesiaSpeechModel {
        CartesiaSpeechModel::from_config(model_id.to_string(), self.model_config("speech"))
    }

    /// A transcription (STT) model (e.g. `"ink-whisper"`); `provider()` is `"{name}.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> CartesiaTranscriptionModel {
        CartesiaTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        )
    }
}

impl ::aimux_core::Provider for CartesiaProvider {
    fn language_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::LanguageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "languageModel"))
    }

    fn embedding_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }

    fn transcription_model(
        &self,
        model_id: &str,
    ) -> Option<Result<::std::sync::Arc<dyn ::aimux_core::TranscriptionModel>, AiMuxError>> {
        Some(Ok(::std::sync::Arc::new(self.transcription(model_id))))
    }

    fn speech_model(
        &self,
        model_id: &str,
    ) -> Option<Result<::std::sync::Arc<dyn ::aimux_core::SpeechModel>, AiMuxError>> {
        Some(Ok(::std::sync::Arc::new(self.speech(model_id))))
    }
}

// ── Speech model ─────────────────────────────────────────────────────────────

/// A Cartesia speech (TTS) model.
pub struct CartesiaSpeechModel {
    model_id: String,
    config: EndpointConfig,
}

impl CartesiaSpeechModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl SpeechModel for CartesiaSpeechModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let (body, warnings) = build_request(options, &self.model_id)?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/tts/bytes"), options),
            Value::Object(body.clone()),
            aimux_provider_utils::create_binary_response_handler(),
            cartesia_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

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
                "Unknown output format \"{output_format}\". Falling back to mp3. Use providerOptions.cartesia to configure container, encoding, and sampleRate directly."
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
    let provider_opts = options::cartesia_options(options)?;
    let opts = provider_opts.as_object()?;

    Some(CartesiaSpeechProviderOptions {
        container: opts
            .get("container")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        encoding: opts
            .get("encoding")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        sample_rate: opts
            .get("sampleRate")
            .and_then(serde_json::Value::as_u64)
            .map(|n| n as u32),
        bit_rate: opts
            .get("bitRate")
            .and_then(serde_json::Value::as_u64)
            .map(|n| n as u32),
        speed: opts.get("speed").and_then(serde_json::Value::as_f64),
        language: opts
            .get("language")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
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
///
/// `do_stream` for these IDs is intentionally NOT implemented — deferred
/// with explicit triggers (RFC-0034 §4/D6: docs behind a login wall, young
/// turns API, schema inferable from the SDK only). Until it lands, both
/// paths return `UnsupportedFunctionality`.
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
    config: EndpointConfig,
}

impl CartesiaTranscriptionModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
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
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(
        &self,
        options: &TranscriptionCallOptions,
    ) -> Result<TranscriptionResult, AiMuxError> {
        if is_streaming_transcription_model_id(&self.model_id) {
            return Err(AiMuxError::UnsupportedFunctionality(format!(
                "non-streaming transcription with {}",
                self.model_id
            )));
        }

        let mut warnings: Vec<Warning> = Vec::new();

        // Parse provider options.
        let mut language: Option<String> = None;
        let mut timestamp_granularities: Option<Vec<String>> = None;
        if let Some(cartesia) = options::cartesia_options(options.provider_options.as_ref()) {
            if let Some(l) = cartesia.get("language").and_then(|v| v.as_str()) {
                language = Some(l.to_string());
            }
            if let Some(tg) = cartesia
                .get("timestampGranularities")
                .and_then(|v| v.as_array())
            {
                timestamp_granularities = Some(
                    tg.iter()
                        .filter_map(|v| v.as_str().map(std::string::ToString::to_string))
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

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_to_api(
            exchange.request(exchange.url("/stt"), options),
            HttpBody::Bytes(body_bytes, content_type),
            aimux_provider_utils::create_json_response_handler::<CartesiaTranscriptionResponse>(),
            cartesia_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let raw_body = resp.raw_value.unwrap_or(Value::Null);
        let parsed = resp.value;

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
