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

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DecisionOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum DecisionQuestion {
    Boolean {
        id: String,
        instructions: String,
    },
    Choice {
        id: String,
        instructions: String,
        options: Vec<DecisionOption>,
    },
    /// Ordered labels, from lowest to highest. Scores use zero-based positions.
    Score {
        id: String,
        instructions: String,
        levels: Vec<String>,
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
    pub fn instructions(&self) -> &str {
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
            if question.instructions().trim().is_empty() {
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
                    levels.iter().map(String::as_str).collect()
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
        levels: Vec<String>,
        probabilities: Option<Vec<f64>>,
        confidence: Option<f64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DecisionCapabilities {
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DecisionResponse {
    pub headers: Option<SharedHeaders>,
    pub body: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DecisionResult {
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

/// Allow independently rounded native probabilities (two decimals per item).
/// Do not normalize them: preserve the provider's numbers and raw response.
fn valid_distribution(values: &[f64]) -> bool {
    values.iter().all(|p| is_probability(*p))
        && (values.iter().sum::<f64>() - 1.0).abs() <= 0.005 * values.len() as f64 + 1e-9
}

/// If the unrounded probabilities q sum to one, their expected index t
/// satisfies sum((i - t) * q[i]) = 0. With probabilities and score independently
/// rounded to two decimals, the residual is bounded by 0.005 per term plus
/// 0.005 for the score. Centering on the supplied score also accounts for a
/// rounded distribution whose sum is slightly different from one.
fn consistent_score(score: f64, probabilities: &[f64]) -> bool {
    let residual: f64 = probabilities
        .iter()
        .enumerate()
        .map(|(index, probability)| (index as f64 - score) * probability)
        .sum();
    let tolerance = 0.005
        + 0.005
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
                                && valid_distribution(distribution)
                                && consistent_score(*expected_value, distribution)
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
    options: DecisionCallOptions,
) -> Result<DecisionResult, AiMuxError> {
    options.validate()?;
    let capabilities = model.capabilities();
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
    timeout::run(
        retries.retry(|| async {
            let result = model.do_decide(&options).await?;
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
    .await
}
