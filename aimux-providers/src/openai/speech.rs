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

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, send};

use super::OpenAIConfig;

/// The output formats accepted by the OpenAI TTS API.
const SUPPORTED_OUTPUT_FORMATS: &[&str] = &["mp3", "opus", "aac", "flac", "wav", "pcm"];

/// An OpenAI-compatible speech (TTS) model.
///
/// Works with any OpenAI-compatible `/audio/speech` endpoint.
pub struct OpenAISpeechModel {
    model_id: String,
    config: OpenAIConfig,
}

impl OpenAISpeechModel {
    #[must_use]
    pub fn new(model_id: String, config: OpenAIConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
        if let Some(ref org) = self.config.org_id {
            headers.insert("OpenAI-Organization".to_string(), org.clone());
        }
        if let Some(ref project) = self.config.project {
            headers.insert("OpenAI-Project".to_string(), project.clone());
        }
        // Config-level extra headers (lowest priority after auth/org/project).
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
        format!("{}/audio/speech", self.config.base_url)
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

        let headers = self.build_headers(options.headers.as_ref());

        let header_list: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: header_list,
                body: HttpBody::Json(Value::Object(body.clone())),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            self.config.retry_config,
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        // send() returns Ok only for 2xx responses; non-2xx (incl. 408/409/429/5xx
        // after exhausting retries) is mapped to an AiMuxError internally.
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
    } else {
        warnings.push(Warning::Unsupported {
            feature: "outputFormat".to_string(),
            details: Some(format!(
                "Unsupported output format: {output_format}. Using mp3 instead."
            )),
        });
    }

    if let Some(ref language) = options.language {
        warnings.push(Warning::Unsupported {
            feature: "language".to_string(),
            details: Some(format!(
                "OpenAI speech models do not support language selection. Language parameter \"{language}\" was ignored."
            )),
        });
    }

    Ok((body, warnings))
}
