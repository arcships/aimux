//! Mistral speech model — implements the `SpeechModel` trait.
//!
//! Aligned with Vercel AI SDK `MistralSpeechModel`
//! (`reference/ai/packages/mistral/src/mistral-speech-model.ts`).
//!
//! Endpoint: `POST {base_url}/audio/speech`; the audio comes back base64 in a
//! JSON body.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};

use crate::shared::EndpointConfig;

const OUTPUT_FORMATS: &[&str] = &["pcm", "wav", "mp3", "flac", "opus"];

/// The request body; fields in the order of the upstream object literal, so
/// the `request.body` string matches `JSON.stringify`.
#[derive(Serialize)]
struct SpeechBody<'a> {
    model: &'a str,
    input: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ref_audio: Option<&'a str>,
    response_format: &'a str,
    stream: bool,
}

#[derive(Deserialize)]
struct SpeechResponseBody {
    audio_data: String,
}

/// A Mistral speech (TTS) model (e.g. `"voxtral-mini-tts-2603"`).
pub struct MistralSpeechModel {
    model_id: String,
    config: EndpointConfig,
}

impl MistralSpeechModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

fn unsupported(feature: &str, details: &str) -> Warning {
    Warning::Unsupported {
        feature: feature.to_string(),
        details: Some(details.to_string()),
    }
}

#[async_trait]
impl SpeechModel for MistralSpeechModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let ref_audio = super::options::speech_ref_audio(options.provider_options.as_ref())?;
        let mut warnings = Vec::new();

        let output_format = options.output_format.as_deref().unwrap_or("mp3");
        let response_format = if OUTPUT_FORMATS.contains(&output_format) {
            output_format
        } else {
            warnings.push(unsupported(
                "outputFormat",
                &format!("Unsupported output format: {output_format}. Using mp3 instead."),
            ));
            "mp3"
        };
        if options.instructions.is_some() {
            warnings.push(unsupported(
                "instructions",
                "Mistral speech models do not support the `instructions` option. \
                 Use a reference audio clip to guide delivery.",
            ));
        }
        if options.speed.is_some() {
            warnings.push(unsupported(
                "speed",
                "Mistral speech models do not support the `speed` option. It was ignored.",
            ));
        }
        if options.language.is_some() {
            warnings.push(unsupported(
                "language",
                "Mistral speech models do not support the `language` option. \
                 Language is inferred from the input text and voice.",
            ));
        }

        let body = SpeechBody {
            model: &self.model_id,
            input: &options.text,
            voice_id: if ref_audio.is_none() {
                options.voice.as_deref()
            } else {
                None
            },
            ref_audio,
            response_format,
            stream: false,
        };
        // The reference clip is never reported back: not in `request.body`,
        // not in an API error.
        let body_values = SpeechBody {
            ref_audio: ref_audio.map(|_| "[redacted]"),
            ..body
        };
        let request_values = serde_json::to_value(&body_values)?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/audio/speech"), options),
            serde_json::to_value(&body)?,
            aimux_provider_utils::create_json_response_handler::<SpeechResponseBody>(),
            super::mistral_failed_response_handler(),
        )
        .await
        .map_err(|error| match error {
            AiMuxError::ApiCall(mut call) => {
                call.request_body_values = request_values;
                AiMuxError::ApiCall(call)
            }
            other => other,
        })?;

        Ok(SpeechResult {
            audio: AudioData::Base64(resp.value.audio_data),
            warnings,
            request: Some(SpeechRequest {
                body: Some(Value::String(serde_json::to_string(&body_values)?)),
            }),
            response: SpeechResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(resp.response_headers),
                body: resp.raw_value,
            },
            provider_metadata: None,
        })
    }
}
