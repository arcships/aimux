//! LMNT speech (TTS) model.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResult,
};

use crate::shared::{EndpointConfig, namespaced_options, speech_response, unsupported};

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "lmnt";

/// The default voice ID used when no voice is provided.
const DEFAULT_VOICE_ID: &str = "ava";

/// The output formats accepted by the LMNT TTS API.
const SUPPORTED_OUTPUT_FORMATS: &[&str] = &["mp3", "aac", "mulaw", "raw", "wav"];

fn lmnt_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    crate::shared::nested_error_response_handler("LMNT request failed")
}

// ── Speech model ─────────────────────────────────────────────────────────────

/// An LMNT speech (TTS) model.
pub struct LMNTSpeechModel {
    model_id: String,
    config: EndpointConfig,
}

impl LMNTSpeechModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl SpeechModel for LMNTSpeechModel {
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
                .request(exchange.url("/v1/ai/speech/bytes"), options)
                .generation(options.timeout.as_ref()),
            Value::Object(body.clone()),
            aimux_provider_utils::create_binary_response_handler(),
            lmnt_failed_response_handler(),
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
        warnings.push(unsupported(
            "outputFormat",
            format!("Unsupported output format: {output_format}. Using mp3 instead."),
        ));
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
    let opts = namespaced_options(options, NAMESPACE)?;

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
