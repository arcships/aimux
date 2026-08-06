//! Conversion from Anthropic usage objects to the unified `LanguageModelV4Usage`
//! shape.
//!
//! Faithful Rust port of the TypeScript `convertAnthropicUsage` in
//! `packages/anthropic/src/convert-anthropic-usage.ts`.

use serde::Deserialize;
use serde_json::Value;

use aimux_core::types::{TokenUsage, Usage};

use crate::anthropic::types::AnthropicUsage;

/// A single iteration entry inside an `AnthropicUsage` object.
#[derive(Debug, Deserialize)]
pub struct AnthropicUsageIteration {
    #[serde(rename = "type")]
    pub itype: String,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicOutputTokensDetails {
    #[serde(default)]
    pub thinking_tokens: Option<u64>,
}

/// Typed view over the Anthropic usage JSON. Every field is optional or
/// defaulted because the API may omit or null-out any of them.
#[derive(Debug, Default, Deserialize)]
pub struct AnthropicUsageInput {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub output_tokens_details: Option<AnthropicOutputTokensDetails>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    pub iterations: Option<Vec<AnthropicUsageIteration>>,
}

/// Input-side token breakdown (mirrors `inputTokens` of `LanguageModelV4Usage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnthropicInputTokens {
    pub total: u64,
    pub no_cache: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

/// Output-side token breakdown (mirrors `outputTokens` of `LanguageModelV4Usage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnthropicOutputTokens {
    pub total: u64,
    pub text: Option<u64>,
    pub reasoning: Option<u64>,
}

/// Result of [`convert_anthropic_usage`]. Mirrors the TS `LanguageModelV4Usage`.
#[derive(Debug, Clone)]
pub struct AnthropicUsageResult {
    pub input_tokens: AnthropicInputTokens,
    pub output_tokens: AnthropicOutputTokens,
    /// `rawUsage ?? usage` — the raw usage object, as returned by the provider.
    pub raw: Value,
}

/// Convert an Anthropic usage object into the unified usage shape.
///
/// `raw_usage` corresponds to the TS `rawUsage` argument; when `None`, the
/// `usage` object itself is used as `raw`.
pub fn convert_anthropic_usage(usage: &Value, raw_usage: Option<&Value>) -> AnthropicUsageResult {
    let u: AnthropicUsageInput = serde_json::from_value(usage.clone()).unwrap_or_default();

    let cache_creation = u.cache_creation_input_tokens.unwrap_or(0);
    let cache_read = u.cache_read_input_tokens.unwrap_or(0);
    let reasoning = u
        .output_tokens_details
        .as_ref()
        .and_then(|d| d.thinking_tokens);

    // When iterations is present (compaction or advisor), sum across executor
    // iterations to get the true executor totals. Advisor (`advisor_message`)
    // iterations are filtered out. A turn served by a server-side fallback is
    // the exception: the top-level totals already reflect the fallback answer.
    let served_by_fallback = u
        .iterations
        .as_ref()
        .map(|iters| iters.iter().any(|i| i.itype == "fallback_message"))
        .unwrap_or(false);

    let (input_tokens, output_tokens) = match &u.iterations {
        Some(iters) if !iters.is_empty() && !served_by_fallback => {
            let exec: Vec<&AnthropicUsageIteration> = iters
                .iter()
                .filter(|i| i.itype == "compaction" || i.itype == "message")
                .collect();
            if !exec.is_empty() {
                (
                    exec.iter().map(|i| i.input_tokens).sum::<u64>(),
                    exec.iter().map(|i| i.output_tokens).sum::<u64>(),
                )
            } else {
                (u.input_tokens, u.output_tokens)
            }
        }
        _ => (u.input_tokens, u.output_tokens),
    };

    let total_input = input_tokens + cache_creation + cache_read;
    let text = reasoning.map(|r| output_tokens.saturating_sub(r));

    let raw = raw_usage.cloned().unwrap_or_else(|| usage.clone());

    AnthropicUsageResult {
        input_tokens: AnthropicInputTokens {
            total: total_input,
            no_cache: input_tokens,
            cache_read,
            cache_write: cache_creation,
        },
        output_tokens: AnthropicOutputTokens {
            total: output_tokens,
            text,
            reasoning,
        },
        raw,
    }
}

/// Convert a typed `AnthropicUsage` (response/`message_start`) into the
/// unified core `Usage`, filling cache fields and the raw payload.
///
/// Semantics (RFC-0015 P0-2): `input_tokens.total` = input + cache_read +
/// cache_creation (Anthropic's own `input_tokens` excludes cache). This is a
/// deliberate correction for consumers.
pub fn usage_from_anthropic(usage: &AnthropicUsage) -> Usage {
    match serde_json::to_value(usage) {
        Ok(v) => {
            let r = convert_anthropic_usage(&v, None);
            Usage {
                input_tokens: TokenUsage {
                    total: Some(r.input_tokens.total as u32),
                    no_cache: Some(r.input_tokens.no_cache as u32),
                    cache_read: Some(r.input_tokens.cache_read as u32),
                    cache_write: Some(r.input_tokens.cache_write as u32),
                    ..Default::default()
                },
                output_tokens: TokenUsage {
                    total: Some(r.output_tokens.total as u32),
                    text: r.output_tokens.text.map(|t| t as u32),
                    reasoning: r.output_tokens.reasoning.map(|t| t as u32),
                    ..Default::default()
                },
                raw: Some(r.raw),
            }
        }
        Err(_) => Usage::default(),
    }
}
