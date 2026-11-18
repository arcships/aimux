//! Provider options of the OpenAI transcription model (openai-transcription-model-options.ts).

use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::optional_field;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// `openAITranscriptionModelOptions`, parsed from `providerOptions.openai`.
/// Every key is `.optional()`: absent is `None`, `null` fails. The schema's
/// defaults (`temperature` 0, `timestampGranularities` `['segment']`) apply
/// where the request is built, when the namespace is present.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenAITranscriptionOptions {
    /// Additional information to include in the transcription response.
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) include: Option<Vec<String>>,
    /// The language of the input audio in ISO-639-1 format.
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) language: Option<String>,
    /// An optional text to guide the model's style.
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) prompt: Option<String>,
    /// The sampling temperature, between 0 and 1.
    #[serde(default, deserialize_with = "temperature")]
    pub(crate) temperature: Option<f64>,
    /// The timestamp granularities to populate.
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) timestamp_granularities: Option<Vec<TimestampGranularity>>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) response_format: Option<ResponseFormat>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) chunking_strategy: Option<ChunkingStrategy>,
    /// The options of the realtime (streaming) session.
    #[cfg_attr(
        not(feature = "realtime"),
        allow(
            dead_code,
            reason = "validated always; read only by the realtime session"
        )
    )]
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) streaming: Option<StreamingOptions>,
}

/// `z.enum(['word', 'segment'])`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TimestampGranularity {
    Word,
    Segment,
}

impl TimestampGranularity {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Word => "word",
            Self::Segment => "segment",
        }
    }
}

/// `z.enum(['json', 'verbose_json', 'diarized_json'])`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseFormat {
    Json,
    VerboseJson,
    DiarizedJson,
}

impl ResponseFormat {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::VerboseJson => "verbose_json",
            Self::DiarizedJson => "diarized_json",
        }
    }
}

/// `chunkingStrategy`: `'auto'`, or `{ type: 'server_vad', threshold?,
/// prefixPaddingMs?, silenceDurationMs? }`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum ChunkingStrategy {
    Auto(Auto),
    ServerVad(ServerVad),
}

/// `z.literal('auto')`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Auto {
    Auto,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServerVad {
    #[serde(rename = "type")]
    kind: ServerVadType,
    /// `z.number().min(0).max(1)`, sent as written.
    #[serde(default, deserialize_with = "threshold")]
    threshold: Option<serde_json::Number>,
    /// `z.number().int().min(0)`, sent as written.
    #[serde(default, deserialize_with = "non_negative_int")]
    prefix_padding_ms: Option<serde_json::Number>,
    /// `z.number().int().min(0)`, sent as written.
    #[serde(default, deserialize_with = "non_negative_int")]
    silence_duration_ms: Option<serde_json::Number>,
}

/// `z.literal('server_vad')`.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ServerVadType {
    ServerVad,
}

impl ChunkingStrategy {
    /// `chunking_strategy` as sent: `auto`, or the server-VAD object as
    /// snake_case JSON (`transcription/openai-transcription-model.ts:241-259`).
    pub(crate) fn wire(&self) -> String {
        match self {
            Self::Auto(Auto::Auto) => "auto".to_string(),
            Self::ServerVad(vad) => {
                let mut wire = serde_json::Map::new();
                wire.insert("type".into(), serde_json::json!(vad.kind));
                for (key, value) in [
                    ("threshold", &vad.threshold),
                    ("prefix_padding_ms", &vad.prefix_padding_ms),
                    ("silence_duration_ms", &vad.silence_duration_ms),
                ] {
                    if let Some(value) = value {
                        wire.insert(key.into(), Value::Number(value.clone()));
                    }
                }
                Value::Object(wire).to_string()
            }
        }
    }
}

/// `streaming`: `{ delay?, include? }`.
#[cfg_attr(
    not(feature = "realtime"),
    allow(
        dead_code,
        reason = "validated always; read only by the realtime session"
    )
)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StreamingOptions {
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) delay: Option<StreamingDelay>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) include: Option<Vec<String>>,
}

/// `z.enum(['minimal', 'low', 'medium', 'high', 'xhigh'])`, sent as written.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum StreamingDelay {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

/// A number in `range` (an integer when `integer`), for an `.optional()` key
/// (`null` fails).
fn number_in<'de, D: Deserializer<'de>>(
    deserializer: D,
    range: std::ops::RangeInclusive<f64>,
    integer: bool,
    expected: &'static str,
) -> Result<Option<serde_json::Number>, D::Error> {
    let number = serde_json::Number::deserialize(deserializer)?;
    let value = number.as_f64().unwrap_or(f64::NAN);
    if !range.contains(&value) || (integer && value.fract() != 0.0) {
        return Err(serde::de::Error::custom(expected));
    }
    Ok(Some(number))
}

/// `temperature`: `z.number().min(0).max(1)`.
fn temperature<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
    let number = number_in(
        deserializer,
        0.0..=1.0,
        false,
        "expected a number from 0 to 1",
    )?;
    Ok(number.as_ref().and_then(serde_json::Number::as_f64))
}

/// `threshold`: `z.number().min(0).max(1)`.
fn threshold<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<serde_json::Number>, D::Error> {
    number_in(
        deserializer,
        0.0..=1.0,
        false,
        "expected a number from 0 to 1",
    )
}

/// `z.number().int().min(0)`.
fn non_negative_int<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<serde_json::Number>, D::Error> {
    number_in(
        deserializer,
        0.0..=f64::MAX,
        true,
        "expected a non-negative integer",
    )
}

/// `parseProviderOptions({ provider: 'openai', providerOptions, schema:
/// openAITranscriptionModelOptions })`
/// (`transcription/openai-transcription-model.ts:173-177`); all `None`
/// without an `openai` namespace.
///
/// # Errors
///
/// `InvalidArgument` "invalid openai provider options" when the namespace
/// does not match the schema, with the serde error as its cause.
pub(crate) fn parse_openai_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<OpenAITranscriptionOptions, aimux_core::AiMuxError> {
    Ok(
        aimux_provider_utils::parse_provider_options("openai", provider_options)?
            .unwrap_or_default(),
    )
}
