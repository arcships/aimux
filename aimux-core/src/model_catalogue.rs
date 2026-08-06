//! Model catalogue types (RFC-0027).
//!
//! Types describing models discovered at runtime (`RuntimeModel`) and the
//! static capability portrait supplemented from community knowledge
//! (`ModelSpec`). `list_models` merges them into `ResolvedModel`.
//!
//! All `ModelSpec` fields are **advisory** — they are surfaced to the caller so
//! they can decide how to configure a request; aimux never auto-applies them in
//! the request path. Missing fields are `None`/`false` (loose deserialization).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ─────────────────────────────────────────────────────────────────────────────
// Runtime discovery (provider `/models`)
// ─────────────────────────────────────────────────────────────────────────────

/// A model entry returned by a provider's `/models` endpoint (the sparse,
/// account-level truth of "what this key can call").
///
/// Carries only what the provider returns — typically just `id`. Configuration
/// comes from [`ModelSpec`] (community knowledge), merged into
/// [`ResolvedModel`].
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export)]
pub struct RuntimeModel {
    /// Model identifier, e.g. `"gpt-4o"`.
    pub id: String,
    /// `owned_by` field from OpenAI-compatible `/models` responses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
    /// `created` unix timestamp from OpenAI-compatible `/models` responses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | null")]
    pub created: Option<u64>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Static portrait (community knowledge, e.g. anya2a)
// ─────────────────────────────────────────────────────────────────────────────

/// Where a [`ModelSpec`] came from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
pub enum CatalogueSource {
    /// `models.anya2a.com` (DeepChat PublicProviderConf, aggregates models.dev).
    #[default]
    Anya2a,
    /// `models.dev/api.json` (degraded fallback).
    ModelsDev,
    /// Manually curated / user-supplied.
    Manual,
}

/// Coarse model category, used to route to the right modality surface.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
pub enum ModelType {
    #[default]
    Chat,
    Completion,
    Embedding,
    Rerank,
    #[serde(alias = "image-generation", alias = "imageGeneration")]
    ImageGen,
    #[serde(alias = "image-edit")]
    ImageEdit,
    Video,
    Audio,
    /// Unknown / unmapped type.
    #[serde(other)]
    Other,
}

/// A single input or output modality.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
pub enum Modality {
    Text,
    Image,
    Audio,
    Video,
    Pdf,
    /// Embedding vector output.
    Embedding,
    /// Relevance score output (rerankers).
    Score,
    #[serde(other)]
    Other,
}

/// Token / request limits for a model.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS, PartialEq)]
#[ts(export)]
pub struct ModelLimits {
    /// Maximum total context window (input + output), in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | null")]
    pub context: Option<u64>,
    /// Maximum output tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | null")]
    pub output: Option<u64>,
    /// Maximum input tokens (when distinct from `context`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | null")]
    pub input: Option<u64>,
}

/// Input/output modality sets.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS, PartialEq)]
#[ts(export)]
pub struct ModelModalities {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<Modality>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output: Vec<Modality>,
}

/// Model-level capability flags (more precise than provider-level `supports_*`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS, PartialEq)]
#[ts(export)]
pub struct ModelCapabilities {
    /// Supports function/tool calling.
    #[serde(default)]
    pub tool_call: bool,
    /// Supports structured / JSON-schema output.
    #[serde(default, rename = "structured_output")]
    pub structured_output: bool,
    /// Accepts a `temperature` parameter.
    #[serde(default)]
    pub temperature: bool,
    /// Accepts multimodal attachments (image/audio/pdf input).
    #[serde(default)]
    pub attachment: bool,
}

/// How reasoning / thinking is controlled for a model.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
pub enum ReasoningMode {
    /// Token-budget control (e.g. Anthropic `budget_tokens`).
    Budget,
    /// Effort-level control (e.g. `low`/`medium`/`high`).
    Effort,
    /// Discrete level control.
    Level,
    /// Always on, not user-controllable.
    Fixed,
    /// Multiple control modes coexist.
    Mixed,
    /// Simple on/off toggle.
    Toggle,
    #[serde(other)]
    Other,
}

/// Visibility of reasoning output to the caller.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
pub enum ReasoningVisibility {
    Hidden,
    Summary,
    Full,
    Mixed,
    Omitted,
    #[serde(other)]
    Other,
}

/// Reasoning / thinking portrait for a model (advisory).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS, PartialEq)]
#[ts(export)]
pub struct ReasoningSpec {
    /// Whether the model family supports reasoning at all.
    #[serde(default)]
    pub supported: bool,
    /// Whether reasoning should be on by default.
    #[serde(default)]
    pub default_enabled: bool,
    /// Primary control mode.
    #[serde(default)]
    pub mode: Option<ReasoningMode>,
    /// Default effort level (when `mode` is Effort/Mixed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_default: Option<String>,
    /// Supported effort levels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effort_options: Vec<String>,
    /// Minimum token budget (when `mode` is Budget/Mixed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | null")]
    pub budget_min: Option<u64>,
    /// Interleaved reasoning (thinking blocks between tool calls).
    #[serde(default)]
    pub interleaved: bool,
    /// Output visibility.
    #[serde(default)]
    pub visibility: Option<ReasoningVisibility>,
}

/// Pricing (advisory, read-only metadata). Units follow the source catalog
/// (anya2a uses USD per 1M tokens).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS, PartialEq)]
#[ts(export)]
pub struct ModelCost {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
}

/// The full static portrait for a model, supplemented from community knowledge.
///
/// All fields are advisory — aimux never auto-applies them in the request path.
/// The caller reads them to decide how to configure `GenerateTextOptions` /
/// `bodyOverrides`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS, PartialEq)]
#[ts(export)]
pub struct ModelSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub r#type: ModelType,
    #[serde(default)]
    pub limits: ModelLimits,
    #[serde(default)]
    pub modalities: ModelModalities,
    #[serde(default)]
    pub capabilities: ModelCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<ModelCost>,
    #[serde(default)]
    pub source: CatalogueSource,
    /// Provider name (normalized to aimux registry spelling).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Verbatim source entry, preserved for forward compatibility / debugging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Merged result
// ─────────────────────────────────────────────────────────────────────────────

/// A model discovered at runtime, optionally enriched with a static portrait.
///
/// `spec` is `Some` when community knowledge (anya2a) had an entry for this
/// `(provider, model_id)`; otherwise `None` and the caller falls back to
/// provider-level defaults (today's behaviour).
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export)]
pub struct ResolvedModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | null")]
    pub created: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<ModelSpec>,
}

impl ResolvedModel {
    /// Build a `ResolvedModel` from a runtime entry, attaching a portrait if one
    /// is available.
    pub fn from_runtime(runtime: RuntimeModel, spec: Option<ModelSpec>) -> Self {
        Self {
            id: runtime.id,
            owned_by: runtime.owned_by,
            created: runtime.created,
            spec,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_model_with_and_without_spec() {
        let r = RuntimeModel {
            id: "gpt-4o".into(),
            owned_by: Some("openai".into()),
            created: Some(1715367049),
        };
        let without = ResolvedModel::from_runtime(r.clone(), None);
        assert_eq!(without.id, "gpt-4o");
        assert!(without.spec.is_none());

        let spec = ModelSpec {
            limits: ModelLimits {
                context: Some(128000),
                output: Some(16384),
                ..Default::default()
            },
            capabilities: ModelCapabilities {
                tool_call: true,
                structured_output: true,
                ..Default::default()
            },
            source: CatalogueSource::Anya2a,
            ..Default::default()
        };
        let with = ResolvedModel::from_runtime(r, Some(spec));
        assert_eq!(with.spec.as_ref().unwrap().limits.context, Some(128000));
        assert!(with.spec.as_ref().unwrap().capabilities.tool_call);
    }

    #[test]
    fn model_type_aliases_loose_deserialize() {
        // anya2a uses "image-generation"; should map to ImageGen.
        let json = r#"{"type":"image-generation"}"#;
        let m: ModelSpec = serde_json::from_str(json).unwrap();
        assert_eq!(m.r#type, ModelType::ImageGen);

        // unknown type → Other
        let json = r#"{"type":"weird"}"#;
        let m: ModelSpec = serde_json::from_str(json).unwrap();
        assert_eq!(m.r#type, ModelType::Other);
    }

    #[test]
    fn empty_json_yields_defaults() {
        let m: ModelSpec = serde_json::from_str("{}").unwrap();
        assert_eq!(m.r#type, ModelType::Chat);
        assert!(!m.capabilities.tool_call);
        assert!(m.reasoning.is_none());
    }
}
