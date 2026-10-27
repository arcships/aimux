//! `convert-cohere-usage.ts`: the `CohereUsage` shape and `convertCohereUsage`.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

use aimux_core::types::Usage;

#[derive(Debug)]
pub struct UsageResponse {
    pub tokens: TokenPair,
    pub raw: Value,
}

impl<'de> Deserialize<'de> for UsageResponse {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(deserializer)?;
        #[derive(Deserialize)]
        struct Validated {
            tokens: TokenPair,
            #[serde(default)]
            billed_units: Option<TokenPair>,
            #[serde(default)]
            cached_tokens: Option<f64>,
        }
        let validated: Validated =
            serde_json::from_value(raw.clone()).map_err(serde::de::Error::custom)?;
        Ok(Self {
            tokens: validated.tokens,
            raw,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TokenPair {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Convert Cohere `tokens` usage into the core `Usage` type.
///
/// Mirrors the TS `convertCohereUsage`:
/// - `input.total = input_tokens`, `input.noCache = input_tokens`
/// - `output.total = output_tokens`
pub(super) fn convert_usage(usage: &UsageResponse) -> Usage {
    let tokens = &usage.tokens;
    Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: Some(tokens.input_tokens),
            no_cache: Some(tokens.input_tokens),
            cache_read: None,
            cache_write: None,
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: Some(tokens.output_tokens),
            text: Some(tokens.output_tokens),
            ..Default::default()
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: usage.raw.as_object().cloned(),
    }
}
