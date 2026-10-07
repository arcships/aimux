//! The `EvaluationModel` trait — the provider-facing interface for the
//! experimental Choice / Score / Boolean evaluation.
//!
//! Aligned with Vercel AI SDK `EvaluationModelV4`
//! (`provider/src/evaluation-model/v4/`).

use std::collections::HashMap;

use async_trait::async_trait;
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::AbortSignal;
use crate::error::AiMuxError;
use crate::shared::{
    JsonObject, ResponseInfo, SharedHeaders, SharedProviderMetadata, SharedProviderOptions, Warning,
};

/// Shared state or structured instructions for an evaluation
/// (`EvaluationModelV4Input`: a string, a JSON object or a JSON array).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum EvaluationInput {
    Text(String),
    #[ts(type = "Record<string, unknown>")]
    Object(JsonObject),
    #[ts(type = "Array<unknown>")]
    Array(Vec<Value>),
}

/// The kind of a question (`EvaluationModelV4Question['type']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum EvaluationQuestionType {
    Choice,
    Score,
    Boolean,
}

impl EvaluationQuestionType {
    /// The wire name (`"choice"`, `"score"`, `"boolean"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::Score => "score",
            Self::Boolean => "boolean",
        }
    }
}

/// The optional descriptions of a Boolean question's outcomes. An absent key
/// and an explicit `null` differ: `Some(None)` is `null` (no description).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BooleanCriteria {
    #[serde(
        rename = "true",
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(optional, type = "EvaluationInput | null")]
    pub true_: Option<Option<EvaluationInput>>,
    #[serde(
        rename = "false",
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(optional, type = "EvaluationInput | null")]
    pub false_: Option<Option<EvaluationInput>>,
}

fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// A judgment to make about the shared state (`EvaluationModelV4Question`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum EvaluationQuestion {
    Choice {
        instructions: EvaluationInput,
        /// Nonempty map of option names to descriptions; `None` means no
        /// description. Insertion order is the option order.
        #[ts(type = "Record<string, EvaluationInput | null>")]
        criteria: IndexMap<String, Option<EvaluationInput>>,
    },
    Score {
        instructions: EvaluationInput,
        /// At least two ordered levels, indexed from zero.
        criteria: Vec<Option<EvaluationInput>>,
    },
    Boolean {
        instructions: EvaluationInput,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<BooleanCriteria>,
    },
}

impl EvaluationQuestion {
    /// The kind of this question.
    #[must_use]
    pub fn question_type(&self) -> EvaluationQuestionType {
        match self {
            Self::Choice { .. } => EvaluationQuestionType::Choice,
            Self::Score { .. } => EvaluationQuestionType::Score,
            Self::Boolean { .. } => EvaluationQuestionType::Boolean,
        }
    }
}

/// Options passed to [`EvaluationModel::do_evaluate`]
/// (`EvaluationModelV4CallOptions`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EvaluationCallOptions {
    /// One shared state, even when the value is an array.
    pub state: EvaluationInput,
    /// Questions by id; insertion order is the question order.
    #[ts(type = "Record<string, EvaluationQuestion>")]
    pub questions: IndexMap<String, EvaluationQuestion>,
    /// Abort signal for cancelling the evaluation.
    #[serde(skip)]
    #[ts(skip)]
    pub abort_signal: Option<AbortSignal>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<SharedHeaders>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

/// The answer to one question (`EvaluationModelV4Answer`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum EvaluationAnswer {
    Choice {
        /// The selected option, with maximal probability when a distribution
        /// exists.
        choice: String,
        /// Complete distribution over the question's options, when available.
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        probabilities: Option<HashMap<String, f64>>,
    },
    Score {
        /// Fractional position in `[0, levels - 1]`.
        score: f64,
        /// Complete distribution keyed by zero-based level indices as
        /// strings; when supplied, `score` is its probability-weighted mean.
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        probabilities: Option<HashMap<String, f64>>,
    },
    Boolean {
        /// Model-estimated P(true), in `[0, 1]`. Not confidence in either
        /// outcome.
        probability: f64,
    },
}

/// Decimal places the provider rounded its output to; omit for full precision.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EvaluationRounding {
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probability_decimals: Option<u32>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_decimals: Option<u32>,
}

/// Token usage of an evaluation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EvaluationUsage {
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
}

/// The result of [`EvaluationModel::do_evaluate`] (`EvaluationModelV4Result`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EvaluationResult {
    /// Exactly one answer per question, under the original question ids.
    #[ts(type = "Record<string, EvaluationAnswer>")]
    pub answers: IndexMap<String, EvaluationAnswer>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rounding: Option<EvaluationRounding>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<EvaluationUsage>,
    pub warnings: Vec<Warning>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<SharedProviderMetadata>,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<ResponseInfo>,
}

/// The experimental evaluation model contract. May change in patch releases.
#[async_trait]
pub trait EvaluationModel: Send + Sync {
    /// Provider name, e.g. `"openai.evaluation"`.
    fn provider(&self) -> &str;

    /// Provider-specific model ID.
    fn model_id(&self) -> &str;

    /// Supported question types, used to reject unsupported calls before any
    /// I/O.
    fn supported_question_types(&self) -> &[EvaluationQuestionType];

    /// Evaluate every question against the same state. No partial results.
    async fn do_evaluate(
        &self,
        options: &EvaluationCallOptions,
    ) -> Result<EvaluationResult, AiMuxError>;
}
