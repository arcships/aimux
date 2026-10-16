//! Shared value types: FinishReason, Usage, Warning, ResponseMetadata.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// Unified finish reason (why the model stopped generating).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct FinishReason {
    /// The unified reason.
    pub unified: FinishReasonUnified,
    /// The raw provider-specific reason string.
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
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct Usage {
    pub input_tokens: TokenUsage,
    pub output_tokens: TokenUsage,
    /// Raw usage information from the provider (opaque, provider-specific).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

/// Token usage detail (with cache breakdown).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct TokenUsage {
    /// Total tokens.
    pub total: Option<u32>,
    /// Tokens that were not served from cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_cache: Option<u32>,
    /// Tokens served from cache (read).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u32>,
    /// Tokens written to cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<u32>,
    /// Text (non-reasoning) tokens (output side).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<u32>,
    /// Reasoning tokens (output side).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u32>,
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
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum Warning {
    /// A feature is not supported by this provider.
    Unsupported {
        feature: String,
        details: Option<String>,
    },
    /// A feature has compatibility issues.
    Compatibility {
        feature: String,
        details: Option<String>,
    },
    /// A deprecated setting was used.
    Deprecated { setting: String, message: String },
    /// Other warning.
    Other { message: String },
}

/// Metadata about the API response.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub struct ResponseMetadata {
    pub id: Option<String>,
    pub timestamp: Option<String>,
    pub model_id: Option<String>,
}

/// Provider-specific metadata (opaque to core).
pub type ProviderMetadata = Value;

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
