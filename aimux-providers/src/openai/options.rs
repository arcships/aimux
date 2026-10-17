//! Chat option schema mirrored from `openai-chat-language-model-options.ts`.

use aimux_core::error::AiMuxError;
use aimux_core::shared::JsonObject;
use serde_json::Value;

use super::convert::parse_option_fields;

pub(crate) const NAMESPACE: &str = "openai";
pub(crate) type ChatOptionsParser = fn(&JsonObject) -> Result<JsonObject, AiMuxError>;

pub(crate) fn parse_chat_options(options: &JsonObject) -> Result<JsonObject, AiMuxError> {
    parse_option_fields(options, NAMESPACE, |key, value| {
        let one_of = |values: &[&str]| value.as_str().is_some_and(|s| values.contains(&s));
        let valid = match key {
            "logitBias" => {
                return Some(value.as_object().ok_or(()).and_then(|object| {
                    let mut parsed = JsonObject::new();
                    for (key, value) in object {
                        if !value.is_number() {
                            return Err(());
                        }
                        let key = key.trim_matches(|c| {
                            matches!(c,
                                '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}'
                                | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
                                | '\u{205f}' | '\u{3000}' | '\u{feff}'
                            )
                        });
                        let number = if key.is_empty() {
                            0.0
                        } else if let Some((digits, radix)) = key
                            .strip_prefix("0x")
                            .or_else(|| key.strip_prefix("0X"))
                            .map(|s| (s, 16))
                            .or_else(|| {
                                key.strip_prefix("0o")
                                    .or_else(|| key.strip_prefix("0O"))
                                    .map(|s| (s, 8))
                            })
                            .or_else(|| {
                                key.strip_prefix("0b")
                                    .or_else(|| key.strip_prefix("0B"))
                                    .map(|s| (s, 2))
                            })
                        {
                            if digits.is_empty() {
                                return Err(());
                            }
                            let bits_per_digit = match radix {
                                16 => 4,
                                8 => 3,
                                _ => 1,
                            };
                            let mut significant_bits = 0usize;
                            let mut mantissa = 0u64;
                            let mut guard = false;
                            let mut sticky = false;
                            for digit in digits.chars() {
                                let digit = digit.to_digit(radix).ok_or(())?;
                                for shift in (0..bits_per_digit).rev() {
                                    let bit = (digit >> shift) & 1;
                                    if significant_bits == 0 && bit == 0 {
                                        continue;
                                    }
                                    if significant_bits < 53 {
                                        mantissa = (mantissa << 1) | u64::from(bit);
                                    } else if significant_bits == 53 {
                                        guard = bit != 0;
                                    } else {
                                        sticky |= bit != 0;
                                    }
                                    significant_bits += 1;
                                }
                            }
                            if guard && (sticky || mantissa & 1 != 0) {
                                mantissa += 1;
                            }
                            let exponent = significant_bits.saturating_sub(53);
                            if exponent > 1023 {
                                f64::INFINITY
                            } else {
                                mantissa as f64 * 2.0f64.powi(exponent as i32)
                            }
                        } else {
                            key.parse::<f64>().map_err(|_| ())?
                        };
                        if !number.is_finite() {
                            return Err(());
                        }
                        let key = if number == 0.0 {
                            "0".to_string()
                        } else if number.abs() >= 1e21 || number.abs() < 1e-6 {
                            let scientific = format!("{number:e}");
                            let (mantissa, exponent) = scientific.split_once('e').ok_or(())?;
                            let exponent = exponent.parse::<i32>().map_err(|_| ())?;
                            format!("{mantissa}e{exponent:+}")
                        } else {
                            number.to_string()
                        };
                        parsed.insert(key, value.clone());
                    }
                    Ok(Value::Object(parsed))
                }));
            }
            "logprobs" => value.is_boolean() || value.is_number(),
            "maxCompletionTokens" => value.is_number(),
            "metadata" => value.as_object().is_some_and(|object| {
                object.iter().all(|(key, value)| {
                    key.encode_utf16().count() <= 64
                        && value
                            .as_str()
                            .is_some_and(|s| s.encode_utf16().count() <= 512)
                })
            }),
            "prediction" => value.is_object(),
            "reasoningEffort" => {
                one_of(&["none", "minimal", "low", "medium", "high", "xhigh", "max"])
            }
            "serviceTier" => one_of(&["auto", "flex", "priority", "fast", "ultrafast", "default"]),
            "textVerbosity" => one_of(&["low", "medium", "high"]),
            "promptCacheRetention" => one_of(&["in_memory", "24h"]),
            "systemMessageMode" => one_of(&["system", "developer", "remove"]),
            "user" | "promptCacheKey" | "safetyIdentifier" => value.is_string(),
            "parallelToolCalls" | "store" | "strictJsonSchema" | "forceReasoning" => {
                value.is_boolean()
            }
            "promptCacheOptions" => {
                return Some(value.as_object().ok_or(()).and_then(|object| {
                    parse_option_fields(object, NAMESPACE, |key, value| {
                        let valid = match key {
                            "mode" => value
                                .as_str()
                                .is_some_and(|s| matches!(s, "implicit" | "explicit")),
                            "ttl" => value.as_str() == Some("30m"),
                            _ => return None,
                        };
                        Some(if valid { Ok(value.clone()) } else { Err(()) })
                    })
                    .map(Value::Object)
                    .map_err(|_| ())
                }));
            }
            _ => return None,
        };
        Some(if valid { Ok(value.clone()) } else { Err(()) })
    })
}
