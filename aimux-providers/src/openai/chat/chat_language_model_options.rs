//! Chat option schema mirrored from `openai-chat-language-model-options.ts`,
//! with the shared provider-options readers.

use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;
use aimux_core::shared::{JsonObject, SharedProviderOptions};
use aimux_provider_utils::{optional_field, parse_option_fields};
use serde::Deserialize;
use serde_json::Value;

pub(crate) const NAMESPACE: &str = "openai";
pub(crate) type ChatOptionsParser = fn(&JsonObject) -> Result<JsonObject, AiMuxError>;

/// `openaiLanguageModelChatOptions` (openai-chat-language-model-options.ts).
/// Every key is `.optional()`: `null` fails as a wrong type does. The
/// parsed namespace is built from the raw object below; this struct is the
/// schema it is checked against.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code, reason = "validated only; the namespace is read as JSON")]
struct OpenAIChatOptions {
    #[serde(default, deserialize_with = "logit_bias")]
    logit_bias: Option<serde_json::Map<String, Value>>,
    #[serde(default, deserialize_with = "optional_field")]
    logprobs: Option<Logprobs>,
    #[serde(default, deserialize_with = "optional_field")]
    parallel_tool_calls: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    user: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    reasoning_effort: Option<ReasoningEffort>,
    #[serde(default, deserialize_with = "optional_field")]
    max_completion_tokens: Option<f64>,
    #[serde(default, deserialize_with = "optional_field")]
    store: Option<bool>,
    #[serde(default, deserialize_with = "metadata")]
    metadata: Option<serde_json::Map<String, Value>>,
    #[serde(default, deserialize_with = "optional_field")]
    prediction: Option<serde_json::Map<String, Value>>,
    #[serde(default, deserialize_with = "optional_field")]
    service_tier: Option<ServiceTier>,
    #[serde(default, deserialize_with = "optional_field")]
    strict_json_schema: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    text_verbosity: Option<TextVerbosity>,
    #[serde(default, deserialize_with = "optional_field")]
    prompt_cache_key: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    prompt_cache_options: Option<PromptCacheOptions>,
    #[serde(default, deserialize_with = "optional_field")]
    prompt_cache_retention: Option<PromptCacheRetention>,
    #[serde(default, deserialize_with = "optional_field")]
    safety_identifier: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    system_message_mode: Option<SystemMessageMode>,
    #[serde(default, deserialize_with = "optional_field")]
    force_reasoning: Option<bool>,
}

/// `logprobs`: `z.union([z.boolean(), z.number()])`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
#[allow(dead_code, reason = "validated only; the namespace is read as JSON")]
enum Logprobs {
    Flag(bool),
    Count(f64),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ReasoningEffort {
    #[serde(rename = "none")]
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ServiceTier {
    Auto,
    Flex,
    Priority,
    Fast,
    Ultrafast,
    Default,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TextVerbosity {
    Low,
    Medium,
    High,
}

/// `promptCacheOptions`: `{ mode?: 'implicit' | 'explicit', ttl?: '30m' }`.
#[derive(Debug, Deserialize)]
#[allow(dead_code, reason = "validated only; the namespace is read as JSON")]
struct PromptCacheOptions {
    #[serde(default, deserialize_with = "optional_field")]
    mode: Option<PromptCacheMode>,
    #[serde(default, deserialize_with = "optional_field")]
    ttl: Option<PromptCacheTtl>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PromptCacheMode {
    Implicit,
    Explicit,
}

#[derive(Debug, Deserialize)]
enum PromptCacheTtl {
    #[serde(rename = "30m")]
    ThirtyMinutes,
}

#[derive(Debug, Deserialize)]
enum PromptCacheRetention {
    #[serde(rename = "in_memory")]
    InMemory,
    #[serde(rename = "24h")]
    OneDay,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SystemMessageMode {
    System,
    Developer,
    Remove,
}

/// `logitBias`: `z.record(z.coerce.number<string>(), z.number())`. Every
/// value is a number and every key coerces to one
/// ([`coerce_logit_bias_key`]); `.optional()`, so `null` fails.
fn logit_bias<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<serde_json::Map<String, Value>>, D::Error> {
    let object = serde_json::Map::<String, Value>::deserialize(deserializer)?;
    for (key, value) in &object {
        if !value.is_number() {
            return Err(serde::de::Error::custom(format!(
                "logitBias[{key:?}]: expected a number"
            )));
        }
        if coerce_logit_bias_key(key).is_none() {
            return Err(serde::de::Error::custom(format!(
                "logitBias key {key:?} is not a number"
            )));
        }
    }
    Ok(Some(object))
}

/// `metadata`: `z.record(z.string().max(64), z.string().max(512))`, lengths
/// in UTF-16 code units as JavaScript counts them.
fn metadata<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<serde_json::Map<String, Value>>, D::Error> {
    let object = serde_json::Map::<String, Value>::deserialize(deserializer)?;
    for (key, value) in &object {
        let value = value
            .as_str()
            .ok_or_else(|| serde::de::Error::custom("expected a string value"))?;
        if key.encode_utf16().count() > 64 || value.encode_utf16().count() > 512 {
            return Err(serde::de::Error::custom(
                "metadata keys are at most 64 and values at most 512 characters",
            ));
        }
    }
    Ok(Some(object))
}

/// Parse the `openai` namespace of a chat call: checked against
/// [`OpenAIChatOptions`], then the declared keys kept (a `z.object` strips
/// the rest) with the `logitBias` keys normalized as `z.coerce.number()`
/// writes them back.
///
/// # Errors
///
/// `InvalidArgument` "invalid openai provider options", with the serde error
/// as its cause, when the namespace does not match the schema.
pub(crate) fn parse_chat_options(options: &JsonObject) -> Result<JsonObject, AiMuxError> {
    serde_json::from_value::<OpenAIChatOptions>(Value::Object(options.clone())).map_err(
        |error| AiMuxError::InvalidArgument {
            message: format!(
                "Invalid argument for parameter providerOptions: invalid {NAMESPACE} provider options"
            ),
            cause: Some(ErrorCause::source(&error)),
        },
    )?;
    parse_option_fields(options, NAMESPACE, |key, value| match key {
        // The schema check above has already proven every key coerces.
        "logitBias" => Some(value.as_object().ok_or(()).and_then(|object| {
            let mut parsed = JsonObject::new();
            for (key, value) in object {
                parsed.insert(coerce_logit_bias_key(key).ok_or(())?, value.clone());
            }
            Ok(Value::Object(parsed))
        })),
        "logprobs"
        | "maxCompletionTokens"
        | "metadata"
        | "prediction"
        | "reasoningEffort"
        | "serviceTier"
        | "textVerbosity"
        | "promptCacheRetention"
        | "systemMessageMode"
        | "user"
        | "promptCacheKey"
        | "safetyIdentifier"
        | "parallelToolCalls"
        | "store"
        | "strictJsonSchema"
        | "forceReasoning" => Some(Ok(value.clone())),
        // A nested `z.object`: only its declared keys are kept.
        "promptCacheOptions" => Some(Ok(Value::Object(
            value
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(key, _)| matches!(key.as_str(), "mode" | "ttl"))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ))),
        _ => None,
    })
}

/// `z.coerce.number<string>()` on a `logitBias` key: `Number(key)` as
/// JavaScript reads it (whitespace trimmed, `""` is 0, `0x`/`0o`/`0b`
/// prefixes), written back as `String(number)`. `None` when the result is
/// `NaN` or infinite, which `z.number()` rejects.
fn coerce_logit_bias_key(key: &str) -> Option<String> {
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
            return None;
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
            let digit = digit.to_digit(radix)?;
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
        key.parse::<f64>().ok()?
    };
    if !number.is_finite() {
        return None;
    }
    let key = if number == 0.0 {
        "0".to_string()
    } else if number.abs() >= 1e21 || number.abs() < 1e-6 {
        let scientific = format!("{number:e}");
        let (mantissa, exponent) = scientific.split_once('e')?;
        let exponent = exponent.parse::<i32>().ok()?;
        format!("{mantissa}e{exponent:+}")
    } else {
        number.to_string()
    };
    Some(key)
}

/// Parse a namespace with the option schema supplied by its package.
pub(crate) fn parse_chat_provider_options<T>(
    options: Option<&SharedProviderOptions>,
    provider: &str,
    parse: impl FnOnce(&JsonObject) -> Result<T, AiMuxError>,
) -> Result<Option<T>, AiMuxError> {
    options
        .and_then(|options| options.get(provider))
        .map(parse)
        .transpose()
}

/// Get a value from provider_options.openai.<key>.
pub(crate) fn openai_option(options: &Option<SharedProviderOptions>, key: &str) -> Option<Value> {
    options
        .as_ref()
        .and_then(|m| m.get("openai"))
        .and_then(|o| o.get(key))
        .cloned()
}
