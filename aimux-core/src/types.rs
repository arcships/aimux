//! Shared value types: FinishReason, Usage, Warning, ResponseMetadata.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// Unified finish reason (why the model stopped generating).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct FinishReason {
    /// The unified reason.
    pub unified: FinishReasonUnified,
    /// The raw provider-specific reason string.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// Unified finish reason enum (aligned with V4 `LanguageModelV4FinishReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum FinishReasonUnified {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
    Error,
    Other,
}

/// Token usage statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct Usage {
    pub input_tokens: InputTokenUsage,
    pub output_tokens: OutputTokenUsage,
    /// Raw usage information from the provider (opaque, provider-specific).
    #[ts(optional, type = "Record<string, unknown>")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Map<String, Value>>,
}

/// Input token usage detail.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct InputTokenUsage {
    /// Total tokens.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
    /// Tokens that were not served from cache.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_cache: Option<u32>,
    /// Tokens served from cache (read).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u32>,
    /// Tokens written to cache.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<u32>,
}

/// Output token usage detail.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct OutputTokenUsage {
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
    /// Text (non-reasoning) tokens (output side).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<u32>,
    /// Reasoning tokens (output side).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u32>,
}

/// Token usage of a generation step or a whole call: the AI SDK's
/// `LanguageModelUsage` (ai@7.0.127 `types/usage.ts:10-68`).
///
/// The user-facing counterpart of the provider-level [`Usage`]: flat totals
/// plus detail objects, converted with [`as_language_model_usage`] and summed
/// across steps with [`add_language_model_usage`]. Unknown counts stay `None`,
/// never zero, and are absent from the JSON, as `JSON.stringify` drops an
/// `undefined` count; the detail objects are always present.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct LanguageModelUsage {
    /// Total input (prompt) tokens.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    /// Breakdown of the input tokens.
    #[serde(default)]
    pub input_token_details: InputTokenDetails,
    /// Total output (completion) tokens.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
    /// Breakdown of the output tokens.
    #[serde(default)]
    pub output_token_details: OutputTokenDetails,
    /// Input plus output tokens, when either is known.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u32>,
    /// The provider's raw usage object, passed through unchanged.
    #[ts(optional, type = "Record<string, unknown>")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Map<String, Value>>,
}

/// `LanguageModelUsage.inputTokenDetails`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct InputTokenDetails {
    /// Input tokens that were not read from or written to a cache.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_cache_tokens: Option<u32>,
    /// Input tokens read from a cache.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
    /// Input tokens written to a cache.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u32>,
}

/// `LanguageModelUsage.outputTokenDetails`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct OutputTokenDetails {
    /// Output tokens of text.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_tokens: Option<u32>,
    /// Output tokens spent on reasoning.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
}

/// The AI SDK's `addTokenCounts` (`types/usage.ts:157-164`): `None` only when
/// both counts are unknown.
fn add_token_counts(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0))),
    }
}

/// Convert provider usage to [`LanguageModelUsage`], as the AI SDK's
/// `asLanguageModelUsage` (`types/usage.ts:81-102`): totals and details are
/// copied, `total_tokens` is input plus output, `raw` passes through.
#[must_use]
pub fn as_language_model_usage(usage: &Usage) -> LanguageModelUsage {
    LanguageModelUsage {
        input_tokens: usage.input_tokens.total,
        input_token_details: InputTokenDetails {
            no_cache_tokens: usage.input_tokens.no_cache,
            cache_read_tokens: usage.input_tokens.cache_read,
            cache_write_tokens: usage.input_tokens.cache_write,
        },
        output_tokens: usage.output_tokens.total,
        output_token_details: OutputTokenDetails {
            text_tokens: usage.output_tokens.text,
            reasoning_tokens: usage.output_tokens.reasoning,
        },
        total_tokens: add_token_counts(usage.input_tokens.total, usage.output_tokens.total),
        raw: usage.raw.clone(),
    }
}

/// Sum two usages, as the AI SDK's `addLanguageModelUsage`
/// (`types/usage.ts:122-155`). The sum carries no `raw` value.
#[must_use]
pub fn add_language_model_usage(
    usage1: &LanguageModelUsage,
    usage2: &LanguageModelUsage,
) -> LanguageModelUsage {
    LanguageModelUsage {
        input_tokens: add_token_counts(usage1.input_tokens, usage2.input_tokens),
        input_token_details: InputTokenDetails {
            no_cache_tokens: add_token_counts(
                usage1.input_token_details.no_cache_tokens,
                usage2.input_token_details.no_cache_tokens,
            ),
            cache_read_tokens: add_token_counts(
                usage1.input_token_details.cache_read_tokens,
                usage2.input_token_details.cache_read_tokens,
            ),
            cache_write_tokens: add_token_counts(
                usage1.input_token_details.cache_write_tokens,
                usage2.input_token_details.cache_write_tokens,
            ),
        },
        output_tokens: add_token_counts(usage1.output_tokens, usage2.output_tokens),
        output_token_details: OutputTokenDetails {
            text_tokens: add_token_counts(
                usage1.output_token_details.text_tokens,
                usage2.output_token_details.text_tokens,
            ),
            reasoning_tokens: add_token_counts(
                usage1.output_token_details.reasoning_tokens,
                usage2.output_token_details.reasoning_tokens,
            ),
        },
        total_tokens: add_token_counts(usage1.total_tokens, usage2.total_tokens),
        raw: None,
    }
}

/// Reasoning effort level (aligned with V4 `reasoning` option).
///
/// Controls how much reasoning/thinking the model performs. Maps to
/// OpenAI `reasoning_effort` and Anthropic `thinking` config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum ReasoningEffort {
    /// Provider default — no explicit reasoning config sent.
    #[default]
    ProviderDefault,
    /// No reasoning.
    None,
    /// Minimal reasoning (maps to lowest supported level).
    Minimal,
    /// Low reasoning effort.
    Low,
    /// Medium reasoning effort.
    Medium,
    /// High reasoning effort.
    High,
    /// Extra-high reasoning effort (maps to max on some providers).
    Xhigh,
}

impl ReasoningEffort {
    /// Whether this is an explicit (custom) effort level rather than the
    /// "provider default" sentinel.
    ///
    /// Replaces the per-provider `is_custom_reasoning` helpers that were
    /// copy-pasted across openai/anthropic/xai convert modules (issue M10).
    #[must_use]
    pub fn is_custom(self) -> bool {
        !matches!(self, ReasoningEffort::ProviderDefault)
    }
}

impl std::fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReasoningEffort::ProviderDefault => write!(f, "provider-default"),
            ReasoningEffort::None => write!(f, "none"),
            ReasoningEffort::Minimal => write!(f, "minimal"),
            ReasoningEffort::Low => write!(f, "low"),
            ReasoningEffort::Medium => write!(f, "medium"),
            ReasoningEffort::High => write!(f, "high"),
            ReasoningEffort::Xhigh => write!(f, "xhigh"),
        }
    }
}

impl std::str::FromStr for ReasoningEffort {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "provider-default" => Ok(ReasoningEffort::ProviderDefault),
            "none" => Ok(ReasoningEffort::None),
            "minimal" => Ok(ReasoningEffort::Minimal),
            "low" => Ok(ReasoningEffort::Low),
            "medium" => Ok(ReasoningEffort::Medium),
            "high" => Ok(ReasoningEffort::High),
            "xhigh" => Ok(ReasoningEffort::Xhigh),
            other => Err(format!("unknown reasoning effort: {other}")),
        }
    }
}

/// A warning issued by the provider (e.g. unsupported parameter).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum Warning {
    /// A feature is not supported by this provider.
    Unsupported {
        feature: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<String>,
    },
    /// A feature has compatibility issues.
    Compatibility {
        feature: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<String>,
    },
    /// A deprecated setting was used.
    Deprecated { setting: String, message: String },
    /// Other warning.
    Other { message: String },
}

/// Metadata about the API response.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct ResponseMetadata {
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<crate::timestamp::Timestamp>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

/// Provider-specific metadata: namespace -> JSON object.
pub type ProviderMetadata = crate::shared::SharedProviderMetadata;

/// A `${string}.${string}` identifier, `<provider-id>.<name>`: the V4
/// provider tool `id` (`@ai-sdk/provider@4.0.21
/// src/language-model/v4/language-model-v4-provider-tool.ts:17`) and custom
/// content `kind` (`language-model-v4-custom-content.ts:13`,
/// `language-model-v4-prompt.ts:138`).
///
/// The TypeScript type admits any string with a `.`; a value without one
/// does not construct or deserialize.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, TS)]
#[serde(transparent)]
#[ts(type = "`${string}.${string}`")]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct NamespacedKind(String);

impl NamespacedKind {
    /// Checks that `value` has the `${string}.${string}` shape.
    ///
    /// # Errors
    ///
    /// [`AiMuxError::InvalidArgument`](crate::error::AiMuxError::InvalidArgument)
    /// when `value` has no `.`.
    pub fn new(value: impl Into<String>) -> Result<Self, crate::error::AiMuxError> {
        let value = value.into();
        if value.contains('.') {
            Ok(Self(value))
        } else {
            Err(crate::error::AiMuxError::InvalidArgument(format!(
                "expected a `<provider-id>.<name>` identifier, got {value:?}"
            )))
        }
    }

    /// The identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NamespacedKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for NamespacedKind {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for NamespacedKind {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for NamespacedKind {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl From<NamespacedKind> for String {
    fn from(kind: NamespacedKind) -> Self {
        kind.0
    }
}

impl TryFrom<String> for NamespacedKind {
    type Error = crate::error::AiMuxError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for NamespacedKind {
    type Error = crate::error::AiMuxError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl std::str::FromStr for NamespacedKind {
    type Err = crate::error::AiMuxError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for NamespacedKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider_usage() -> Usage {
        Usage {
            input_tokens: InputTokenUsage {
                total: Some(10),
                no_cache: Some(6),
                cache_read: Some(4),
                cache_write: None,
            },
            output_tokens: OutputTokenUsage {
                total: Some(5),
                text: Some(3),
                reasoning: Some(2),
            },
            raw: Some(serde_json::Map::from_iter([(
                "prompt_tokens".to_string(),
                Value::from(10),
            )])),
        }
    }

    /// `asLanguageModelUsage` (ai@7.0.127 `types/usage.ts:81-102`) copies the
    /// totals and details, adds input and output into `totalTokens` and
    /// passes `raw` through.
    #[test]
    fn provider_usage_converts_as_the_ai_sdk() {
        let usage = as_language_model_usage(&provider_usage());
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.input_token_details.no_cache_tokens, Some(6));
        assert_eq!(usage.input_token_details.cache_read_tokens, Some(4));
        assert_eq!(usage.input_token_details.cache_write_tokens, None);
        assert_eq!(usage.output_tokens, Some(5));
        assert_eq!(usage.output_token_details.text_tokens, Some(3));
        assert_eq!(usage.output_token_details.reasoning_tokens, Some(2));
        assert_eq!(usage.total_tokens, Some(15));
        assert_eq!(usage.raw, provider_usage().raw);

        // `addTokenCounts(undefined, 5)` is 5: one known side is a total.
        let output_only = as_language_model_usage(&Usage {
            output_tokens: OutputTokenUsage {
                total: Some(5),
                ..OutputTokenUsage::default()
            },
            ..Usage::default()
        });
        assert_eq!(output_only.total_tokens, Some(5));
        assert_eq!(output_only.input_tokens, None);
    }

    /// `addLanguageModelUsage` (`types/usage.ts:122-155`) sums each count,
    /// keeps a count unknown only when both sides are (`addTokenCounts`,
    /// `:157-164`), and returns no `raw`. The JSON is the AI SDK's
    /// `LanguageModelUsage` (`:10-67`) through `JSON.stringify`: camelCase
    /// counts and detail objects, an unknown (`undefined`) count absent.
    #[test]
    fn usages_add_as_the_ai_sdk() {
        let step = as_language_model_usage(&provider_usage());
        let total = add_language_model_usage(&LanguageModelUsage::default(), &step);
        assert_eq!(total.input_tokens, Some(10));
        assert_eq!(total.input_token_details.cache_write_tokens, None);
        assert_eq!(total.total_tokens, Some(15));
        assert_eq!(total.raw, None);

        let twice = add_language_model_usage(&total, &step);
        assert_eq!(twice.input_tokens, Some(20));
        assert_eq!(twice.output_token_details.reasoning_tokens, Some(4));
        assert_eq!(twice.total_tokens, Some(30));

        assert_eq!(
            serde_json::to_value(&total).unwrap(),
            serde_json::json!({
                "inputTokens": 10,
                "inputTokenDetails": { "noCacheTokens": 6, "cacheReadTokens": 4 },
                "outputTokens": 5,
                "outputTokenDetails": { "textTokens": 3, "reasoningTokens": 2 },
                "totalTokens": 15,
            })
        );
        // An unknown usage keeps its detail objects, empty.
        assert_eq!(
            serde_json::to_value(LanguageModelUsage::default()).unwrap(),
            serde_json::json!({ "inputTokenDetails": {}, "outputTokenDetails": {} })
        );
    }
}
