//! Cartesia speech (TTS) model.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResult,
};

use crate::shared::{EndpointConfig, namespaced_options, speech_response, unsupported};

use super::provider::{NAMESPACE, cartesia_failed_response_handler};

/// The valid sample rates for Cartesia output.
const SAMPLE_RATES: &[u32] = &[8000, 16000, 22050, 24000, 44100, 48000];

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
            exchange
                .request(exchange.url("/tts/bytes"), options)
                .generation_for(options),
            Value::Object(body.clone()),
            aimux_provider_utils::create_binary_response_handler(),
            cartesia_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let audio_bytes = resp.value.to_vec();

        let timestamp = aimux_core::Timestamp::now();

        Ok(SpeechResult {
            audio: AudioData::Binary(audio_bytes),
            warnings,
            usage: None,
            request: Some(SpeechRequest {
                body: Some(Value::Object(body)),
            }),
            response: speech_response(timestamp, &self.model_id, response_headers),
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
        warnings.push(unsupported(
            "outputFormat",
            format!(
                "Unknown output format \"{output_format}\". Falling back to mp3. Use providerOptions.cartesia to configure container, encoding, and sampleRate directly."
            ),
        ));
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
                    warnings.push(unsupported(
                        "outputFormat",
                        format!(
                            "Unsupported Cartesia sample rate in output format \"{}\". Using {} Hz instead.",
                            output_format,
                            get_sample_rate(&resolved)
                        ),
                    ));
                }
            } else {
                warnings.push(unsupported(
                    "outputFormat",
                    format!(
                        "Unsupported Cartesia sample rate in output format \"{}\". Using {} Hz instead.",
                        output_format,
                        get_sample_rate(&resolved)
                    ),
                ));
            }
        } else {
            warnings.push(unsupported(
                "outputFormat",
                format!(
                    "Unsupported Cartesia sample rate in output format \"{}\". Using {} Hz instead.",
                    output_format,
                    get_sample_rate(&resolved)
                ),
            ));
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
            warnings.push(unsupported(
                "providerOptions.cartesia.encoding",
                "Cartesia MP3 output does not accept an encoding. The encoding option was ignored.",
            ));
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
        warnings.push(unsupported(
            "providerOptions.cartesia.bitRate",
            "Cartesia raw and WAV output do not accept a bit rate. The bitRate option was ignored.",
        ));
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
        AiMuxError::invalid_argument(
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
            warnings.push(unsupported(
                "speed",
                "Cartesia speed must be between 0.6 and 1.5. The speed option was ignored.",
            ));
        }
    }

    if options.instructions.is_some() {
        warnings.push(unsupported("instructions", "Cartesia speech models do not support instructions. Instructions parameter was ignored.",));
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
    let opts = namespaced_options(options, NAMESPACE)?;

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
