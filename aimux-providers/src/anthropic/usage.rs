//! Conversion from Anthropic usage objects to the unified `LanguageModelV4Usage`
//! shape.
//!
//! Faithful Rust port of the TypeScript `convertAnthropicUsage` in
//! `packages/anthropic/src/convert-anthropic-usage.ts`.

use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::types::{TokenUsage, Usage};

use crate::anthropic::options::CANONICAL as CANONICAL_KEY;
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
#[must_use]
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

/// The result-level `providerMetadata` of a response, as `@ai-sdk/anthropic`
/// builds it: under the canonical `anthropic` key and, for a provider created
/// with another `name`, under that key too.
///
/// `usage` is the raw usage object (for a stream: `message_start`'s, updated
/// by `message_delta`'s). `iterations`, `container` and `contextManagement`
/// are re-keyed in camelCase; every absent piece is `null`.
pub(crate) fn result_provider_metadata(
    options_name: &str,
    usage: &Value,
    stop_sequence: Option<&str>,
    container: Option<&Value>,
    context_management: Option<&Value>,
) -> Value {
    let field = |value: &Value, key: &str| value.get(key).cloned().unwrap_or(Value::Null);
    let iterations =
        usage
            .get("iterations")
            .and_then(Value::as_array)
            .map_or(Value::Null, |iterations| {
                Value::Array(
                    iterations
                        .iter()
                        .map(|i| {
                            json!({
                                "type": field(i, "type"),
                                "inputTokens": field(i, "input_tokens"),
                                "outputTokens": field(i, "output_tokens"),
                            })
                        })
                        .collect(),
                )
            });
    let container = container.map_or(Value::Null, |c| {
        let skills = c
            .get("skills")
            .and_then(Value::as_array)
            .map_or(Value::Null, |skills| {
                Value::Array(
                    skills
                        .iter()
                        .map(|s| {
                            json!({
                                "type": field(s, "type"),
                                "skillId": field(s, "skill_id"),
                                "version": field(s, "version"),
                            })
                        })
                        .collect(),
                )
            });
        json!({ "expiresAt": field(c, "expires_at"), "id": field(c, "id"), "skills": skills })
    });
    let context_management = context_management.map_or(Value::Null, |cm| {
        let edits = cm
            .get("applied_edits")
            .and_then(Value::as_array)
            .map(|edits| {
                edits
                    .iter()
                    .map(|edit| match edit.as_object() {
                        Some(edit) => Value::Object(
                            edit.iter()
                                .map(|(k, v)| (snake_to_camel(k), v.clone()))
                                .collect(),
                        ),
                        None => edit.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        json!({ "appliedEdits": edits })
    });
    let metadata = json!({
        "usage": usage,
        "stopSequence": stop_sequence,
        "iterations": iterations,
        "container": container,
        "contextManagement": context_management,
    });
    let mut keys = vec![CANONICAL_KEY];
    if options_name != CANONICAL_KEY {
        keys.push(options_name);
    }
    Value::Object(
        keys.into_iter()
            .map(|key| (key.to_string(), metadata.clone()))
            .collect(),
    )
}

/// `cleared_input_tokens` -> `clearedInputTokens`.
fn snake_to_camel(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut upper = false;
    for c in key.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Convert a typed `AnthropicUsage` (response/`message_start`) into the
/// unified core `Usage`, filling cache fields and the raw payload.
///
/// Semantics (RFC-0015 P0-2): `input_tokens.total` = input + cache_read +
/// cache_creation (Anthropic's own `input_tokens` excludes cache). This is a
/// deliberate correction for consumers.
#[must_use]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_metadata_rekeys_iterations_container_and_edits() {
        let usage = json!({
            "input_tokens": 5,
            "output_tokens": 7,
            "iterations": [
                { "type": "message", "input_tokens": 5, "output_tokens": 7 },
            ],
        });
        let container = json!({
            "id": "container_1",
            "expires_at": "2026-01-01T00:00:00Z",
            "skills": [{ "type": "custom", "skill_id": "pdf", "version": "latest" }],
        });
        let edits = json!({ "applied_edits": [
            { "type": "clear_tool_uses_20250919", "cleared_tool_uses": 2, "cleared_input_tokens": 90 },
            { "type": "compact_20260112" },
        ]});
        let metadata =
            result_provider_metadata("proxy", &usage, Some("END"), Some(&container), Some(&edits));
        let expected = json!({
            "usage": usage,
            "stopSequence": "END",
            "iterations": [{ "type": "message", "inputTokens": 5, "outputTokens": 7 }],
            "container": {
                "expiresAt": "2026-01-01T00:00:00Z",
                "id": "container_1",
                "skills": [{ "type": "custom", "skillId": "pdf", "version": "latest" }],
            },
            "contextManagement": { "appliedEdits": [
                { "type": "clear_tool_uses_20250919", "clearedToolUses": 2, "clearedInputTokens": 90 },
                { "type": "compact_20260112" },
            ]},
        });
        // The same object under the canonical key and the custom name.
        assert_eq!(
            metadata,
            json!({ CANONICAL_KEY: expected, "proxy": expected })
        );
    }

    /// The typed usage keeps what the API sent: serializing it gives the raw
    /// object back (nothing invented, nothing dropped), so `iterations` reach
    /// the token accounting and `usage.raw` is the provider's own object.
    #[test]
    fn typed_usage_round_trips_the_raw_object() {
        let raw = json!({
            "input_tokens": 100,
            "output_tokens": 50,
            "iterations": [
                { "type": "compaction", "input_tokens": 60, "output_tokens": 10 },
                { "type": "message", "input_tokens": 40, "output_tokens": 40 },
            ],
            "service_tier": "standard",
        });
        let typed: AnthropicUsage = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(serde_json::to_value(&typed).unwrap(), raw);
        let usage = usage_from_anthropic(&typed);
        assert_eq!(usage.raw, Some(raw));
        // Summed across the executor iterations, not the top-level totals.
        assert_eq!(usage.input_tokens.total, Some(100));
        assert_eq!(usage.output_tokens.total, Some(50));

        let plain: AnthropicUsage =
            serde_json::from_value(json!({ "input_tokens": 3, "output_tokens": 4 })).unwrap();
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            json!({ "input_tokens": 3, "output_tokens": 4 })
        );
    }

    #[test]
    fn absent_pieces_are_null_under_the_canonical_key_only() {
        let usage = json!({ "input_tokens": 1, "output_tokens": 2 });
        let metadata = result_provider_metadata(CANONICAL_KEY, &usage, None, None, None);
        assert_eq!(
            metadata,
            json!({ CANONICAL_KEY: {
                "usage": usage,
                "stopSequence": null,
                "iterations": null,
                "container": null,
                "contextManagement": null,
            }})
        );
    }
}
