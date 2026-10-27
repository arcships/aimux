//! `convert-mistral-usage.ts`: `mistralUsageSchema` (the `UsageResponse`
//! deserializer) and `convertMistralUsage`.

#![allow(dead_code)]

use serde::Deserialize;
use serde_json::Value;

use aimux_core::types::Usage;

#[derive(Debug)]
pub struct UsageResponse {
    pub fields: UsageFields,
    pub raw: Value,
}

impl<'de> Deserialize<'de> for UsageResponse {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(deserializer)?;
        let fields = serde_json::from_value(raw.clone()).map_err(serde::de::Error::custom)?;
        Ok(Self { fields, raw })
    }
}

#[derive(Debug, Deserialize)]
pub struct UsageFields {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub prompt_audio_seconds: Option<f64>,
    pub request_count: Option<f64>,
    pub service_tier: Option<String>,
    pub num_cached_tokens: Option<u32>,
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    pub prompt_token_details: Option<PromptTokensDetails>,
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Deserialize)]
pub struct PromptTokensDetails {
    pub cached_tokens: Option<u32>,
    pub audio_tokens: Option<f64>,
    pub messages: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
pub struct CompletionTokensDetails {
    pub reasoning_tokens: Option<f64>,
}

/// Convert a Mistral `UsageResponse` into the core `Usage` type.
///
/// Mirrors the TS `convertMistralUsage`:
/// - `input.total = prompt_tokens`
/// - `input.noCache = prompt_tokens - cache_read_tokens`
/// - `input.cacheRead = cache_read_tokens` (or undefined when 0)
/// - `output.total = completion_tokens`
pub(super) fn convert_usage(usage: &UsageResponse) -> Usage {
    let raw = usage.raw.clone();
    let usage = &usage.fields;
    let prompt_tokens = usage.prompt_tokens;
    let completion_tokens = usage.completion_tokens;

    let cache_read = usage
        .num_cached_tokens
        .or_else(|| {
            usage
                .prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens)
        })
        .or_else(|| {
            usage
                .prompt_token_details
                .as_ref()
                .and_then(|d| d.cached_tokens)
        })
        .unwrap_or(0);

    let no_cache = prompt_tokens - cache_read;

    Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: Some(prompt_tokens),
            no_cache: Some(no_cache),
            cache_read: if cache_read > 0 {
                Some(cache_read)
            } else {
                None
            },
            cache_write: None,
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: Some(completion_tokens),
            text: Some(completion_tokens),
            ..Default::default()
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: raw.as_object().cloned(),
    }
}
