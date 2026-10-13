//! Shared SystemOne wire codec. Provider policy stays in each adapter.
//!
//! The SystemOne wire is the V4 decision shape under other names: a question
//! map keyed by id, Choice criteria keyed by label, Score criteria as an
//! ordered list, and `noul` for Boolean.
use std::collections::HashMap;

use aimux_core::AiMuxError;
use aimux_core::decision_model::*;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::decision_support::{
    DecisionProbabilitySource, invalid, provider_metadata, response_info,
};

pub(crate) struct ResponseProfile<'a> {
    pub provider: &'a str,
    pub rounding: Option<DecisionRounding>,
    pub probability_source: DecisionProbabilitySource,
}

pub(crate) fn request_body(model_id: &str, options: &DecisionCallOptions) -> Value {
    let mut questions = Map::new();
    for (id, question) in &options.questions {
        let body = match question {
            DecisionQuestion::Boolean {
                instructions,
                criteria,
            } => {
                let mut body = json!({"type": "noul", "instructions": instructions});
                if let Some(criteria) = criteria {
                    body["criteria"] = json!(criteria);
                }
                body
            }
            DecisionQuestion::Choice {
                instructions,
                criteria,
            } => json!({"type": "choice", "instructions": instructions, "criteria": criteria}),
            DecisionQuestion::Score {
                instructions,
                criteria,
            } => json!({"type": "score", "instructions": instructions, "criteria": criteria}),
        };
        questions.insert(id.clone(), body);
    }
    json!({"model": model_id, "state": options.state, "questions": questions})
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireAnswer {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        probabilities: IndexMap<String, f64>,
        confidence: f64,
    },
    /// The wire's `legend` repeats the request's levels by index.
    Score {
        score: f64,
        legend: IndexMap<String, Option<DecisionInput>>,
        probabilities: IndexMap<String, f64>,
        confidence: f64,
    },
}

#[derive(Deserialize)]
pub(crate) struct WireResponse {
    model: String,
    /// A `null` answer has no value (vLLM reports skipped questions so).
    answers: IndexMap<String, Option<WireAnswer>>,
    usage: WireUsage,
}

#[derive(Deserialize)]
struct WireUsage {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

/// One V4 answer per requested question, from the wire's answer map.
pub(crate) fn convert_response(
    request: &DecisionCallOptions,
    data: WireResponse,
    raw: Value,
    headers: HashMap<String, String>,
    profile: ResponseProfile<'_>,
) -> Result<DecisionResult, AiMuxError> {
    let provider = profile.provider;
    if data.model.trim().is_empty() {
        return Err(invalid(format!("{provider} omitted response model")));
    }
    let mut wire = data.answers;
    let mut answers = IndexMap::new();
    let mut extras = Map::new();
    for (id, question) in &request.questions {
        let answer = wire
            .shift_remove(id)
            .ok_or_else(|| invalid(format!("{provider} omitted the answer to question {id:?}")))?
            .ok_or_else(|| invalid(format!("{provider} returned no answer to question {id:?}")))?;
        let answer = match (question, answer) {
            (DecisionQuestion::Boolean { .. }, WireAnswer::Noul { noul }) => {
                DecisionAnswer::Boolean { probability: noul }
            }
            (
                DecisionQuestion::Choice { .. },
                WireAnswer::Choice {
                    choice,
                    probabilities,
                    confidence,
                },
            ) => {
                extras.insert(id.clone(), json!({"confidence": confidence}));
                DecisionAnswer::Choice {
                    choice,
                    probabilities: Some(probabilities),
                }
            }
            (
                DecisionQuestion::Score { criteria, .. },
                WireAnswer::Score {
                    score,
                    legend,
                    probabilities,
                    confidence,
                },
            ) => {
                // The server must have read the levels the request sent, in
                // order; its distribution is keyed by their indices.
                if legend.len() != criteria.len()
                    || criteria
                        .iter()
                        .enumerate()
                        .any(|(index, level)| legend.get(&index.to_string()) != Some(level))
                {
                    return Err(invalid(format!(
                        "{provider} score {id:?} legend does not match the requested levels"
                    )));
                }
                extras.insert(id.clone(), json!({"confidence": confidence}));
                DecisionAnswer::Score {
                    score,
                    probabilities: Some(probabilities),
                }
            }
            _ => {
                return Err(invalid(format!(
                    "{provider} answered question {id:?} with the wrong type"
                )));
            }
        };
        answers.insert(id.clone(), answer);
    }
    if let Some(id) = wire.keys().next() {
        return Err(invalid(format!(
            "{provider} answered a question that was not asked: {id:?}"
        )));
    }
    // Checked here as well as in `decide`, so a wrong answer keeps the
    // exchange's HTTP context.
    validate_decision_answers(&request.questions, &answers, profile.rounding)?;
    Ok(DecisionResult {
        answers,
        rounding: profile.rounding,
        usage: Some(DecisionUsage {
            input_tokens: data.usage.input_tokens,
            output_tokens: data.usage.output_tokens,
        }),
        warnings: Vec::new(),
        provider_metadata: Some(provider_metadata(
            provider,
            profile.probability_source,
            extras,
        )),
        response: Some(response_info(Some(&data.model), headers, raw)),
    })
}
