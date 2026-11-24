//! Hume speech (TTS) model.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResult,
};

use crate::shared::{EndpointConfig, namespaced_options, speech_response, unsupported};

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "hume";

/// The default voice ID used when no voice is provided.
const DEFAULT_VOICE_ID: &str = "d8ab67c6-953d-4bd8-9370-8fa53a0f1453";

/// The output formats accepted by the Hume TTS API.
const SUPPORTED_OUTPUT_FORMATS: &[&str] = &["mp3", "pcm", "wav"];

/// Hume errors carry a top-level `message` with a `code` such as `"E0101"`
/// (https://dev.hume.ai/docs/resources/errors); the nested `{error:
/// {message, code}}` shape matches the AI SDK's Hume error schema.
fn hume_error_parts(data: &Value) -> aimux_provider_utils::ProviderErrorParts {
    let error = data.get("error");
    let message = data
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| {
            error
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        })
        .or_else(|| error.and_then(Value::as_str))
        .unwrap_or("Hume request failed")
        .to_string();
    aimux_provider_utils::ProviderErrorParts {
        message,
        provider_code: data
            .get("code")
            .or_else(|| error.and_then(|value| value.get("code")))
            .and_then(crate::shared::json_error_code),
    }
}

fn hume_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(hume_error_parts)
}

// ── Speech model ─────────────────────────────────────────────────────────────

/// A Hume speech (TTS) model.
pub struct HumeSpeechModel {
    /// Hume does not use model IDs for TTS; this is always an empty string.
    model_id: String,
    config: EndpointConfig,
}

impl HumeSpeechModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self {
            model_id: String::new(),
            config,
        }
    }
}

#[async_trait]
impl SpeechModel for HumeSpeechModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let (body, warnings) = build_request(options)?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange
                .request(exchange.url("/v0/tts/file"), options)
                .generation_for(options),
            Value::Object(body.clone()),
            aimux_provider_utils::create_binary_response_handler(),
            hume_failed_response_handler(),
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
        warnings.push(unsupported(
            "outputFormat",
            format!("Unsupported output format: {output_format}. Using mp3 instead."),
        ));
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
        warnings.push(unsupported(
            "language",
            format!(
                "Hume speech models do not support language selection. Language parameter \"{language}\" was ignored."
            ),
        ));
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
    let opts = namespaced_options(options, NAMESPACE)?;
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
                    if let Some(s) = u.get("speed").and_then(serde_json::Value::as_f64) {
                        m.insert("speed".to_string(), json!(s));
                    }
                    if let Some(ts) = u.get("trailingSilence").and_then(serde_json::Value::as_f64) {
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
