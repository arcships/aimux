//! Typed decisions: Choice, Score and Boolean answers against one shared
//! state.
//!
//! [`DecisionModel`] is the provider contract, the AI SDK's
//! `DecisionModelV4` (`@ai-sdk/provider@4.0.22 src/decision-model/v4/`).
//! [`decide`] is the operation (`ai@7.0.128 src/decide/decide.ts`): it
//! validates the input, rejects unsupported question types before any I/O,
//! retries the model call, and validates the answers after the retries.
//!
//! Retries, the timeout and recording are operation settings
//! ([`DecideOptions`]); the model call runs inside
//! [`OperationSettings::scope`](crate::operation::OperationSettings::scope),
//! as `generate_text`'s does. What a provider needs beyond the V4 call options
//! (images, typed choice values, runtime protocols) goes in its
//! `provider_options`; what it reports beyond the V4 result (probability
//! source, confidence) goes in its `provider_metadata`.

use async_trait::async_trait;
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::error::AiMuxError;
use crate::shared::{
    JsonObject, ResponseInfo, SharedHeaders, SharedProviderMetadata, SharedProviderOptions, Warning,
};
use crate::{AbortSignal, retry, timeout};

/// Shared state or structured instructions for a decision
/// (`DecisionModelV4Input`: a string, a JSON object or a JSON array).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(extend("x-aimux-codec" = "ordered-untagged"))]
pub enum DecisionInput {
    Text(String),
    Object(JsonObject),
    Array(Vec<Value>),
}

impl From<&str> for DecisionInput {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for DecisionInput {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

/// The kind of a question (`DecisionModelV4Question['type']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DecisionQuestionType {
    Choice,
    Score,
    Boolean,
}

impl DecisionQuestionType {
    /// Every question type, for models that support them all.
    pub const ALL: &'static [Self] = &[Self::Choice, Self::Score, Self::Boolean];

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
/// Other keys are rejected: Boolean criteria may only describe `true` and
/// `false` (`ai@7.0.128 src/decide/validate-decision.ts:94-105`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionBooleanCriteria {
    #[serde(
        rename = "true",
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<Option<DecisionInput>>", extend("x-aimux-codec" = "presence"))]
    pub true_: Option<Option<DecisionInput>>,
    #[serde(
        rename = "false",
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<Option<DecisionInput>>", extend("x-aimux-codec" = "presence"))]
    pub false_: Option<Option<DecisionInput>>,
}

fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// A judgment to make about the shared state (`DecisionModelV4Question`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DecisionQuestion {
    Choice {
        instructions: DecisionInput,
        /// Nonempty map of option names to descriptions; `None` means no
        /// description. Insertion order is the option order.
        criteria: IndexMap<String, Option<DecisionInput>>,
    },
    Score {
        instructions: DecisionInput,
        /// At least two ordered levels, indexed from zero.
        criteria: Vec<Option<DecisionInput>>,
    },
    Boolean {
        instructions: DecisionInput,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<DecisionBooleanCriteria>,
    },
}

impl DecisionQuestion {
    /// The kind of this question.
    #[must_use]
    pub fn question_type(&self) -> DecisionQuestionType {
        match self {
            Self::Choice { .. } => DecisionQuestionType::Choice,
            Self::Score { .. } => DecisionQuestionType::Score,
            Self::Boolean { .. } => DecisionQuestionType::Boolean,
        }
    }

    /// The question's instructions.
    #[must_use]
    pub fn instructions(&self) -> &DecisionInput {
        match self {
            Self::Choice { instructions, .. }
            | Self::Score { instructions, .. }
            | Self::Boolean { instructions, .. } => instructions,
        }
    }
}

/// Questions by id; insertion order is the question order.
pub type DecisionQuestions = IndexMap<String, DecisionQuestion>;

/// Options passed to [`DecisionModel::do_decide`]
/// (`DecisionModelV4CallOptions`,
/// `@ai-sdk/provider@4.0.22 src/decision-model/v4/decision-model-v4-call-options.ts:7-14`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionCallOptions {
    /// One shared state, even when the value is an array.
    pub state: DecisionInput,
    pub questions: DecisionQuestions,
    /// Abort signal for cancelling the call.
    #[serde(skip)]
    pub abort_signal: Option<AbortSignal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<SharedHeaders>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

impl DecisionCallOptions {
    /// Call options with only a state and questions.
    #[must_use]
    pub fn new(state: impl Into<DecisionInput>, questions: DecisionQuestions) -> Self {
        Self {
            state: state.into(),
            questions,
            abort_signal: None,
            headers: None,
            provider_options: None,
        }
    }
}

/// The answer to one question (`DecisionModelV4Answer`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DecisionAnswer {
    Choice {
        /// The selected option, with maximal probability when a distribution
        /// exists.
        choice: String,
        /// Complete distribution over the question's options, when available.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        probabilities: Option<IndexMap<String, f64>>,
    },
    Score {
        /// Fractional position in `[0, levels - 1]`.
        score: f64,
        /// Complete distribution keyed by zero-based level indices as
        /// strings; when supplied, `score` is its probability-weighted mean.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        probabilities: Option<IndexMap<String, f64>>,
    },
    Boolean {
        /// Model-estimated P(true), in `[0, 1]`. Not confidence in either
        /// outcome.
        probability: f64,
    },
}

/// Decimal places the provider rounded its output to; omit for full
/// precision. Validation allows half a unit in the last place per rounded
/// value (`decision-model-v4-result.ts:39-42`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRounding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probability_decimals: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_decimals: Option<u32>,
}

/// Token usage reported by a decision model
/// (`decision-model-v4-result.ts:43-46`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
}

/// The result of [`DecisionModel::do_decide`] (`DecisionModelV4Result`,
/// `@ai-sdk/provider@4.0.22 src/decision-model/v4/decision-model-v4-result.ts:31-56`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionResult {
    /// Exactly one answer per question, under the original question ids.
    pub answers: IndexMap<String, DecisionAnswer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rounding: Option<DecisionRounding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<DecisionUsage>,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<SharedProviderMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<ResponseInfo>,
}

/// The experimental decision model contract
/// (`@ai-sdk/provider@4.0.22 src/decision-model/v4/decision-model-v4.ts:6-16`).
#[async_trait]
pub trait DecisionModel: Send + Sync {
    /// Specification version (always `"v4"`).
    fn specification_version(&self) -> &'static str {
        "v4"
    }

    /// Provider name, e.g. `"jev"` or `"google.evaluation"`.
    fn provider(&self) -> &str;

    /// Provider-specific model ID.
    fn model_id(&self) -> &str;

    /// Supported question types, used to reject unsupported calls before any
    /// I/O.
    fn supported_question_types(&self) -> &[DecisionQuestionType];

    /// Decide every answer against the same state. No partial results.
    ///
    /// # Errors
    ///
    /// Provider, transport and invalid response errors.
    async fn do_decide(&self, options: &DecisionCallOptions) -> Result<DecisionResult, AiMuxError>;
}

/// A model's `DecisionModelV4` properties as JSON, as the AI SDK names them
/// (`decision-model-v4.ts:7-11`): `{specificationVersion, provider, modelId,
/// supportedQuestionTypes}`. The bindings' `describe` returns this.
#[must_use]
pub fn decision_model_properties(model: &dyn DecisionModel) -> Value {
    serde_json::json!({
        "specificationVersion": model.specification_version(),
        "provider": model.provider(),
        "modelId": model.model_id(),
        "supportedQuestionTypes": model.supported_question_types(),
    })
}

/// The AI SDK's `DecisionUnsupportedQuestionTypeError`
/// (`@ai-sdk/provider@4.0.22 src/errors/decision-unsupported-question-type-error.ts:10-37`),
/// with its message. The error taxonomy has no variant for it (one would
/// ripple into the C ABI), so it is reported as unsupported functionality.
#[must_use]
pub fn unsupported_question_type_error(
    question_id: &str,
    question_type: DecisionQuestionType,
    provider: &str,
    model_id: &str,
) -> AiMuxError {
    AiMuxError::UnsupportedFunctionality(format!(
        "Question \"{question_id}\" has type \"{}\", which is not supported by provider \"{provider}\" and model \"{model_id}\".",
        question_type.as_str()
    ))
}

/// Options of [`decide`]: the call options plus the operation settings
/// (`ai@7.0.128 src/decide/decide.ts:53-61`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecideOptions {
    pub state: DecisionInput,
    pub questions: DecisionQuestions,
    /// Maximum retries for transient provider failures. Defaults to 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    /// Operation deadline (aimux extension).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<crate::options::TimeoutConfiguration>,
    #[serde(skip)]
    pub abort_signal: Option<AbortSignal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<SharedHeaders>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

impl DecideOptions {
    /// Options with only a state and questions.
    #[must_use]
    pub fn new(state: impl Into<DecisionInput>, questions: DecisionQuestions) -> Self {
        Self {
            state: state.into(),
            questions,
            max_retries: None,
            timeout: None,
            abort_signal: None,
            headers: None,
            provider_options: None,
        }
    }
}

/// Token usage of a decision, with the total when both counts are known
/// (`ai@7.0.128 src/decide/decision-result.ts:27-31`): an unknown count is
/// `undefined` there, so it is omitted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecideUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u32>,
}

/// The result of [`decide`] (`DecisionResult` of
/// `ai@7.0.128 src/decide/decision-result.ts:22-39`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecideResult {
    pub answers: IndexMap<String, DecisionAnswer>,
    pub usage: DecideUsage,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rounding: Option<DecisionRounding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<SharedProviderMetadata>,
    /// The model's response information; the timestamp defaults to the end of
    /// the call and the model id to the model's.
    pub response: ResponseInfo,
}

fn invalid_input(parameter: &str, message: &str) -> AiMuxError {
    AiMuxError::InvalidArgument(format!(
        "Invalid argument for parameter {parameter}: {message}"
    ))
}

/// Reject a request no model can answer
/// (`ai@7.0.128 src/decide/validate-decision.ts:50-125`). The state,
/// instructions and descriptions are JSON inputs by type, and Boolean
/// criteria cannot carry other keys; what remains is the question map and
/// the criteria sizes.
///
/// # Errors
///
/// `InvalidArgument` with the upstream message.
pub fn validate_decision_input(questions: &DecisionQuestions) -> Result<(), AiMuxError> {
    if questions.is_empty() {
        return Err(invalid_input(
            "questions",
            "must be a nonempty question map",
        ));
    }
    for (id, question) in questions {
        let parameter = format!("questions.{id}");
        match question {
            DecisionQuestion::Choice { criteria, .. } if criteria.is_empty() => {
                return Err(invalid_input(
                    &parameter,
                    "choice criteria must be a nonempty option map",
                ));
            }
            DecisionQuestion::Score { criteria, .. } if criteria.len() < 2 => {
                return Err(invalid_input(
                    &parameter,
                    "score criteria must contain at least two ordered levels",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Absolute tolerance for sums and means; provider output is never
/// renormalized (`ai@7.0.128 src/decide/validate-decision.ts:8-9`).
const TOLERANCE: f64 = 1e-6;

fn invalid_answer(message: String) -> AiMuxError {
    AiMuxError::InvalidResponseData(message)
}

fn is_probability(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn validate_distribution(
    distribution: &IndexMap<String, f64>,
    keys: &[String],
    id: &str,
    rounding_error: f64,
) -> Result<(), AiMuxError> {
    if distribution.len() != keys.len()
        || !keys.iter().all(|key| distribution.contains_key(key))
        || !distribution.values().all(|p| is_probability(*p))
    {
        return Err(invalid_answer(format!(
            "Question \"{id}\" must have a complete distribution of finite probabilities in [0, 1]."
        )));
    }
    let sum: f64 = distribution.values().sum();
    if (sum - 1.0).abs() > TOLERANCE + keys.len() as f64 * rounding_error {
        return Err(invalid_answer(format!(
            "Question \"{id}\" probabilities must sum to 1 within the declared rounding precision."
        )));
    }
    Ok(())
}

fn rounding_error(decimals: Option<u32>) -> Result<f64, AiMuxError> {
    match decimals {
        None => Ok(0.0),
        Some(decimals @ 0..=15) => Ok(0.5 * 10_f64.powi(-(decimals as i32))),
        Some(_) => Err(invalid_answer(
            "Decision rounding decimals must be integers between 0 and 15.".into(),
        )),
    }
}

/// Check the answers against the questions
/// (`ai@7.0.128 src/decide/validate-decision.ts:176-298`): exactly one
/// answer of the right type per question, choices among the options and of
/// maximal probability, scores within the rubric and equal to the
/// distribution's mean, probabilities in `[0, 1]` and summing to 1, each
/// within the declared rounding.
///
/// # Errors
///
/// `InvalidResponseData` with the upstream message.
pub fn validate_decision_answers(
    questions: &DecisionQuestions,
    answers: &IndexMap<String, DecisionAnswer>,
    rounding: Option<DecisionRounding>,
) -> Result<(), AiMuxError> {
    let rounding = rounding.unwrap_or_default();
    let probability_error = rounding_error(rounding.probability_decimals)?;
    let score_error = rounding_error(rounding.score_decimals)?;
    if answers.len() != questions.len() || !questions.keys().all(|id| answers.contains_key(id)) {
        return Err(invalid_answer(
            "Decision must return exactly one answer for every question.".into(),
        ));
    }
    for (id, question) in questions {
        match (question, &answers[id]) {
            (
                DecisionQuestion::Choice { criteria, .. },
                DecisionAnswer::Choice {
                    choice,
                    probabilities,
                },
            ) => {
                if !criteria.contains_key(choice) {
                    return Err(invalid_answer(format!(
                        "Question \"{id}\" selected an unknown option."
                    )));
                }
                if let Some(probabilities) = probabilities {
                    let keys: Vec<String> = criteria.keys().cloned().collect();
                    validate_distribution(probabilities, &keys, id, probability_error)?;
                    let selected = probabilities[choice];
                    if probabilities.values().any(|p| *p > selected + TOLERANCE) {
                        return Err(invalid_answer(format!(
                            "Question \"{id}\" did not select a highest-probability option."
                        )));
                    }
                }
            }
            (
                DecisionQuestion::Score { criteria, .. },
                DecisionAnswer::Score {
                    score,
                    probabilities,
                },
            ) => {
                let max = criteria.len() - 1;
                if !score.is_finite() || *score < 0.0 || *score > max as f64 {
                    return Err(invalid_answer(format!(
                        "Question \"{id}\" score must be in [0, {max}]."
                    )));
                }
                if let Some(probabilities) = probabilities {
                    let keys: Vec<String> = (0..criteria.len()).map(|i| i.to_string()).collect();
                    validate_distribution(probabilities, &keys, id, probability_error)?;
                    let mean: f64 = keys
                        .iter()
                        .enumerate()
                        .map(|(index, key)| index as f64 * probabilities[key])
                        .sum();
                    let mean_rounding_error: f64 = (0..criteria.len())
                        .map(|index| index as f64 * probability_error)
                        .sum();
                    if (mean - score).abs() > TOLERANCE + mean_rounding_error + score_error {
                        return Err(invalid_answer(format!(
                            "Question \"{id}\" score must equal the probability-weighted mean within the declared rounding precision."
                        )));
                    }
                }
            }
            (DecisionQuestion::Boolean { .. }, DecisionAnswer::Boolean { probability }) => {
                if !is_probability(*probability) {
                    return Err(invalid_answer(format!(
                        "Question \"{id}\" must return P(true) as a finite probability in [0, 1]."
                    )));
                }
            }
            _ => {
                return Err(invalid_answer(format!(
                    "Question \"{id}\" returned an answer with the wrong type."
                )));
            }
        }
    }
    Ok(())
}

fn check_abort(signal: Option<&AbortSignal>) -> Result<(), AiMuxError> {
    match signal {
        Some(signal) if signal.is_aborted() => Err(AiMuxError::from_abort_signal(signal)),
        _ => Ok(()),
    }
}

/// Decide answers to typed questions against one shared state
/// (`ai@7.0.128 src/decide/decide.ts:36-200`).
///
/// In order: the input is validated; a question whose type the model does
/// not support is rejected before any I/O; the model is called with retries
/// (`max_retries`, default 2) within the timeout; the answers are validated
/// after the retries, so invalid answers are not retried. The response
/// timestamp defaults to now and its model id to the model's. With a
/// recorder installed, the call is recorded with its options and result.
///
/// The operation's telemetry callbacks and tracing are not part of this
/// port.
///
/// # Errors
///
/// `InvalidArgument` for an invalid request, `UnsupportedFunctionality` for
/// an unsupported question type, provider and transport errors, retry
/// exhaustion, timeout, abort, and `InvalidResponseData` for invalid answers.
pub async fn decide(
    model: &dyn DecisionModel,
    options: DecideOptions,
) -> Result<DecideResult, AiMuxError> {
    validate_decision_input(&options.questions)?;
    for (id, question) in &options.questions {
        let question_type = question.question_type();
        if !model.supported_question_types().contains(&question_type) {
            return Err(unsupported_question_type_error(
                id,
                question_type,
                model.provider(),
                model.model_id(),
            ));
        }
    }
    // The operation settings a model below the decision reads, as in
    // `generate_text` (a language model under `DecisionLanguageModel` may be
    // a router whose children take the retry count from them).
    let settings = crate::operation::OperationSettings {
        max_retries: options.max_retries,
        timeout: options.timeout,
        session_id: None,
    };
    let timeout = timeout::OperationTimeout::new(options.timeout.unwrap_or_default())?;
    let abort_signal = options.abort_signal.clone();
    let retries = retry::prepare_retries(options.max_retries, abort_signal.clone());
    let context = crate::recording::recorder().map(|recorder| {
        let ctx =
            crate::recording::RecordingContext::new(crate::recording::new_call_id(), recorder);
        ctx.recorder.record_decision_input(
            &ctx.call_id,
            &options,
            model.provider(),
            model.model_id(),
        );
        ctx.recorder.record_provider(
            &ctx.call_id,
            &crate::recording::ProviderRecord::from_model(model.provider(), model.model_id()),
        );
        ctx
    });
    let call = DecisionCallOptions {
        state: options.state,
        questions: options.questions,
        abort_signal: abort_signal.clone(),
        headers: options.headers,
        provider_options: options.provider_options,
    };
    let result = settings
        .scope(timeout::run(
            crate::recording::RecordingContext::scope(
                context.clone(),
                retries.retry(|| async {
                    check_abort(abort_signal.as_ref())?;
                    if let Some(ctx) = &context {
                        let _ = ctx.start_attempt();
                    }
                    model.do_decide(&call).await
                }),
            ),
            abort_signal.as_ref(),
            timeout,
        ))
        .await
        .and_then(|result| {
            check_abort(abort_signal.as_ref())?;
            validate_decision_answers(&call.questions, &result.answers, result.rounding)?;
            let usage = result.usage.unwrap_or_default();
            let mut response = result.response.unwrap_or_default();
            response
                .timestamp
                .get_or_insert_with(crate::timestamp::Timestamp::now);
            response
                .model_id
                .get_or_insert_with(|| model.model_id().to_owned());
            Ok(DecideResult {
                answers: result.answers,
                usage: DecideUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    total_tokens: usage
                        .input_tokens
                        .zip(usage.output_tokens)
                        .and_then(|(input, output)| input.checked_add(output)),
                },
                warnings: result.warnings,
                rounding: result.rounding,
                provider_metadata: result.provider_metadata,
                response,
            })
        });
    if let Some(ctx) = context {
        let outcome = match &result {
            Ok(result) => crate::recording::OutcomeRecord {
                status: crate::recording::OutcomeStatus::Success,
                usage: serde_json::to_value(result.usage).ok(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boolean_criteria_keep_null_apart_from_absent() {
        let criteria: DecisionBooleanCriteria =
            serde_json::from_value(serde_json::json!({"true": null})).unwrap();
        assert_eq!(criteria.true_, Some(None));
        assert_eq!(criteria.false_, None);
        assert_eq!(
            serde_json::to_value(&criteria).unwrap(),
            serde_json::json!({"true": null})
        );
    }

    /// `ai@7.0.128 src/decide/validate-decision.ts:96-105`: Boolean criteria
    /// may only describe true and false.
    #[test]
    fn boolean_criteria_reject_other_keys() {
        assert!(
            serde_json::from_value::<DecisionBooleanCriteria>(
                serde_json::json!({"true": "yes", "maybe": "no"})
            )
            .is_err()
        );
    }
}
