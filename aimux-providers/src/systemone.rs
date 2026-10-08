//! Shared SystemOne wire codec. Provider policy stays in each adapter.
use aimux_core::AiMuxError;
use aimux_core::decision_model::*;
use aimux_core::types::{TokenUsage, Usage};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

pub(crate) struct ResponseProfile<'a> {
    pub provider: &'a str,
    pub rounding: DecisionRounding,
    pub probability_source: DecisionProbabilitySource,
}

pub(crate) fn request_body(
    model_id: &str,
    options: &DecisionCallOptions,
) -> Result<Value, AiMuxError> {
    options.validate()?;
    if !matches!(
        options.state,
        Value::String(_) | Value::Object(_) | Value::Array(_)
    ) {
        return Err(AiMuxError::InvalidArgument(
            "SystemOne state must be a string, object, or array".into(),
        ));
    }
    let mut questions = serde_json::Map::new();
    for question in &options.questions {
        let id = question.id();
        let body = match question {
            DecisionQuestion::Boolean {
                instructions,
                criteria,
                ..
            } => {
                let mut body = json!({"type": "noul", "instructions": instructions});
                if let Some(criteria) = criteria {
                    body["criteria"] = json!(criteria);
                }
                body
            }
            DecisionQuestion::Choice {
                instructions,
                options,
                ..
            } => {
                let criteria: serde_json::Map<String, Value> = options
                    .iter()
                    .map(|option| (option.label.clone(), json!(option.description)))
                    .collect();
                json!({"type": "choice", "instructions": instructions, "criteria": criteria})
            }
            DecisionQuestion::Score {
                instructions,
                levels,
                ..
            } => {
                json!({"type": "score", "instructions": instructions, "criteria": levels})
            }
        };
        questions.insert(id.into(), body);
    }
    Ok(json!({"model": model_id, "state": options.state, "questions": questions}))
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireAnswer {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Score {
        score: f64,
        legend: BTreeMap<String, DecisionDescription>,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
}

#[derive(Deserialize)]
pub(crate) struct WireResponse {
    model: String,
    answers: BTreeMap<String, WireAnswer>,
    usage: WireUsage,
}

#[derive(Deserialize)]
struct WireUsage {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

pub(crate) fn convert_response(
    data: WireResponse,
    raw: Value,
    headers: HashMap<String, String>,
    profile: ResponseProfile<'_>,
) -> Result<DecisionResult, AiMuxError> {
    let provider = profile.provider;
    if data.model.trim().is_empty() {
        return Err(AiMuxError::InvalidResponseData(format!(
            "{provider} omitted response model"
        )));
    }
    let mut answers = BTreeMap::new();
    for (id, answer) in data.answers {
        let answer = match answer {
            WireAnswer::Noul { noul } => DecisionAnswer::Boolean {
                probability_true: noul,
            },
            WireAnswer::Choice {
                choice,
                probabilities,
                confidence,
            } => DecisionAnswer::Choice {
                value: None,
                selected: choice,
                probabilities: Some(probabilities),
                confidence: Some(confidence),
            },
            WireAnswer::Score {
                score,
                legend,
                probabilities,
                confidence,
            } => {
                if legend.len() != probabilities.len() {
                    return Err(AiMuxError::InvalidResponseData(format!(
                        "{provider} score {id:?} has mismatched legend and probability keys"
                    )));
                }
                let mut levels = Vec::with_capacity(legend.len());
                let mut distribution = Vec::with_capacity(legend.len());
                for index in 0..legend.len() {
                    let key = index.to_string();
                    levels.push(
                        legend
                            .get(&key)
                            .ok_or_else(|| {
                                AiMuxError::InvalidResponseData(format!(
                                    "{provider} score {id:?} has non-contiguous legend"
                                ))
                            })?
                            .clone(),
                    );
                    distribution.push(*probabilities.get(&key).ok_or_else(|| {
                        AiMuxError::InvalidResponseData(format!(
                            "{provider} score {id:?} has missing probability"
                        ))
                    })?);
                }
                DecisionAnswer::Score {
                    expected_value: score,
                    levels,
                    probabilities: Some(distribution),
                    confidence: Some(confidence),
                }
            }
        };
        answers.insert(id, answer);
    }
    let usage = Some(Usage {
        input_tokens: TokenUsage {
            total: data.usage.input_tokens,
            ..Default::default()
        },
        output_tokens: TokenUsage {
            total: data.usage.output_tokens,
            ..Default::default()
        },
        raw: raw.get("usage").cloned(),
    });
    Ok(DecisionResult {
        rounding: profile.rounding,
        answers,
        provider: provider.into(),
        model: data.model,
        model_version: None,
        probability_source: profile.probability_source,
        usage,
        latency_ms: None,
        provider_metadata: Some(HashMap::from([(provider.into(), raw.clone())])),
        response: Some(DecisionResponse {
            headers: Some(headers),
            body: Some(raw),
        }),
    })
}
