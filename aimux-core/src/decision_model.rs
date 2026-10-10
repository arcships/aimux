//! Typed finite-answer decisions. Providers perform one attempt; [`decide`]
//! owns validation, retries, cancellation and the operation deadline.

use std::collections::{BTreeMap, HashSet};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::shared::{SharedHeaders, SharedProviderMetadata, SharedProviderOptions};
use crate::{AbortSignal, AiMuxError, retry, timeout};

/// Text or JSON context shared by all questions. Images are supplied separately;
/// JSON objects are not interpreted as media parts.
pub type DecisionState = serde_json::Value;

/// An inline image shared by all questions, placed before the textual state.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DecisionImage {
    pub data: crate::shared::FileBytes,
    pub media_type: String,
    /// Provider image detail, for example OpenAI's `low`, `high`, `auto`, `original`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A native choice value, distinct from the canonical option label used as a key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum DecisionValue {
    Text(String),
    Boolean(bool),
}

/// Native question text or structured guidance, serialized without coercion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum DecisionDescription {
    Text(String),
    Object(#[ts(type = "Record<string, unknown>")] serde_json::Map<String, serde_json::Value>),
    Array(#[ts(type = "unknown[]")] Vec<serde_json::Value>),
}

impl From<String> for DecisionDescription {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for DecisionDescription {
    fn from(value: &str) -> Self {
        Self::Text(value.into())
    }
}

/// A named score level with a separate description (OpenAI's native rubric).
/// Convert into `DecisionDescription` to use it in `DecisionQuestion::Score`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct DecisionScoreLevel {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl From<DecisionScoreLevel> for DecisionDescription {
    fn from(level: DecisionScoreLevel) -> Self {
        let mut object = serde_json::Map::new();
        object.insert("label".into(), level.label.into());
        if let Some(description) = level.description {
            object.insert("description".into(), description.into());
        }
        Self::Object(object)
    }
}

impl DecisionDescription {
    fn is_empty(&self) -> bool {
        match self {
            Self::Text(text) => text.trim().is_empty(),
            Self::Object(object) => object.is_empty(),
            Self::Array(array) => array.is_empty(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct DecisionBooleanCriteria {
    #[serde(rename = "true", default, skip_serializing_if = "Option::is_none")]
    pub true_description: Option<DecisionDescription>,
    #[serde(rename = "false", default, skip_serializing_if = "Option::is_none")]
    pub false_description: Option<DecisionDescription>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DecisionOption {
    pub label: String,
    /// Native value; when omitted, the label is sent as the value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<DecisionValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<DecisionDescription>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum DecisionQuestion {
    Boolean {
        id: String,
        instructions: DecisionDescription,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<DecisionBooleanCriteria>,
    },
    Choice {
        id: String,
        instructions: DecisionDescription,
        options: Vec<DecisionOption>,
    },
    /// Ordered labels, from lowest to highest. Scores use zero-based positions.
    Score {
        id: String,
        instructions: DecisionDescription,
        levels: Vec<DecisionDescription>,
    },
}

impl DecisionQuestion {
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Boolean { id, .. } | Self::Choice { id, .. } | Self::Score { id, .. } => id,
        }
    }

    #[must_use]
    pub fn instructions(&self) -> &DecisionDescription {
        match self {
            Self::Boolean { instructions, .. }
            | Self::Choice { instructions, .. }
            | Self::Score { instructions, .. } => instructions,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DecisionCallOptions {
    #[serde(skip)]
    #[ts(skip)]
    pub recording_context: Option<crate::recording::RecordingContext>,
    pub state: DecisionState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<DecisionImage>,
    pub questions: Vec<DecisionQuestion>,
    #[serde(skip)]
    #[ts(skip)]
    pub abort_signal: Option<AbortSignal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<crate::options::TimeoutConfiguration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<SharedHeaders>,
}

impl DecisionCallOptions {
    #[must_use]
    pub fn new(state: DecisionState, questions: Vec<DecisionQuestion>) -> Self {
        Self {
            state,
            images: Vec::new(),
            questions,
            recording_context: None,
            abort_signal: None,
            max_retries: None,
            timeout: None,
            provider_options: None,
            headers: None,
        }
    }

    /// Validate provider-independent request invariants before any HTTP call.
    /// # Errors
    /// Returns `InvalidArgument` for empty questions, duplicate IDs or labels,
    /// blank instructions, empty choices, or a score with fewer than two levels.
    pub fn validate(&self) -> Result<(), AiMuxError> {
        let invalid = |message: String| AiMuxError::InvalidArgument(message);
        if self.questions.is_empty() {
            return Err(invalid("decide requires at least one question".into()));
        }
        let mut ids = HashSet::new();
        for question in &self.questions {
            let id = question.id();
            if id.trim().is_empty() || !ids.insert(id) {
                return Err(invalid(format!(
                    "empty or duplicate decision question ID: {id:?}"
                )));
            }
            if question.instructions().is_empty() {
                return Err(invalid(format!("question {id:?} requires instructions")));
            }
            let labels: Vec<&str> = match question {
                DecisionQuestion::Boolean { .. } => continue,
                DecisionQuestion::Choice { options, .. } => {
                    if options.is_empty() {
                        return Err(invalid(format!("question {id:?} requires choices")));
                    }
                    let mut values = HashSet::new();
                    if options.iter().any(|option| {
                        !values.insert(
                            option
                                .value
                                .clone()
                                .unwrap_or_else(|| DecisionValue::Text(option.label.clone())),
                        )
                    }) {
                        return Err(invalid(format!(
                            "question {id:?} has duplicate native values"
                        )));
                    }
                    options.iter().map(|option| option.label.as_str()).collect()
                }
                DecisionQuestion::Score { levels, .. } => {
                    if levels.len() < 2 {
                        return Err(invalid(format!(
                            "question {id:?} requires at least two score levels"
                        )));
                    }
                    if levels.iter().any(DecisionDescription::is_empty)
                        || levels
                            .iter()
                            .enumerate()
                            .any(|(i, level)| levels[..i].contains(level))
                    {
                        return Err(invalid(format!(
                            "question {id:?} has empty or duplicate levels"
                        )));
                    }
                    continue;
                }
            };
            let mut unique = HashSet::new();
            if labels
                .iter()
                .any(|label| label.trim().is_empty() || !unique.insert(*label))
            {
                return Err(invalid(format!(
                    "question {id:?} has empty or duplicate labels"
                )));
            }
        }
        Ok(())
    }
}

/// How the provider produced probability values. This is provenance, not a
/// calibration guarantee. Raw provider metadata retains calibration details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DecisionProbabilitySource {
    Native,
    LogitScoring,
    ModelEstimate,
}

impl std::str::FromStr for DecisionProbabilitySource {
    type Err = AiMuxError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "native" => Ok(Self::Native),
            "logit_scoring" => Ok(Self::LogitScoring),
            "model_estimate" => Ok(Self::ModelEstimate),
            _ => Err(AiMuxError::InvalidArgument(format!(
                "unknown decision probability source: {value:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum DecisionAnswer {
    /// The provider declined this question; other answers remain usable.
    Refusal,
    /// A conditional question was not evaluated. Provider details remain in raw.
    Skipped,
    /// The provider abstained rather than selecting an answer.
    Abstention,
    /// P(true); callers choose their own thresholds.
    Boolean { probability_true: f64 },
    Choice {
        selected: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<DecisionValue>,
        probabilities: Option<BTreeMap<String, f64>>,
        confidence: Option<f64>,
    },
    Score {
        expected_value: f64,
        levels: Vec<DecisionDescription>,
        probabilities: Option<Vec<f64>>,
        confidence: Option<f64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DecisionCapabilities {
    #[serde(default)]
    pub supports_images: bool,
    #[serde(default)]
    pub max_images: Option<usize>,
    #[serde(default)]
    pub supports_typed_choices: bool,
    #[serde(default)]
    pub rounding: DecisionRounding,
    pub probability_source: DecisionProbabilitySource,
    pub supports_boolean: bool,
    pub supports_choice: bool,
    pub supports_score: bool,
    pub returns_distributions: bool,
    pub max_questions: Option<usize>,
    pub min_choices: Option<usize>,
    pub max_choices: Option<usize>,
    pub max_score_levels: Option<usize>,
}

/// Declared decimal rounding precision. None means only floating-point noise
/// is tolerated, not decimal rounding. Providers must report their actual rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DecisionRounding {
    pub probability_decimals: Option<u8>,
    pub score_decimals: Option<u8>,
}

impl DecisionRounding {
    fn error(decimals: Option<u8>) -> f64 {
        decimals.map_or(0.0, |digits| 0.5 * 10_f64.powi(-i32::from(digits)))
    }

    fn is_valid(self) -> bool {
        [self.probability_decimals, self.score_decimals]
            .into_iter()
            .flatten()
            .all(|digits| digits <= 15)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DecisionResponse {
    pub headers: Option<SharedHeaders>,
    pub body: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DecisionResult {
    #[serde(default)]
    pub rounding: DecisionRounding,
    pub answers: BTreeMap<String, DecisionAnswer>,
    pub provider: String,
    pub model: String,
    pub model_version: Option<String>,
    pub probability_source: DecisionProbabilitySource,
    pub usage: Option<crate::types::Usage>,
    pub latency_ms: Option<f64>,
    pub provider_metadata: Option<SharedProviderMetadata>,
    pub response: Option<DecisionResponse>,
}

fn is_probability(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

/// Allow independent rounding at the provider-declared precision and f32
/// softmax noise. Many runtimes return f32 probabilities as JSON f64 values.
/// Do not normalize them: preserve the provider's numbers and raw response.
fn valid_distribution(values: &[f64], rounding: DecisionRounding) -> bool {
    values.iter().all(|p| is_probability(*p))
        && (values.iter().sum::<f64>() - 1.0).abs()
            <= DecisionRounding::error(rounding.probability_decimals) * values.len() as f64
                + f64::from(f32::EPSILON) * 2.0
}

/// If the unrounded probabilities q sum to one, their expected index t
/// satisfies sum((i - t) * q[i]) = 0. With probabilities and score independently
/// rounded at their declared precisions, the residual is bounded by the
/// corresponding per-term and score errors. Centering on the score also accounts for a
/// rounded distribution whose sum is slightly different from one.
fn consistent_score(score: f64, probabilities: &[f64], rounding: DecisionRounding) -> bool {
    let residual: f64 = probabilities
        .iter()
        .enumerate()
        .map(|(index, probability)| (index as f64 - score) * probability)
        .sum();
    let tolerance = DecisionRounding::error(rounding.score_decimals)
        + DecisionRounding::error(rounding.probability_decimals)
            * (0..probabilities.len())
                .map(|index| (index as f64 - score).abs())
                .sum::<f64>();
    residual.abs() <= tolerance + f64::from(f32::EPSILON) * 2.0 * probabilities.len() as f64
}

impl DecisionResult {
    /// Reject partial, mistyped, out-of-range and mismatched answers.
    /// # Errors
    /// Returns `InvalidResponseData` when the response violates the request.
    pub fn validate(&self, request: &DecisionCallOptions) -> Result<(), AiMuxError> {
        if !self.rounding.is_valid() {
            return Err(AiMuxError::InvalidResponseData(
                "invalid decision rounding precision".into(),
            ));
        }
        let invalid = |id: &str| {
            AiMuxError::InvalidResponseData(format!("invalid decision answer for {id:?}"))
        };
        if self.answers.len() != request.questions.len() {
            return Err(AiMuxError::InvalidResponseData(
                "decision answers do not match question IDs".into(),
            ));
        }
        for question in &request.questions {
            let id = question.id();
            let answer = self.answers.get(id).ok_or_else(|| invalid(id))?;
            let valid = match (question, answer) {
                (
                    _,
                    DecisionAnswer::Refusal | DecisionAnswer::Skipped | DecisionAnswer::Abstention,
                ) => true,
                (
                    DecisionQuestion::Boolean { .. },
                    DecisionAnswer::Boolean { probability_true },
                ) => is_probability(*probability_true),
                (
                    DecisionQuestion::Choice { options, .. },
                    DecisionAnswer::Choice {
                        selected,
                        value,
                        probabilities,
                        confidence,
                    },
                ) => {
                    options.iter().any(|option| {
                        &option.label == selected
                            && match (&option.value, value) {
                                (Some(expected), Some(actual)) => expected == actual,
                                (None, None) => true,
                                (None, Some(DecisionValue::Text(actual))) => actual == selected,
                                _ => false,
                            }
                    }) && confidence.is_none_or(is_probability)
                        && probabilities.as_ref().is_none_or(|distribution| {
                            distribution.len() == options.len()
                                && options
                                    .iter()
                                    .all(|option| distribution.contains_key(&option.label))
                                && valid_distribution(
                                    &distribution.values().copied().collect::<Vec<_>>(),
                                    self.rounding,
                                )
                        })
                }
                (
                    DecisionQuestion::Score { levels, .. },
                    DecisionAnswer::Score {
                        expected_value,
                        levels: actual_levels,
                        probabilities,
                        confidence,
                    },
                ) => {
                    levels.len() >= 2
                        && levels == actual_levels
                        && expected_value.is_finite()
                        && (0.0..=(levels.len() - 1) as f64).contains(expected_value)
                        && confidence.is_none_or(is_probability)
                        && probabilities.as_ref().is_none_or(|distribution| {
                            distribution.len() == levels.len()
                                && valid_distribution(distribution, self.rounding)
                                && consistent_score(*expected_value, distribution, self.rounding)
                        })
                }
                _ => false,
            };
            if !valid {
                return Err(invalid(id));
            }
        }
        Ok(())
    }
}

#[async_trait]
pub trait DecisionModel: Send + Sync {
    fn specification_version(&self) -> &'static str {
        "v4"
    }
    fn provider(&self) -> &str;
    fn model_id(&self) -> &str;
    fn capabilities(&self) -> DecisionCapabilities;
    fn config_snapshot(&self) -> crate::recording::ProviderRecord {
        let mut snapshot =
            crate::recording::ProviderRecord::minimal(self.provider(), self.model_id());
        snapshot.profile = Some(serde_json::json!({"decision_capabilities": self.capabilities()}));
        snapshot
    }
    fn retry_config(&self) -> crate::retry::RetryConfig {
        crate::retry::RetryConfig::default()
    }
    /// Perform one provider attempt.
    /// # Errors
    /// Returns provider, transport or invalid response errors.
    async fn do_decide(&self, options: &DecisionCallOptions) -> Result<DecisionResult, AiMuxError>;
}

/// Make one typed decision with Core-owned retries and timeout.
/// # Errors
/// Returns invalid request, unsupported capability, provider, invalid response,
/// retry exhaustion, timeout or caller abort errors.
pub async fn decide(
    model: &dyn DecisionModel,
    mut options: DecisionCallOptions,
) -> Result<DecisionResult, AiMuxError> {
    options.validate()?;
    let capabilities = model.capabilities();
    if !options.images.is_empty() && !capabilities.supports_images {
        return Err(AiMuxError::UnsupportedFunctionality(format!(
            "{} decisions do not support images",
            model.provider()
        )));
    }
    if capabilities
        .max_images
        .is_some_and(|max| options.images.len() > max)
    {
        return Err(AiMuxError::InvalidArgument(
            "too many decision images".into(),
        ));
    }
    if !capabilities.supports_typed_choices && options.questions.iter().any(|q| {
        matches!(q, DecisionQuestion::Choice { options, .. } if options.iter().any(|o| o.value.is_some()))
    }) {
        return Err(AiMuxError::UnsupportedFunctionality(format!(
            "{} decisions do not support native typed choice values", model.provider()
        )));
    }
    if !capabilities.rounding.is_valid() {
        return Err(AiMuxError::InvalidArgument(
            "invalid provider rounding precision".into(),
        ));
    }
    if capabilities
        .max_questions
        .is_some_and(|max| options.questions.len() > max)
    {
        return Err(AiMuxError::InvalidArgument(
            "too many decision questions for this provider".into(),
        ));
    }
    for question in &options.questions {
        let (supported, valid_size) = match question {
            DecisionQuestion::Boolean { .. } => (capabilities.supports_boolean, true),
            DecisionQuestion::Choice { options, .. } => (
                capabilities.supports_choice,
                capabilities
                    .min_choices
                    .is_none_or(|min| options.len() >= min)
                    && capabilities
                        .max_choices
                        .is_none_or(|max| options.len() <= max),
            ),
            DecisionQuestion::Score { levels, .. } => (
                capabilities.supports_score,
                capabilities
                    .max_score_levels
                    .is_none_or(|max| levels.len() <= max),
            ),
        };
        if !supported {
            return Err(AiMuxError::UnsupportedFunctionality(format!(
                "decision question type unsupported by {}",
                model.provider()
            )));
        }
        if !valid_size {
            return Err(AiMuxError::InvalidArgument(format!(
                "question {:?} exceeds provider criteria limits",
                question.id()
            )));
        }
    }
    let timeout = timeout::OperationTimeout::new(options.timeout.unwrap_or_default())?;
    let abort_signal = options.abort_signal.clone();
    let retries = retry::prepare_retries(
        options.max_retries,
        model.retry_config(),
        abort_signal.clone(),
    );
    let context = crate::recording::recorder().map(|recorder| {
        let ctx =
            crate::recording::RecordingContext::new(crate::recording::new_call_id(), recorder);
        options.recording_context = Some(ctx.clone());
        ctx.recorder.record_decision_input(
            &ctx.call_id,
            &options,
            model.provider(),
            model.model_id(),
        );
        ctx.recorder
            .record_provider(&ctx.call_id, &model.config_snapshot());
        ctx
    });
    let result = timeout::run(
        retries.retry(|| async {
            if let Some(ctx) = &options.recording_context {
                let _ = ctx.start_attempt();
            }
            let result = model.do_decide(&options).await?;
            if result.rounding != capabilities.rounding {
                return Err(AiMuxError::InvalidResponseData(
                    "decision rounding does not match provider capabilities".into(),
                ));
            }
            result.validate(&options)?;
            if capabilities.returns_distributions
                && result.answers.values().any(|answer| {
                    matches!(
                        answer,
                        DecisionAnswer::Choice {
                            probabilities: None,
                            ..
                        } | DecisionAnswer::Score {
                            probabilities: None,
                            ..
                        }
                    )
                })
            {
                return Err(AiMuxError::InvalidResponseData(
                    "decision provider omitted promised distributions".into(),
                ));
            }
            Ok(result)
        }),
        abort_signal.as_ref(),
        timeout,
    )
    .await;
    if let Some(ctx) = context {
        let outcome = match &result {
            Ok(result) => crate::recording::OutcomeRecord {
                status: crate::recording::OutcomeStatus::Success,
                usage: serde_json::to_value(&result.usage).ok(),
                decision_result: serde_json::to_value(result)
                    .ok()
                    .map(crate::recording::redact_json),
                ..Default::default()
            },
            Err(error) => crate::recording::OutcomeRecord::from_error(error),
        };
        ctx.recorder.record_transport_closed(&ctx.call_id);
        ctx.recorder.record_outcome(&ctx.call_id, &outcome);
    }
    result
}
