//! OpenAI speech (TTS) model — implements the `SpeechModel` trait.
//!
//! Aligned with Vercel AI SDK `OpenAISpeechModel`
//! (`reference/ai/packages/openai/src/speech/openai-speech-model.ts`).
//!
//! Endpoint: `POST {base_url}/audio/speech`
//!
//! The OpenAI TTS API accepts `model`, `input` (text), `voice`, `response_format`,
//! `speed`, and `instructions` in the request body and returns raw binary audio
//! bytes in the response body. The `language` option is not supported and produces
//! an `unsupported` warning.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};

use super::config::OpenAIModelConfig;

/// The output formats accepted by the OpenAI TTS API.
const SUPPORTED_OUTPUT_FORMATS: &[&str] = &["mp3", "opus", "aac", "flac", "wav", "pcm"];

/// An OpenAI-compatible speech (TTS) model.
///
/// Works with any OpenAI-compatible `/audio/speech` endpoint.
pub struct OpenAISpeechModel {
    model_id: String,
    config: OpenAIModelConfig,
}

impl OpenAISpeechModel {
    pub(crate) fn from_config(model_id: String, config: OpenAIModelConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl SpeechModel for OpenAISpeechModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let (body, warnings) = build_request_body_and_warnings(options, &self.model_id)?;

        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let body = Value::Object(body);

        let resp = aimux_provider_utils::post_json_to_api(
            self.config
                .http_request(self.config.url("/audio/speech")?, headers, options),
            body.clone(),
            aimux_provider_utils::create_binary_response_handler(),
            super::openai_failed_response_handler(),
        )
        .await?;

        // send() returns Ok only for 2xx responses; non-2xx (incl. 408/409/429/5xx
        // after exhausting retries) is mapped to an AiMuxError internally.
        let response_headers = resp.response_headers;

        let audio_bytes = resp.value.to_vec();

        let timestamp = chrono::Utc::now().to_rfc3339();

        Ok(SpeechResult {
            audio: AudioData::Binary(audio_bytes),
            warnings,
            request: Some(SpeechRequest { body: Some(body) }),
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

// ── Request body builder ────────────────────────────────────────────────────

/// Build the OpenAI TTS request body and collect any warnings.
///
/// Mirrors the TS `OpenAISpeechModel.getArgs`:
/// - `voice` defaults to `"alloy"`.
/// - `response_format` defaults to `"mp3"`; unsupported formats emit a warning
///   and fall back to `"mp3"`.
/// - `language` is not supported and emits a warning.
/// - `speed` and `instructions` are forwarded when present.
fn build_request_body_and_warnings(
    options: &SpeechCallOptions,
    model_id: &str,
) -> Result<(Map<String, Value>, Vec<Warning>), AiMuxError> {
    let mut warnings = Vec::new();

    let voice = options.voice.as_deref().unwrap_or("alloy");
    let output_format = options.output_format.as_deref().unwrap_or("mp3");

    let mut body = Map::new();
    body.insert("model".to_string(), json!(model_id));
    body.insert("input".to_string(), json!(options.text));
    body.insert("voice".to_string(), json!(voice));
    body.insert("response_format".to_string(), json!("mp3"));

    if let Some(speed) = options.speed {
        body.insert("speed".to_string(), json!(speed));
    }
    if let Some(ref instructions) = options.instructions {
        body.insert("instructions".to_string(), json!(instructions));
    }

    if SUPPORTED_OUTPUT_FORMATS.contains(&output_format) {
        body.insert("response_format".to_string(), json!(output_format));
    } else if !output_format.is_empty() {
        warnings.push(Warning::Unsupported {
            feature: "outputFormat".to_string(),
            details: Some(format!(
                "Unsupported output format: {output_format}. Using mp3 instead."
            )),
        });
    }

    if let Some(provider_options) = options
        .provider_options
        .as_ref()
        .and_then(|options| options.get("openai"))
    {
        if let Some(value) = provider_options
            .get("speed")
            .filter(|value| !value.is_null())
        {
            let speed = value
                .as_f64()
                .filter(|speed| (0.25..=4.0).contains(speed))
                .ok_or_else(|| {
                    AiMuxError::InvalidArgument("invalid openai speech option: speed".into())
                })?;
            body.insert("speed".into(), json!(speed));
        }
        if let Some(value) = provider_options
            .get("instructions")
            .filter(|value| !value.is_null())
        {
            let instructions = value.as_str().ok_or_else(|| {
                AiMuxError::InvalidArgument("invalid openai speech option: instructions".into())
            })?;
            body.insert("instructions".into(), json!(instructions));
        }
    }

    if let Some(ref language) = options.language
        && !language.is_empty()
    {
        warnings.push(Warning::Unsupported {
            feature: "language".to_string(),
            details: Some(format!(
                "OpenAI speech models do not support language selection. Language parameter \"{language}\" was ignored."
            )),
        });
    }

    Ok((body, warnings))
}
