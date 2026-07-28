//! Shared value types: FinishReason, Usage, Warning, ResponseMetadata.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Unified finish reason (why the model stopped generating).
#[derive(Debug, Clone)]
pub struct FinishReason {
    /// The unified reason.
    pub unified: FinishReasonUnified,
    /// The raw provider-specific reason string.
    pub raw: Option<String>,
}

/// Unified finish reason enum (aligned with V4 `LanguageModelV4FinishReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FinishReasonUnified {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
    Error,
    Other,
}

/// Token usage statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: TokenUsage,
    pub output_tokens: TokenUsage,
}

/// Token usage detail (with cache breakdown).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
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
            other => Err(format!("unknown reasoning effort: {}", other)),
        }
    }
}

/// A warning issued by the provider (e.g. unsupported parameter).
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone, Default)]
pub struct ResponseMetadata {
    pub id: Option<String>,
    pub timestamp: Option<String>,
    pub model_id: Option<String>,
}

/// Provider-specific metadata (opaque to core).
pub type ProviderMetadata = Value;
