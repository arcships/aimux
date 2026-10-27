//! The providerOptions namespace of the Anthropic Messages protocol.
//!
//! `@ai-sdk/anthropic` always reads `providerOptions.anthropic` and, when the
//! provider was created with a custom `name`, also `providerOptions[<first
//! segment of name>]`; the custom key wins where both set the same option.
//! What the model writes back (reasoning signatures, MCP server names, ...)
//! is keyed by that custom name too, so a transcript round-trips under the
//! name the caller chose. This module is the one place that knows the
//! canonical key and how the two are merged.

use serde_json::Value;

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The canonical providerOptions key, read for every model of the protocol
/// whatever its name.
pub(crate) const CANONICAL: &str = "anthropic";

/// The type discriminator for provider-managed container skills.
pub(crate) const PROVIDER_SKILL_TYPE: &str = CANONICAL;

/// The providerOptions key of a provider name: its first dot-separated
/// segment, trimmed (`"proxy.messages"` -> `"proxy"`, `"proxy"` -> `"proxy"`).
pub(crate) fn options_name_of(provider: &str) -> String {
    provider
        .split('.')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Merge the canonical options with the custom-name options (custom wins,
/// shallowly). `None` when neither is set.
fn merge(canonical: Option<&JsonObject>, custom: Option<&JsonObject>) -> Option<JsonObject> {
    match (canonical, custom) {
        (None, None) => None,
        (Some(only), None) | (None, Some(only)) => Some(only.clone()),
        (Some(canonical), Some(custom)) => {
            let mut merged = canonical.clone();
            merged.extend(custom.iter().map(|(k, v)| (k.clone(), v.clone())));
            Some(merged)
        }
    }
}

/// The Anthropic options in a providerOptions object (a part's, a message's):
/// `anthropic` merged with `name` when `name` is a different key.
pub(crate) fn anthropic_options(
    provider_options: Option<&SharedProviderOptions>,
    name: &str,
) -> Option<JsonObject> {
    let canonical = provider_options.and_then(|options| options.get(CANONICAL));
    let custom = (name != CANONICAL)
        .then(|| provider_options.and_then(|options| options.get(name)))
        .flatten();
    merge(canonical, custom)
}

/// [`anthropic_options`] for `CallOptions::provider_options` or a tool's options.
pub(crate) fn anthropic_options_in(
    provider_options: Option<&SharedProviderOptions>,
    name: &str,
) -> Option<JsonObject> {
    anthropic_options(provider_options, name)
}

pub(super) fn parse_anthropic_option_object(shape: &str, value: &Value) -> Result<Value, ()> {
    let object = value.as_object().ok_or(())?;
    let parsed = crate::openai::convert::parse_option_fields(object, CANONICAL, |key, value| {
        let kind = match (shape, key) {
            ("options", "sendReasoning" | "disableParallelToolUse" | "toolStreaming") => "bool",
            ("options", "structuredOutputMode") => "outputFormat|jsonTool|auto",
            ("options", "thinking") => "thinking",
            ("options", "cacheControl") => "cache",
            ("options", "metadata") => "metadata",
            ("options", "mcpServers") => "[]mcp",
            ("options", "container") => "container",
            ("options", "effort") => "low|medium|high|xhigh|max",
            ("options", "taskBudget") => "taskBudget",
            ("options", "speed") | ("fallback", "speed") => "fast|standard",
            ("options", "serviceTier") => "auto|standard_only",
            ("options", "inferenceGeo") => "us|global",
            ("options", "fallbacks") => "fallbacks",
            ("options", "anthropicBeta") => "[]string",
            ("options", "safeguards") => "[]safeguard",
            ("options", "compaction") => "compaction",
            ("options", "contextManagement") => "context",
            ("thinking", "type") => "adaptive|enabled|disabled|between_tools",
            ("thinking", "display")
                if object.get("type").and_then(Value::as_str) == Some("adaptive") =>
            {
                "omitted|summarized|updates"
            }
            ("thinking", "budgetTokens")
                if object.get("type").and_then(Value::as_str) == Some("enabled") =>
            {
                "number"
            }
            ("thinking", "blockBinding")
                if object
                    .get("type")
                    .is_none_or(|t| t.as_str() == Some("adaptive")) =>
            {
                "binding"
            }
            ("binding", "prefixMismatchBehavior") => "error|drop_block",
            ("cache", "type") => "ephemeral",
            ("cache", "ttl") => "5m|1h",
            ("metadata", "userId") => "string",
            ("mcp", "type") => "url",
            ("mcp", "name" | "url") => "string",
            ("mcp", "authorizationToken") => "?string",
            ("mcp", "toolConfiguration") => "?toolConfiguration",
            ("toolConfiguration", "enabled") => "?bool",
            ("toolConfiguration", "allowedTools") => "?[]string",
            ("container", "id") | ("skill", "version") => "string",
            ("container", "skills") => "[]skill",
            ("skill", "type") => "anthropic|custom",
            ("skill", "skillId")
                if object.get("type").and_then(Value::as_str) == Some(PROVIDER_SKILL_TYPE) =>
            {
                "string"
            }
            ("skill", "providerReference")
                if object.get("type").and_then(Value::as_str) == Some("custom") =>
            {
                "stringRecord"
            }
            ("taskBudget", "type") => "tokens",
            ("taskBudget", "total") => "total",
            ("taskBudget", "remaining") => "remaining",
            ("fallback", "model") => "string",
            ("fallback", "max_tokens") => "integer",
            ("fallback", "thinking" | "output_config") | ("safeguard", "classifierContext") => {
                "record"
            }
            ("safeguard", "type") => "dangerous_tool_use",
            ("compaction", "type") => "summarize",
            ("compaction", "instructions") => "string",
            ("edit", "instructions")
                if object.get("type").and_then(Value::as_str) == Some("compact_20260112") =>
            {
                "string"
            }
            ("context", "edits") => "[]edit",
            ("edit", "type") => "clear_tool_uses_20250919|clear_thinking_20251015|compact_20260112",
            ("edit", "trigger")
                if object.get("type").and_then(Value::as_str)
                    == Some("clear_tool_uses_20250919") =>
            {
                "trigger"
            }
            ("edit", "trigger")
                if object.get("type").and_then(Value::as_str) == Some("compact_20260112") =>
            {
                "inputTokens"
            }
            ("edit", "clearAtLeast")
                if object.get("type").and_then(Value::as_str)
                    == Some("clear_tool_uses_20250919") =>
            {
                "inputTokens"
            }
            ("edit", "keep")
                if object.get("type").and_then(Value::as_str)
                    == Some("clear_tool_uses_20250919") =>
            {
                "toolUses"
            }
            ("edit", "keep")
                if object.get("type").and_then(Value::as_str)
                    == Some("clear_thinking_20251015") =>
            {
                "keepThinking"
            }
            ("edit", "clearToolInputs")
                if object.get("type").and_then(Value::as_str)
                    == Some("clear_tool_uses_20250919") =>
            {
                "bool"
            }
            ("edit", "excludeTools")
                if object.get("type").and_then(Value::as_str)
                    == Some("clear_tool_uses_20250919") =>
            {
                "[]string"
            }
            ("edit", "pauseAfterCompaction")
                if object.get("type").and_then(Value::as_str) == Some("compact_20260112") =>
            {
                "bool"
            }
            ("trigger", "type") => "input_tokens|tool_uses",
            ("inputTokens", "type") => "input_tokens",
            ("toolUses", "type") => "tool_uses",
            ("thinkingTurns", "type") => "thinking_turns",
            ("trigger" | "inputTokens" | "toolUses" | "thinkingTurns", "value") => "number",
            _ => return None,
        };
        Some(parse_anthropic_option_value(kind, value))
    })
    .map_err(|_| ())?;
    let required: &[&str] = match shape {
        "thinking" if !object.contains_key("type") => &["blockBinding"],
        "thinking" | "cache" | "compaction" | "safeguard" | "edit" => &["type"],
        "binding" => &["prefixMismatchBehavior"],
        "mcp" => &["type", "name", "url"],
        "skill" if object.get("type").and_then(Value::as_str) == Some("custom") => {
            &["type", "providerReference"]
        }
        "skill" => &["type", "skillId"],
        "taskBudget" => &["type", "total"],
        "fallback" => &["model"],
        "context" => &["edits"],
        "trigger" | "inputTokens" | "toolUses" | "thinkingTurns" => &["type", "value"],
        _ => &[],
    };
    if required.iter().any(|key| !parsed.contains_key(*key)) {
        return Err(());
    }
    Ok(Value::Object(parsed))
}

fn parse_anthropic_option_value(kind: &str, value: &Value) -> Result<Value, ()> {
    if let Some(kind) = kind.strip_prefix('?') {
        return if value.is_null() {
            Ok(Value::Null)
        } else {
            parse_anthropic_option_value(kind, value)
        };
    }
    if let Some(kind) = kind.strip_prefix("[]") {
        return value
            .as_array()
            .ok_or(())?
            .iter()
            .map(|v| parse_anthropic_option_value(kind, v))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array);
    }
    let valid = match kind {
        "bool" => value.is_boolean(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value
            .as_f64()
            .is_some_and(|n| n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0),
        "total" => value
            .as_f64()
            .is_some_and(|n| n.fract() == 0.0 && (20000.0..=9_007_199_254_740_991.0).contains(&n)),
        "remaining" => value
            .as_f64()
            .is_some_and(|n| n.fract() == 0.0 && (0.0..=9_007_199_254_740_991.0).contains(&n)),
        "record" => value.is_object(),
        "stringRecord" => value
            .as_object()
            .is_some_and(|o| o.values().all(Value::is_string)),
        "fallbacks" => {
            return if value.as_str() == Some("default") {
                Ok(value.clone())
            } else {
                parse_anthropic_option_value("[]fallback", value)
            };
        }
        "keepThinking" => {
            return if value.as_str() == Some("all") {
                Ok(value.clone())
            } else {
                parse_anthropic_option_object("thinkingTurns", value)
            };
        }
        "thinking" | "binding" | "cache" | "metadata" | "mcp" | "toolConfiguration"
        | "container" | "skill" | "taskBudget" | "fallback" | "safeguard" | "compaction"
        | "context" | "edit" | "trigger" | "inputTokens" | "toolUses" | "thinkingTurns" => {
            return parse_anthropic_option_object(kind, value);
        }
        _ => value
            .as_str()
            .is_some_and(|s| kind.split('|').any(|allowed| allowed == s)),
    };
    if valid { Ok(value.clone()) } else { Err(()) }
}
