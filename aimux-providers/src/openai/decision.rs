//! Native OpenAI Decisions API (an aimux extension; the AI SDK's OpenAI
//! package decides through its Responses model instead).
//!
//! The V4 call options carry what the API takes beyond them in
//! `provider_options.openai`: `images` (inline images before the state),
//! `choice_values` (native choice values per question and option label; a
//! label is its own value otherwise) and `safety_identifier`.
use std::collections::{HashMap, HashSet};

use aimux_core::AiMuxError;
use aimux_core::decision_model::*;
use async_trait::async_trait;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::config::OpenAIModelConfig;
use crate::decision_support::{
    DecisionConfig, DecisionImage, DecisionProbabilitySource, invalid, provider_metadata,
    response_info,
};

/// The OpenAI Decisions API takes at most this many images per request.
const MAX_IMAGES: usize = 128;

/// Native OpenAI Decisions model; [`OpenAIProvider::decision`](super::OpenAIProvider::decision)
/// creates it.
pub struct OpenAIDecisionModel {
    config: DecisionConfig,
    model_id: String,
}

impl OpenAIDecisionModel {
    /// A native Decisions model. The server validates model availability.
    /// # Errors
    /// Rejects an empty ID.
    pub(crate) fn from_config(
        model_id: &str,
        config: OpenAIModelConfig,
    ) -> Result<Self, AiMuxError> {
        if model_id.trim().is_empty() {
            return Err(AiMuxError::InvalidArgument(
                "decision model ID cannot be empty".into(),
            ));
        }
        Ok(Self {
            config: DecisionConfig::OpenAI(config),
            model_id: model_id.into(),
        })
    }
}

/// A native choice value: text or a Boolean.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
enum NativeValue {
    Text(String),
    Boolean(bool),
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderOptions {
    safety_identifier: Option<String>,
    #[serde(default)]
    images: Vec<DecisionImage>,
    /// Question id → option label → native value.
    #[serde(default)]
    choice_values: HashMap<String, HashMap<String, NativeValue>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScoreLevel {
    label: String,
    #[serde(default)]
    description: Option<String>,
}

fn text(input: &DecisionInput) -> Result<&str, AiMuxError> {
    match input {
        DecisionInput::Text(text) => Ok(text),
        _ => Err(AiMuxError::UnsupportedFunctionality(
            "OpenAI decisions require text instructions and criteria descriptions".into(),
        )),
    }
}

fn score_level(level: Option<&DecisionInput>) -> Result<Value, AiMuxError> {
    let unsupported = || {
        AiMuxError::UnsupportedFunctionality(
            "OpenAI score levels require text or {label, description}".into(),
        )
    };
    let level = match level {
        Some(DecisionInput::Text(label)) => ScoreLevel {
            label: label.clone(),
            description: None,
        },
        Some(DecisionInput::Object(object)) => {
            serde_json::from_value(Value::Object(object.clone())).map_err(|_| unsupported())?
        }
        _ => return Err(unsupported()),
    };
    if level.label.trim().is_empty() {
        return Err(AiMuxError::InvalidArgument(
            "OpenAI score level label cannot be blank".into(),
        ));
    }
    let mut wire = json!({"label": level.label});
    if let Some(description) = level.description {
        wire["description"] = json!(description);
    }
    Ok(wire)
}

/// The request body, and each choice question's (label, native value) pairs
/// for reading the answers back.
struct Request {
    body: Value,
    values: HashMap<String, Vec<(String, NativeValue)>>,
}

fn request_body(model: &str, options: &DecisionCallOptions) -> Result<Request, AiMuxError> {
    let provider_options: ProviderOptions = match options
        .provider_options
        .as_ref()
        .and_then(|p| p.get("openai"))
    {
        None => ProviderOptions::default(),
        Some(openai) => serde_json::from_value(Value::Object(openai.clone())).map_err(|error| {
            AiMuxError::InvalidArgument(format!("OpenAI decision options: {error}"))
        })?,
    };
    let state = match &options.state {
        DecisionInput::Text(text) => text.clone(),
        other => serde_json::to_string(other)?,
    };
    if provider_options.images.len() > MAX_IMAGES {
        return Err(AiMuxError::InvalidArgument(
            "too many decision images".into(),
        ));
    }
    let input = if provider_options.images.is_empty() {
        json!(state)
    } else {
        let mut content = Vec::new();
        for image in &provider_options.images {
            let inline = crate::decision_support::inline_image(image)?;
            let mut part = json!({"type":"input_image", "image_url":inline.data_url});
            if let Some(detail) = &image.detail {
                if !matches!(detail.as_str(), "low" | "high" | "auto" | "original") {
                    return Err(AiMuxError::InvalidArgument(
                        "invalid OpenAI image detail".into(),
                    ));
                }
                part["detail"] = json!(detail);
            }
            content.push(part);
        }
        content.push(json!({"type":"input_text", "text":state}));
        json!([{"role":"user", "content":content}])
    };
    for (id, values) in &provider_options.choice_values {
        let Some(DecisionQuestion::Choice { criteria, .. }) = options.questions.get(id) else {
            return Err(AiMuxError::InvalidArgument(format!(
                "OpenAI choice_values name {id:?}, which is not a choice question"
            )));
        };
        if let Some(label) = values.keys().find(|label| !criteria.contains_key(*label)) {
            return Err(AiMuxError::InvalidArgument(format!(
                "OpenAI choice_values for {id:?} name an unknown option {label:?}"
            )));
        }
    }
    let mut questions = Vec::with_capacity(options.questions.len());
    let mut values = HashMap::new();
    for (id, question) in &options.questions {
        let mut wire = match question {
            DecisionQuestion::Boolean {
                instructions,
                criteria,
            } => {
                if criteria.is_some() {
                    return Err(AiMuxError::UnsupportedFunctionality(
                        "OpenAI predicates do not accept true/false criteria".into(),
                    ));
                }
                json!({"type":"predicate", "instructions":text(instructions)?})
            }
            DecisionQuestion::Choice {
                instructions,
                criteria,
            } => {
                if !(2..=255).contains(&criteria.len()) {
                    return Err(AiMuxError::InvalidArgument(
                        "OpenAI choices require 2–255 options".into(),
                    ));
                }
                let overrides = provider_options.choice_values.get(id);
                let native: Vec<(String, NativeValue)> = criteria
                    .keys()
                    .map(|label| {
                        let value = overrides
                            .and_then(|values| values.get(label))
                            .cloned()
                            .unwrap_or_else(|| NativeValue::Text(label.clone()));
                        (label.clone(), value)
                    })
                    .collect();
                let mut unique = HashSet::new();
                if !native.iter().all(|(_, value)| unique.insert(value)) {
                    return Err(AiMuxError::InvalidArgument(format!(
                        "question {id:?} has duplicate native values"
                    )));
                }
                let choices = criteria
                    .values()
                    .zip(&native)
                    .map(|(description, (_, value))| {
                        let mut choice = json!({"value": value});
                        if let Some(description) = description {
                            choice["description"] = json!(text(description)?);
                        }
                        Ok(choice)
                    })
                    .collect::<Result<Vec<Value>, AiMuxError>>()?;
                values.insert(id.clone(), native);
                json!({"type":"choice", "instructions":text(instructions)?, "choices":choices})
            }
            DecisionQuestion::Score {
                instructions,
                criteria,
            } => {
                let levels = criteria
                    .iter()
                    .map(|level| score_level(level.as_ref()))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut labels = HashSet::new();
                if levels.iter().any(|level| !labels.insert(&level["label"])) {
                    return Err(AiMuxError::InvalidArgument(
                        "duplicate OpenAI score level label".into(),
                    ));
                }
                json!({"type":"score", "instructions":text(instructions)?, "levels":levels})
            }
        };
        wire["name"] = json!(id);
        questions.push(wire);
    }
    let mut body = json!({"model":model, "input":input, "questions":questions});
    if let Some(identifier) = provider_options.safety_identifier {
        if identifier.chars().count() > 128 {
            return Err(AiMuxError::InvalidArgument(
                "safety_identifier exceeds 128 characters".into(),
            ));
        }
        body["safety_identifier"] = json!(identifier);
    }
    Ok(Request { body, values })
}

#[derive(Deserialize)]
struct WireAnswer {
    name: String,
    #[serde(flatten)]
    answer: WireAnswerKind,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireAnswerKind {
    Predicate {
        probability: f64,
    },
    Choice {
        choice: NativeValue,
        confidence: f64,
        probabilities: Vec<ChoiceProbability>,
    },
    Score {
        score: f64,
        confidence: f64,
        probabilities: Vec<ScoreProbability>,
    },
    Refusal,
}

#[derive(Deserialize)]
struct ChoiceProbability {
    value: NativeValue,
    probability: f64,
}

#[derive(Deserialize)]
struct ScoreProbability {
    value: usize,
    label: String,
    probability: f64,
}

#[derive(Deserialize)]
struct WireResponse {
    model: String,
    answers: Vec<WireAnswer>,
    usage: Option<Value>,
}

fn convert_response(
    data: WireResponse,
    raw: Value,
    headers: HashMap<String, String>,
    request: &DecisionCallOptions,
    values: &HashMap<String, Vec<(String, NativeValue)>>,
) -> Result<DecisionResult, AiMuxError> {
    let mismatch = || invalid("OpenAI decision answers do not match the request");
    if data.model.trim().is_empty() || data.answers.len() != request.questions.len() {
        return Err(mismatch());
    }
    let mut by_name = HashMap::new();
    let mut extras = Map::new();
    for wire in data.answers {
        let question = request.questions.get(&wire.name).ok_or_else(mismatch)?;
        let answer = match (question, wire.answer) {
            (_, WireAnswerKind::Refusal) => {
                // V4 decisions return every answer or none.
                return Err(invalid(format!(
                    "OpenAI declined to answer question {:?}",
                    wire.name
                )));
            }
            (DecisionQuestion::Boolean { .. }, WireAnswerKind::Predicate { probability }) => {
                DecisionAnswer::Boolean { probability }
            }
            (
                DecisionQuestion::Choice { .. },
                WireAnswerKind::Choice {
                    choice,
                    confidence,
                    probabilities,
                },
            ) => {
                let native = &values[&wire.name];
                let label_of = |value: &NativeValue| {
                    native
                        .iter()
                        .find(|(_, native)| native == value)
                        .map(|(label, _)| label.clone())
                        .ok_or_else(mismatch)
                };
                let selected = label_of(&choice)?;
                let mut distribution = IndexMap::new();
                for entry in probabilities {
                    if distribution
                        .insert(label_of(&entry.value)?, entry.probability)
                        .is_some()
                    {
                        return Err(mismatch());
                    }
                }
                let mut extra = json!({"confidence": confidence});
                if let Some(typed) = request
                    .provider_options
                    .as_ref()
                    .and_then(|p| p.get("openai"))
                    .and_then(|openai| openai.get("choice_values"))
                    .and_then(|values| values.get(&wire.name))
                    && typed.get(&selected).is_some()
                {
                    extra["value"] = json!(choice);
                }
                extras.insert(wire.name.clone(), extra);
                DecisionAnswer::Choice {
                    choice: selected,
                    probabilities: Some(distribution),
                }
            }
            (
                DecisionQuestion::Score { criteria, .. },
                WireAnswerKind::Score {
                    score,
                    confidence,
                    probabilities,
                },
            ) => {
                if probabilities.len() != criteria.len() {
                    return Err(mismatch());
                }
                let mut distribution = vec![None; criteria.len()];
                for entry in probabilities {
                    let Some(level) = criteria.get(entry.value) else {
                        return Err(mismatch());
                    };
                    if score_level(level.as_ref())?["label"] != entry.label
                        || distribution[entry.value]
                            .replace(entry.probability)
                            .is_some()
                    {
                        return Err(mismatch());
                    }
                }
                let distribution = distribution
                    .into_iter()
                    .enumerate()
                    .map(|(index, probability)| Some((index.to_string(), probability?)))
                    .collect::<Option<IndexMap<_, _>>>()
                    .ok_or_else(mismatch)?;
                extras.insert(wire.name.clone(), json!({"confidence": confidence}));
                DecisionAnswer::Score {
                    score,
                    probabilities: Some(distribution),
                }
            }
            _ => return Err(mismatch()),
        };
        if by_name.insert(wire.name, answer).is_some() {
            return Err(mismatch());
        }
    }
    let answers = request
        .questions
        .keys()
        .map(|id| Some((id.clone(), by_name.remove(id)?)))
        .collect::<Option<IndexMap<_, _>>>()
        .ok_or_else(mismatch)?;
    // Usage is observational metadata. Missing or inconsistent counters must
    // not discard valid decisions; unknown counts stay unset.
    let count = |key: &str| {
        data.usage
            .as_ref()
            .and_then(|usage| usage[key].as_u64())
            .and_then(|n| u32::try_from(n).ok())
    };
    let usage = data.usage.as_ref().map(|_| DecisionUsage {
        input_tokens: count("input_tokens"),
        output_tokens: count("output_tokens"),
    });
    // Checked here as well as in `decide`, so a wrong answer keeps the
    // exchange's HTTP context.
    validate_decision_answers(&request.questions, &answers, None)?;
    Ok(DecisionResult {
        answers,
        rounding: None,
        usage,
        warnings: Vec::new(),
        provider_metadata: Some(provider_metadata(
            "openai",
            DecisionProbabilitySource::Native,
            extras,
        )),
        response: Some(response_info(Some(&data.model), headers, raw)),
    })
}

#[async_trait]
impl DecisionModel for OpenAIDecisionModel {
    fn provider(&self) -> &str {
        "openai"
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn supported_question_types(&self) -> &[DecisionQuestionType] {
        DecisionQuestionType::ALL
    }
    async fn do_decide(&self, options: &DecisionCallOptions) -> Result<DecisionResult, AiMuxError> {
        let Request { body, values } = request_body(&self.model_id, options)?;
        let url = self.config.url("/decisions")?;
        crate::decision_support::post(&self.config, options, &url, body, |raw, headers| {
            let data = serde_json::from_value(raw.clone())
                .map_err(|e| invalid(format!("OpenAI decisions: {e}")))?;
            convert_response(data, raw, headers, options, &values)
        })
        .await
    }
}
