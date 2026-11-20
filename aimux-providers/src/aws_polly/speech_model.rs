//! Amazon Polly speech (TTS) model.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;
use aimux_core::shared::{SharedProviderOptions, Warning};
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResult,
};
use aimux_provider_utils::HttpBody;

use crate::shared::{EndpointConfig, namespaced_options, speech_response, unsupported};

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "aws_polly";

/// Default voice ID used when no voice is provided. `Joanna` is a widely
/// available neural voice.
const DEFAULT_VOICE_ID: &str = "Joanna";

/// Default output format when none is requested.
const DEFAULT_OUTPUT_FORMAT: &str = "mp3";

/// Output formats accepted by the Polly `SynthesizeSpeech` API.
const SUPPORTED_OUTPUT_FORMATS: &[&str] = &[
    "mp3",
    "ogg_opus",
    "ogg_vorbis",
    "pcm",
    "mulaw",
    "alaw",
    "json",
];

/// AWS error shape: `{"__type": "...", "message": "..."}` (no `error` wrapper).
/// Passed to `send` so the http layer extracts the right fields on non-2xx.
fn aws_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(aws_error_parts)
}

fn aws_error_parts(data: &Value) -> aimux_provider_utils::ProviderErrorParts {
    aimux_provider_utils::ProviderErrorParts {
        message: data
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Amazon Polly request failed")
            .to_string(),
        provider_code: data
            .get("__type")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

// ── Speech model ─────────────────────────────────────────────────────────────

/// An Amazon Polly speech (TTS) model.
pub struct AwsPollySpeechModel {
    model_id: String,
    config: EndpointConfig,
}

impl AwsPollySpeechModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl SpeechModel for AwsPollySpeechModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let (body, warnings) = build_request(options, &self.model_id);
        let body_str = serde_json::to_string(&Value::Object(body.clone())).map_err(|e| {
            AiMuxError::JsonParse {
                message: e.to_string(),
                cause: Some(ErrorCause::source(&e)),
            }
        })?;
        // The request is signed by the transport (`SigV4Fetch`), over the exact
        // bytes sent: user-supplied headers and `Content-Type` are part of the
        // signature.
        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_to_api(
            exchange
                .request(exchange.url("/v1/speech"), options)
                .generation(options.timeout.as_ref()),
            HttpBody::Bytes(body_str.into_bytes(), "application/json".to_string()),
            aimux_provider_utils::create_binary_response_handler(),
            aws_failed_response_handler(),
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

/// Build the Polly `SynthesizeSpeech` request body and collect warnings.
///
/// Field mapping:
/// - `Engine` — derived from the model id (`aws_polly/{engine}`).
/// - `Text` — the input text.
/// - `VoiceId` — from `options.voice` (defaults to `Joanna`).
/// - `OutputFormat` — from `options.output_format` (defaults to `mp3`);
///   unsupported formats emit a warning and fall back to `mp3`.
/// - `LanguageCode` — from `options.language` when present.
/// - `speed` / `instructions` are not supported by Polly and emit warnings.
/// - Provider options (`aws_polly` key) override: `engine`, `sampleRate`,
///   `textType`, `lexiconNames`, `speechMarkTypes`.
fn build_request(
    options: &SpeechCallOptions,
    model_id: &str,
) -> (Map<String, Value>, Vec<Warning>) {
    let mut warnings = Vec::new();

    let (engine, engine_warning) = resolve_engine(model_id);
    warnings.extend(engine_warning);

    let voice = options.voice.as_deref().unwrap_or(DEFAULT_VOICE_ID);
    let output_format = options
        .output_format
        .as_deref()
        .unwrap_or(DEFAULT_OUTPUT_FORMAT);

    let mut body = Map::new();
    body.insert("Engine".to_string(), json!(engine));
    body.insert("Text".to_string(), json!(options.text));
    body.insert("VoiceId".to_string(), json!(voice));

    if SUPPORTED_OUTPUT_FORMATS.contains(&output_format) {
        body.insert("OutputFormat".to_string(), json!(output_format));
    } else {
        warnings.push(unsupported(
            "outputFormat",
            format!("Unsupported output format: {output_format}. Using mp3 instead."),
        ));
        body.insert("OutputFormat".to_string(), json!(DEFAULT_OUTPUT_FORMAT));
    }

    if let Some(ref language) = options.language {
        body.insert("LanguageCode".to_string(), json!(language));
    }

    // `speed` and `instructions` have no Polly equivalent.
    if options.speed.is_some() {
        warnings.push(unsupported(
            "speed",
            "Amazon Polly does not support the speed option.",
        ));
    }
    if options.instructions.is_some() {
        warnings.push(unsupported(
            "instructions",
            "Amazon Polly does not support the instructions option.",
        ));
    }

    // Provider-specific options (`aws_polly` key).
    if let Some(opts) = parse_polly_provider_options(options.provider_options.as_ref()) {
        if let Some(ref engine_override) = opts.engine {
            body.insert("Engine".to_string(), json!(engine_override));
        }
        if let Some(ref sample_rate) = opts.sample_rate {
            body.insert("SampleRate".to_string(), json!(sample_rate));
        }
        if let Some(ref text_type) = opts.text_type {
            body.insert("TextType".to_string(), json!(text_type));
        }
        if let Some(ref lexicon_names) = opts.lexicon_names {
            body.insert("LexiconNames".to_string(), json!(lexicon_names));
        }
        if let Some(ref speech_mark_types) = opts.speech_mark_types {
            body.insert("SpeechMarkTypes".to_string(), json!(speech_mark_types));
        }
    }

    (body, warnings)
}

/// Map a model id to a Polly `Engine` value.
///
/// Accepts both `"aws_polly/{engine}"` (the canonical id form) and a bare
/// engine name. Unknown engines default to `"standard"` with a warning.
fn resolve_engine(model_id: &str) -> (&'static str, Option<Warning>) {
    let normalized = model_id.strip_prefix("aws_polly/").unwrap_or(model_id);
    match normalized {
        "generative" => ("generative", None),
        "neural" => ("neural", None),
        "long-form" => ("long-form", None),
        "standard" => ("standard", None),
        "" => ("standard", None),
        other => (
            "standard",
            Some(unsupported(
                "engine",
                format!("Unknown Polly engine: {other}. Using standard instead."),
            )),
        ),
    }
}

// ── Provider options parsing ─────────────────────────────────────────────────

/// Parsed `aws_polly` speech provider options.
#[derive(Debug, Default)]
struct PollySpeechProviderOptions {
    engine: Option<String>,
    sample_rate: Option<String>,
    text_type: Option<String>,
    lexicon_names: Option<Vec<String>>,
    speech_mark_types: Option<Vec<String>>,
}

/// Extract Polly-specific speech options from the shared provider options.
///
/// Sample rate is accepted as a string or number and normalized to a string
/// (Polly expects e.g. `"22050"`).
fn parse_polly_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Option<PollySpeechProviderOptions> {
    let opts = namespaced_options(options, NAMESPACE)?;

    Some(PollySpeechProviderOptions {
        engine: opts
            .get("engine")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        sample_rate: opts.get("sampleRate").and_then(|v| {
            v.as_str()
                .map(std::string::ToString::to_string)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        }),
        text_type: opts
            .get("textType")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string),
        lexicon_names: opts
            .get("lexiconNames")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(std::string::ToString::to_string))
                    .collect()
            }),
        speech_mark_types: opts
            .get("speechMarkTypes")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(std::string::ToString::to_string))
                    .collect()
            }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_canonical_engine_ids() {
        assert_eq!(resolve_engine("aws_polly/generative").0, "generative");
        assert_eq!(resolve_engine("aws_polly/neural").0, "neural");
        assert_eq!(resolve_engine("aws_polly/long-form").0, "long-form");
        assert_eq!(resolve_engine("aws_polly/standard").0, "standard");
    }

    #[test]
    fn resolves_bare_engine_ids() {
        assert_eq!(resolve_engine("neural").0, "neural");
        assert_eq!(resolve_engine("standard").0, "standard");
    }

    #[test]
    fn unknown_engine_defaults_to_standard_with_warning() {
        let (engine, warning) = resolve_engine("aws_polly/unknown-engine");
        assert_eq!(engine, "standard");
        assert!(warning.is_some());
    }

    #[test]
    fn extracts_aws_error_fields() {
        let data = serde_json::json!({
            "__type": "UnrecognizedClientException",
            "message": "The security token included in the request is invalid."
        });
        let parts = aws_error_parts(&data);
        assert_eq!(
            parts.provider_code.as_deref(),
            Some("UnrecognizedClientException")
        );
        assert_eq!(
            parts.message,
            "The security token included in the request is invalid."
        );
    }

    #[test]
    fn missing_aws_message_uses_provider_fallback() {
        let parts = aws_error_parts(&serde_json::json!({ "__type": "AccessDeniedException" }));
        assert_eq!(parts.message, "Amazon Polly request failed");
    }
}
