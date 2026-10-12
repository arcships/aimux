//! The providerOptions and providerMetadata namespace of Mistral.
//!
//! `@ai-sdk/mistral` reads `providerOptions.mistral` and writes response
//! metadata under `mistral`, whatever the provider is named. This module is
//! the one place that spells the key.

use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions / providerMetadata key.
pub(crate) const NAMESPACE: &str = "mistral";

/// The Mistral options in a providerOptions map.
pub(crate) fn mistral_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}

/// Validate only the options declared by the upstream Mistral schema.
pub(crate) fn validated_mistral_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<Option<&JsonObject>, aimux_core::AiMuxError> {
    let Some(options) = mistral_options(provider_options) else {
        return Ok(None);
    };
    for (key, value) in options {
        let valid = match key.as_str() {
            "safePrompt" | "structuredOutputs" | "strictJsonSchema" | "parallelToolCalls" => {
                value.is_boolean()
            }
            "documentImageLimit" | "documentPageLimit" => value.is_number(),
            "promptCacheKey" => value.is_string(),
            "reasoningEffort" => matches!(value.as_str(), Some("high" | "none")),
            _ => true,
        };
        if !valid {
            return Err(aimux_core::AiMuxError::InvalidArgument(format!(
                "Invalid mistral provider option {key}"
            )));
        }
    }
    Ok(Some(options))
}

/// The `parseProviderOptions` failure: an `InvalidArgumentError` naming the
/// provider, with the offending key as the cause.
fn invalid(key: &str) -> AiMuxError {
    AiMuxError::InvalidArgument(format!("invalid mistral provider options: {key}"))
}

/// One optional key, converted by `get`; a present value of the wrong shape
/// (`null` included) is invalid, as in the zod schema.
fn field<'a, T>(
    options: &'a JsonObject,
    key: &str,
    get: impl FnOnce(&'a Value) -> Option<T>,
) -> Result<Option<T>, AiMuxError> {
    options
        .get(key)
        .map(|value| get(value).ok_or_else(|| invalid(key)))
        .transpose()
}

/// `z.string().min(1)`.
fn non_empty_str(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.is_empty())
}

/// `refAudio` of `mistralSpeechModelOptions`.
pub(crate) fn speech_ref_audio(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<Option<&str>, AiMuxError> {
    match mistral_options(provider_options) {
        Some(options) => Ok(field(options, "refAudio", non_empty_str)?),
        None => Ok(None),
    }
}

/// `mistralTranscriptionModelOptions`, validated.
#[derive(Default)]
pub(crate) struct TranscriptionOptions<'a> {
    pub(crate) language: Option<&'a str>,
    pub(crate) temperature: Option<f64>,
    pub(crate) timestamp_granularities: Option<Vec<&'a str>>,
    pub(crate) diarize: Option<bool>,
    pub(crate) context_bias: Option<Vec<&'a str>>,
}

pub(crate) fn transcription_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<TranscriptionOptions<'_>, AiMuxError> {
    let Some(options) = mistral_options(provider_options) else {
        return Ok(TranscriptionOptions::default());
    };
    Ok(TranscriptionOptions {
        language: field(options, "language", non_empty_str)?,
        temperature: field(options, "temperature", Value::as_f64)?,
        // z.array(z.enum(['segment', 'word'])).min(1)
        timestamp_granularities: field(options, "timestampGranularities", |value| {
            let items: Vec<&str> = value
                .as_array()?
                .iter()
                .map(|item| {
                    item.as_str()
                        .filter(|text| matches!(*text, "segment" | "word"))
                })
                .collect::<Option<_>>()?;
            (!items.is_empty()).then_some(items)
        })?,
        diarize: field(options, "diarize", Value::as_bool)?,
        // z.array(z.string().min(1).regex(/^[^\s,]+$/)).max(100)
        context_bias: field(options, "contextBias", |value| {
            let items: Vec<&str> = value
                .as_array()?
                .iter()
                .map(|item| {
                    item.as_str().filter(|text| {
                        !text.is_empty() && !text.contains(|c: char| c == ',' || c.is_whitespace())
                    })
                })
                .collect::<Option<_>>()?;
            (items.len() <= 100).then_some(items)
        })?,
    })
}
