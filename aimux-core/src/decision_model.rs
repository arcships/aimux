//! Typed finite-answer decisions. Providers perform one attempt; [`decide`]
//! owns validation, retries, cancellation and the operation deadline.

use std::collections::{BTreeMap, HashSet};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::shared::{SharedHeaders, SharedProviderMetadata, SharedProviderOptions};
use crate::{AbortSignal, AiMuxError, retry, timeout};

/// Text or JSON context shared by all questions. Media requires a future
/// explicit input contract; JSON objects are not interpreted as media parts.
pub type DecisionState = serde_json::Value;

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
    /// P(true); callers choose their own thresholds.
    Boolean { probability_true: f64 },
    Choice {
        selected: String,
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

/// Allow independent rounding at the provider-declared precision.
/// Do not normalize them: preserve the provider's numbers and raw response.
fn valid_distribution(values: &[f64], rounding: DecisionRounding) -> bool {
    values.iter().all(|p| is_probability(*p))
        && (values.iter().sum::<f64>() - 1.0).abs()
            <= DecisionRounding::error(rounding.probability_decimals) * values.len() as f64 + 1e-9
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
    residual.abs() <= tolerance + 1e-9
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
                    DecisionQuestion::Boolean { .. },
                    DecisionAnswer::Boolean { probability_true },
                ) => is_probability(*probability_true),
                (
                    DecisionQuestion::Choice { options, .. },
                    DecisionAnswer::Choice {
                        selected,
                        probabilities,
                        confidence,
                    },
                ) => {
                    options.iter().any(|option| &option.label == selected)
                        && confidence.is_none_or(is_probability)
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
        crate::recording::ProviderRecord::from_model(self.provider(), self.model_id())
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
    let retries = retry::prepare_retries(options.max_retries, abort_signal.clone());
    let context = crate::recording::recorder().map(|recorder| {
        let ctx =
            crate::recording::RecordingContext::new(crate::recording::new_call_id(), recorder);
        options.recording_context = Some(ctx.clone());
        ctx.recorder.record_decision_input(
            &ctx.call_id,
            &options,
            model.provider(),
            model.model_id(),
            &capabilities,
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
