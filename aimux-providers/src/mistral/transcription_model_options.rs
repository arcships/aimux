//! `mistral-transcription-model-options.ts`.

use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

use super::chat_language_model_options::{field, mistral_options, non_empty_str};

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
