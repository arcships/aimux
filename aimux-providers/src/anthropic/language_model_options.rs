//! The providerOptions namespace of the Anthropic Messages protocol.
//!
//! `@ai-sdk/anthropic` always reads `providerOptions.anthropic` and, when the
//! provider was created with a custom `name`, also `providerOptions[<first
//! segment of name>]`; the custom key wins where both set the same option.
//! What the model writes back (reasoning signatures, MCP server names, ...)
//! is keyed by that custom name too, so a transcript round-trips under the
//! name the caller chose. This module is the one place that knows the
//! canonical key and how the two are merged.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;
use aimux_core::shared::{JsonObject, SharedProviderOptions};
use aimux_provider_utils::optional_field;

/// The canonical providerOptions key, read for every model of the protocol
/// whatever its name.
pub(crate) const CANONICAL: &str = "anthropic";

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

/// The Anthropic options of a call: `anthropic` and, for a custom name, the
/// `name` namespace, each parsed against `anthropicLanguageModelOptions` and
/// then merged shallowly with the custom one winning, as
/// anthropic-language-model.ts:376-398 does. `None` when neither is set.
///
/// # Errors
///
/// [`parse_anthropic_options`]'s, for the first namespace that fails.
pub(super) fn parse_anthropic_namespaces(
    provider_options: Option<&SharedProviderOptions>,
    name: &str,
) -> Result<Option<JsonObject>, AiMuxError> {
    let parse = |key: &str| {
        provider_options
            .and_then(|options| options.get(key))
            .map(|options| parse_anthropic_options(key, options))
            .transpose()
    };
    let canonical = parse(CANONICAL)?;
    let custom = if name == CANONICAL {
        None
    } else {
        parse(name)?
    };
    Ok(merge(canonical.as_ref(), custom.as_ref()))
}

/// Parse one namespace against [`AnthropicOptions`]: the declared keys are
/// kept, in the order given, and the rest stripped, as a `z.object` strips
/// them.
///
/// # Errors
///
/// `InvalidArgument` "invalid {provider} provider options", with the serde
/// error as its cause, when the namespace does not match the schema.
pub(super) fn parse_anthropic_options(
    provider: &str,
    options: &JsonObject,
) -> Result<JsonObject, AiMuxError> {
    let invalid = |cause: serde_json::Error| AiMuxError::InvalidArgument {
        message: format!(
            "Invalid argument for parameter providerOptions: invalid {provider} provider options"
        ),
        cause: Some(ErrorCause::source(&cause)),
    };
    let input = Value::Object(options.clone());
    let parsed = AnthropicOptions::deserialize(&input).map_err(invalid)?;
    let declared = JsonObject::deserialize(serde_json::to_value(&parsed).map_err(invalid)?)
        .map_err(invalid)?;
    Ok(retain_declared(options, &declared))
}

/// `input` with, at every level, only the keys `declared` has, in `input`'s
/// order and with `input`'s values. `declared` is the schema's own
/// serialization of `input`: a key the schema strips is not in it (a key it
/// declares but `input` lacks is, as `null`, and is never looked up).
fn retain_declared(input: &JsonObject, declared: &JsonObject) -> JsonObject {
    input
        .iter()
        .filter_map(|(key, value)| {
            declared
                .get(key)
                .map(|declared| (key.clone(), retain_declared_value(value, declared)))
        })
        .collect()
}

fn retain_declared_value(input: &Value, declared: &Value) -> Value {
    match (input, declared) {
        (Value::Object(input), Value::Object(declared)) => {
            Value::Object(retain_declared(input, declared))
        }
        (Value::Array(input), Value::Array(declared)) => Value::Array(
            input
                .iter()
                .zip(declared)
                .map(|(value, declared)| retain_declared_value(value, declared))
                .collect(),
        ),
        _ => input.clone(),
    }
}

/// A `.nullish()` field: absent is `None`, `null` is `Some(None)` and is
/// written back as `null`.
fn nullish<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// `Number.MAX_SAFE_INTEGER`, the bound of `z.number().int()`.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// `z.number().int().min(min)`: a safe integer of at least `min`.
fn safe_integer<E: serde::de::Error>(number: f64, min: f64) -> Result<f64, E> {
    if number.fract() == 0.0 && (min..=MAX_SAFE_INTEGER).contains(&number) {
        Ok(number)
    } else {
        Err(E::custom(format!(
            "expected an integer from {min} to {MAX_SAFE_INTEGER}, got {number}"
        )))
    }
}

/// `taskBudget.total`: `z.number().int().min(20000)`.
fn task_budget_total<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    safe_integer(f64::deserialize(deserializer)?, 20_000.0)
}

/// `taskBudget.remaining`: `z.number().int().min(0).optional()`.
fn task_budget_remaining<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    safe_integer(f64::deserialize(deserializer)?, 0.0).map(Some)
}

/// `fallbacks[].max_tokens`: `z.number().int().optional()`.
fn fallback_max_tokens<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    safe_integer(f64::deserialize(deserializer)?, -MAX_SAFE_INTEGER).map(Some)
}

/// `anthropicLanguageModelOptions` (anthropic-language-model-options.ts,
/// `@ai-sdk/anthropic` 4.0.71). Every key is `.optional()`: `null` fails as
/// a wrong type does. The parsed namespace is the input with the keys this
/// schema keeps ([`parse_anthropic_options`]), so the values reach the
/// request as the caller wrote them.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnthropicOptions {
    #[serde(default, deserialize_with = "optional_field")]
    send_reasoning: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    structured_output_mode: Option<StructuredOutputMode>,
    #[serde(default, deserialize_with = "optional_field")]
    thinking: Option<Thinking>,
    #[serde(default, deserialize_with = "optional_field")]
    disable_parallel_tool_use: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    cache_control: Option<CacheControl>,
    #[serde(default, deserialize_with = "optional_field")]
    metadata: Option<Metadata>,
    #[serde(default, deserialize_with = "optional_field")]
    mcp_servers: Option<Vec<McpServer>>,
    #[serde(default, deserialize_with = "optional_field")]
    container: Option<Container>,
    #[serde(default, deserialize_with = "optional_field")]
    tool_streaming: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    effort: Option<Effort>,
    #[serde(default, deserialize_with = "optional_field")]
    task_budget: Option<TaskBudget>,
    #[serde(default, deserialize_with = "optional_field")]
    speed: Option<Speed>,
    #[serde(default, deserialize_with = "optional_field")]
    service_tier: Option<ServiceTier>,
    #[serde(default, deserialize_with = "optional_field")]
    inference_geo: Option<InferenceGeo>,
    #[serde(default, deserialize_with = "optional_field")]
    fallbacks: Option<Fallbacks>,
    #[serde(default, deserialize_with = "optional_field")]
    anthropic_beta: Option<Vec<String>>,
    #[serde(default, deserialize_with = "optional_field")]
    safeguards: Option<Vec<Safeguard>>,
    #[serde(default, deserialize_with = "optional_field")]
    compaction: Option<Compaction>,
    #[serde(default, deserialize_with = "optional_field")]
    context_management: Option<ContextManagement>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum StructuredOutputMode {
    OutputFormat,
    JsonTool,
    Auto,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Speed {
    Fast,
    Standard,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ServiceTier {
    Auto,
    StandardOnly,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum InferenceGeo {
    Us,
    Global,
}

/// `thinking`: the variants of a `type` discriminated union, or, with no
/// `type` (`z.never().optional()`), `blockBinding` alone.
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum Thinking {
    Typed(TypedThinking),
    Untyped(UntypedThinking),
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TypedThinking {
    Adaptive(AdaptiveThinking),
    Enabled(EnabledThinking),
    Disabled,
    BetweenTools,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AdaptiveThinking {
    #[serde(default, deserialize_with = "optional_field")]
    display: Option<ThinkingDisplay>,
    #[serde(default, deserialize_with = "optional_field")]
    block_binding: Option<BlockBinding>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnabledThinking {
    #[serde(default, deserialize_with = "optional_field")]
    budget_tokens: Option<f64>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UntypedThinking {
    /// `type: z.never().optional()`: any `type`, `null` included, fails.
    #[serde(
        rename = "type",
        default,
        deserialize_with = "optional_field",
        skip_serializing
    )]
    #[allow(dead_code, reason = "a present `type` fails to deserialize")]
    no_type: Option<Never>,
    block_binding: BlockBinding,
}

/// A value no input deserializes to.
#[derive(Deserialize)]
enum Never {}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum ThinkingDisplay {
    Omitted,
    Summarized,
    Updates,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BlockBinding {
    prefix_mismatch_behavior: PrefixMismatchBehavior,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PrefixMismatchBehavior {
    Error,
    DropBlock,
}

#[derive(Deserialize, Serialize)]
struct CacheControl {
    #[serde(rename = "type")]
    kind: EphemeralType,
    #[serde(default, deserialize_with = "optional_field")]
    ttl: Option<CacheTtl>,
}

#[derive(Deserialize, Serialize)]
enum EphemeralType {
    #[serde(rename = "ephemeral")]
    Ephemeral,
}

#[derive(Deserialize, Serialize)]
enum CacheTtl {
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "1h")]
    OneHour,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    #[serde(default, deserialize_with = "optional_field")]
    user_id: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct McpServer {
    #[serde(rename = "type")]
    kind: UrlType,
    name: String,
    url: String,
    #[serde(default, deserialize_with = "nullish")]
    authorization_token: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullish")]
    tool_configuration: Option<Option<ToolConfiguration>>,
}

#[derive(Deserialize, Serialize)]
enum UrlType {
    #[serde(rename = "url")]
    Url,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolConfiguration {
    #[serde(default, deserialize_with = "nullish")]
    enabled: Option<Option<bool>>,
    #[serde(default, deserialize_with = "nullish")]
    allowed_tools: Option<Option<Vec<String>>>,
}

#[derive(Deserialize, Serialize)]
struct Container {
    #[serde(default, deserialize_with = "optional_field")]
    id: Option<String>,
    #[serde(default, deserialize_with = "optional_field")]
    skills: Option<Vec<Skill>>,
}

/// A container skill, discriminated by `type`: a provider-managed
/// (`"anthropic"`) or a custom one.
#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Skill {
    Anthropic(ProviderSkill),
    Custom(CustomSkill),
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderSkill {
    skill_id: String,
    #[serde(default, deserialize_with = "optional_field")]
    version: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CustomSkill {
    provider_reference: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "optional_field")]
    version: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct TaskBudget {
    #[serde(rename = "type")]
    kind: TokensType,
    #[serde(deserialize_with = "task_budget_total")]
    total: f64,
    #[serde(default, deserialize_with = "task_budget_remaining")]
    remaining: Option<f64>,
}

#[derive(Deserialize, Serialize)]
enum TokensType {
    #[serde(rename = "tokens")]
    Tokens,
}

/// `fallbacks`: `"default"`, or the fallback models in order.
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum Fallbacks {
    Default(DefaultFallbacks),
    Models(Vec<Fallback>),
}

#[derive(Deserialize, Serialize)]
enum DefaultFallbacks {
    #[serde(rename = "default")]
    Default,
}

/// A fallback model. Its keys are the wire's, snake_case.
#[derive(Deserialize, Serialize)]
struct Fallback {
    model: String,
    #[serde(default, deserialize_with = "fallback_max_tokens")]
    max_tokens: Option<f64>,
    #[serde(default, deserialize_with = "optional_field")]
    thinking: Option<JsonObject>,
    #[serde(default, deserialize_with = "optional_field")]
    output_config: Option<JsonObject>,
    #[serde(default, deserialize_with = "optional_field")]
    speed: Option<Speed>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Safeguard {
    #[serde(rename = "type")]
    kind: DangerousToolUseType,
    #[serde(default, deserialize_with = "optional_field")]
    classifier_context: Option<JsonObject>,
}

#[derive(Deserialize, Serialize)]
enum DangerousToolUseType {
    #[serde(rename = "dangerous_tool_use")]
    DangerousToolUse,
}

#[derive(Deserialize, Serialize)]
struct Compaction {
    #[serde(rename = "type")]
    kind: SummarizeType,
    #[serde(default, deserialize_with = "optional_field")]
    instructions: Option<String>,
}

#[derive(Deserialize, Serialize)]
enum SummarizeType {
    #[serde(rename = "summarize")]
    Summarize,
}

#[derive(Deserialize, Serialize)]
struct ContextManagement {
    edits: Vec<ContextEdit>,
}

/// A context-management edit, discriminated by its strategy `type`.
#[derive(Deserialize, Serialize)]
#[serde(tag = "type")]
enum ContextEdit {
    #[serde(rename = "clear_tool_uses_20250919")]
    ClearToolUses(ClearToolUses),
    #[serde(rename = "clear_thinking_20251015")]
    ClearThinking(ClearThinking),
    #[serde(rename = "compact_20260112")]
    Compact(Compact),
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClearToolUses {
    #[serde(default, deserialize_with = "optional_field")]
    trigger: Option<ClearToolUsesTrigger>,
    #[serde(default, deserialize_with = "optional_field")]
    keep: Option<ToolUses>,
    #[serde(default, deserialize_with = "optional_field")]
    clear_at_least: Option<InputTokens>,
    #[serde(default, deserialize_with = "optional_field")]
    clear_tool_inputs: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    exclude_tools: Option<Vec<String>>,
}

/// `clear_tool_uses_20250919.trigger`, discriminated by `type`.
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum ClearToolUsesTrigger {
    InputTokens(InputTokens),
    ToolUses(ToolUses),
}

#[derive(Deserialize, Serialize)]
struct ClearThinking {
    #[serde(default, deserialize_with = "optional_field")]
    keep: Option<KeepThinking>,
}

/// `clear_thinking_20251015.keep`: `"all"`, or a number of thinking turns.
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum KeepThinking {
    All(AllType),
    Turns(ThinkingTurns),
}

#[derive(Deserialize, Serialize)]
enum AllType {
    #[serde(rename = "all")]
    All,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Compact {
    #[serde(default, deserialize_with = "optional_field")]
    trigger: Option<InputTokens>,
    #[serde(default, deserialize_with = "optional_field")]
    pause_after_compaction: Option<bool>,
    #[serde(default, deserialize_with = "optional_field")]
    instructions: Option<String>,
}

/// `{ type: 'input_tokens', value: number }`.
#[derive(Deserialize, Serialize)]
struct InputTokens {
    #[serde(rename = "type")]
    kind: InputTokensType,
    value: f64,
}

#[derive(Deserialize, Serialize)]
enum InputTokensType {
    #[serde(rename = "input_tokens")]
    InputTokens,
}

/// `{ type: 'tool_uses', value: number }`.
#[derive(Deserialize, Serialize)]
struct ToolUses {
    #[serde(rename = "type")]
    kind: ToolUsesType,
    value: f64,
}

#[derive(Deserialize, Serialize)]
enum ToolUsesType {
    #[serde(rename = "tool_uses")]
    ToolUses,
}

/// `{ type: 'thinking_turns', value: number }`.
#[derive(Deserialize, Serialize)]
struct ThinkingTurns {
    #[serde(rename = "type")]
    kind: ThinkingTurnsType,
    value: f64,
}

#[derive(Deserialize, Serialize)]
enum ThinkingTurnsType {
    #[serde(rename = "thinking_turns")]
    ThinkingTurns,
}
