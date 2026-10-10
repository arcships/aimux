//! xAI speech (TTS) model.
//!
//! Aligned with `XaiSpeechModel`
//! (`reference/aisdk-pinned/xai/src/xai-speech-model.ts`).
//!
//! Endpoint: `POST {base_url}/tts`. The response is raw audio; with
//! `withTimestamps` it is a JSON envelope carrying base64 audio plus
//! character-level timings.

use async_trait::async_trait;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::speech_model::{
    AudioData, SpeechCallOptions, SpeechModel, SpeechRequest, SpeechResponse, SpeechResult,
};

use super::options::{self, xai_metadata};
use crate::shared::EndpointConfig;

/// An xAI speech model. It has no model id: `model_id()` is `""`.
pub struct XaiSpeechModel {
    config: EndpointConfig,
}

impl XaiSpeechModel {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

/// The `with_timestamps` JSON envelope: only the fields the implementation
/// reads, all nullish.
#[derive(Deserialize)]
struct TimestampsResponse {
    audio: Option<String>,
    content_type: Option<String>,
    duration: Option<f64>,
    audio_timestamps: Option<AudioTimestamps>,
}

#[derive(Deserialize)]
struct AudioTimestamps {
    graph_chars: Vec<String>,
    graph_times: Vec<(f64, f64)>,
}

fn unsupported(feature: &str, details: &str) -> Warning {
    Warning::Unsupported {
        feature: feature.to_owned(),
        details: Some(details.to_owned()),
    }
}

/// `getArgs`: the request body, the warnings and whether timestamps were asked.
fn build_request(options: &SpeechCallOptions) -> Result<(Value, Vec<Warning>, bool), AiMuxError> {
    let mut warnings = Vec::new();
    let empty = Map::new();
    let xai = options::xai_options(options.provider_options.as_ref()).unwrap_or(&empty);
    let sample_rate = options::sample_rate(xai)?;
    let bit_rate = options::opt_int(
        xai,
        "bitRate",
        0,
        i64::MAX,
        &[32000, 64000, 96000, 128000, 192000],
    )?;
    let latency = options::opt_int(xai, "optimizeStreamingLatency", 0, 2, &[])?;
    let text_normalization = options::opt_bool(xai, "textNormalization")?;
    let with_timestamps = options::opt_bool(xai, "withTimestamps")?;
    let replace = match xai.get("replace").filter(|v| !v.is_null()) {
        None => None,
        Some(Value::Object(map)) if map.values().all(Value::is_string) => {
            Some(Value::Object(map.clone()))
        }
        Some(_) => {
            return Err(AiMuxError::InvalidArgument(
                "Invalid argument for parameter providerOptions: xai.replace must be a record of strings"
                    .to_owned(),
            ));
        }
    };

    let output_format = options.output_format.as_deref().unwrap_or("mp3");
    let codec = if ["mp3", "wav", "pcm", "mulaw", "alaw"].contains(&output_format) {
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
            "xAI speech models do not support the `instructions` option. \
             Use xAI speech tags in `text` to control delivery.",
        ));
    }

    let mut format = Map::new();
    format.insert("codec".to_owned(), json!(codec));
    if let Some(rate) = sample_rate {
        format.insert("sample_rate".to_owned(), json!(rate));
    }
    if let Some(rate) = bit_rate {
        if codec == "mp3" {
            format.insert("bit_rate".to_owned(), json!(rate));
        } else {
            warnings.push(unsupported(
                "providerOptions",
                "xAI `bitRate` is supported only for mp3 output. It was ignored.",
            ));
        }
    }

    let mut body = Map::new();
    body.insert("text".to_owned(), json!(options.text));
    body.insert(
        "voice_id".to_owned(),
        json!(options.voice.as_deref().unwrap_or("eve")),
    );
    body.insert(
        "language".to_owned(),
        json!(options.language.as_deref().unwrap_or("auto")),
    );
    body.insert("output_format".to_owned(), Value::Object(format));
    for (key, value) in [
        ("speed", options.speed.map(|v| json!(v))),
        ("optimize_streaming_latency", latency.map(|v| json!(v))),
        ("text_normalization", text_normalization.map(|v| json!(v))),
        ("with_timestamps", with_timestamps.map(|v| json!(v))),
        ("replace", replace),
    ] {
        if let Some(value) = value {
            body.insert(key.to_owned(), value);
        }
    }
    Ok((Value::Object(body), warnings, with_timestamps == Some(true)))
}

#[async_trait]
impl SpeechModel for XaiSpeechModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        ""
    }

    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> {
        let (body, warnings, with_timestamps) = build_request(options)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let url = exchange.url("/tts");

        // With `with_timestamps` the API returns a JSON envelope instead of
        // raw audio bytes.
        let (audio, headers, raw, envelope) = if with_timestamps {
            let resp = aimux_provider_utils::post_json_to_api(
                exchange.request(url, options),
                body.clone(),
                aimux_provider_utils::create_json_response_handler::<TimestampsResponse>(),
                super::xai_failed_response_handler(),
            )
            .await?;
            let envelope = resp.value;
            // Empty audio is returned as-is so the core layer reports it.
            let audio = match &envelope.audio {
                Some(audio) => base64::engine::general_purpose::STANDARD
                    .decode(audio)
                    .map_err(|e| {
                        AiMuxError::InvalidResponseData(format!("invalid base64 audio: {e}"))
                    })?,
                None => Vec::new(),
            };
            (audio, resp.response_headers, resp.raw_value, Some(envelope))
        } else {
            let resp = aimux_provider_utils::post_json_to_api(
                exchange.request(url, options),
                body.clone(),
                aimux_provider_utils::create_binary_response_handler(),
                super::xai_failed_response_handler(),
            )
            .await?;
            (resp.value.to_vec(), resp.response_headers, None, None)
        };

        // xAI returns a trace id on every response.
        let mut metadata = Map::new();
        if let Some(trace_id) = headers.get("x-trace-id") {
            metadata.insert("traceId".to_owned(), json!(trace_id));
        }
        if let Some(envelope) = &envelope {
            if let Some(duration) = envelope.duration {
                metadata.insert("duration".to_owned(), json!(duration));
            }
            if let Some(content_type) = &envelope.content_type {
                metadata.insert("contentType".to_owned(), json!(content_type));
            }
            if let Some(timestamps) = &envelope.audio_timestamps {
                metadata.insert(
                    "audioTimestamps".to_owned(),
                    json!({ "graphChars": timestamps.graph_chars, "graphTimes": timestamps.graph_times }),
                );
            }
        }

        Ok(SpeechResult {
            audio: AudioData::Binary(audio),
            warnings,
            request: Some(SpeechRequest { body: Some(body) }),
            response: SpeechResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(String::new()),
                headers: Some(headers),
                body: raw,
            },
            provider_metadata: Some(xai_metadata(Value::Object(metadata))),
        })
    }
}
