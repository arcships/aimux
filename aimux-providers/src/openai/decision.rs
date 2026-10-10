//! Native OpenAI Decisions API, including inline images and typed choices.
use std::collections::{BTreeMap, HashMap, HashSet};

use aimux_core::AiMuxError;
use aimux_core::decision_model::*;
use aimux_core::recording::ProviderRecord;
use aimux_core::types::{TokenUsage, Usage};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use super::OpenAIConfig;

pub struct OpenAIDecisionModel {
    config: OpenAIConfig,
    model_id: String,
}

impl OpenAIDecisionModel {
    /// Create a native Decisions handle. The server validates model availability.
    /// # Errors
    /// Rejects an empty ID, another provider's config, and generation overrides.
    pub fn new(model_id: &str, config: OpenAIConfig) -> Result<Self, AiMuxError> {
        if model_id.trim().is_empty() {
            return Err(AiMuxError::InvalidArgument(
                "decision model ID cannot be empty".into(),
            ));
        }
        if config.provider != "openai" || config.body_overrides.is_some() {
            return Err(AiMuxError::UnsupportedFunctionality(
                "native OpenAI decisions require an OpenAI config without body_overrides".into(),
            ));
        }
        Ok(Self {
            config,
            model_id: model_id.into(),
        })
    }
}

fn text_description(description: &DecisionDescription) -> Result<&str, AiMuxError> {
    match description {
        DecisionDescription::Text(text) => Ok(text),
        _ => Err(AiMuxError::UnsupportedFunctionality(
            "OpenAI decisions require text instructions and criteria descriptions".into(),
        )),
    }
}

fn score_level(level: &DecisionDescription) -> Result<DecisionScoreLevel, AiMuxError> {
    let level = match level {
        DecisionDescription::Text(label) => DecisionScoreLevel {
            label: label.clone(),
            description: None,
        },
        DecisionDescription::Object(object) => {
            serde_json::from_value(Value::Object(object.clone())).map_err(|_| {
                AiMuxError::UnsupportedFunctionality(
                    "OpenAI score levels require text or {label, description}".into(),
                )
            })?
        }
        _ => {
            return Err(AiMuxError::UnsupportedFunctionality(
                "OpenAI score levels require text or {label, description}".into(),
            ));
        }
    };
    if level.label.trim().is_empty() {
        return Err(AiMuxError::InvalidArgument(
            "OpenAI score level label cannot be blank".into(),
        ));
    }
    Ok(level)
}

fn native_value(option: &DecisionOption) -> DecisionValue {
    option
        .value
        .clone()
        .unwrap_or_else(|| DecisionValue::Text(option.label.clone()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderOptions {
    safety_identifier: Option<String>,
}

fn request_body(model: &str, options: &DecisionCallOptions) -> Result<Value, AiMuxError> {
    options.validate()?;
    let text = match &options.state {
        Value::String(text) => text.clone(),
        Value::Object(_) | Value::Array(_) => options.state.to_string(),
        _ => {
            return Err(AiMuxError::InvalidArgument(
                "OpenAI decision state must be text, an object, or an array".into(),
            ));
        }
    };
    let input = if options.images.is_empty() {
        json!(text)
    } else {
        let mut content = Vec::new();
        for image in &options.images {
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
        content.push(json!({"type":"input_text", "text":text}));
        json!([{"role":"user", "content":content}])
    };
    let mut questions = Vec::with_capacity(options.questions.len());
    for question in &options.questions {
        let mut wire = match question {
            DecisionQuestion::Boolean {
                instructions,
                criteria,
                ..
            } => {
                if criteria.is_some() {
                    return Err(AiMuxError::UnsupportedFunctionality(
                        "OpenAI predicates do not accept true/false criteria".into(),
                    ));
                }
                json!({"type":"predicate", "instructions":text_description(instructions)?})
            }
            DecisionQuestion::Choice {
                instructions,
                options,
                ..
            } => {
                if !(2..=255).contains(&options.len()) {
                    return Err(AiMuxError::InvalidArgument(
                        "OpenAI choices require 2–255 options".into(),
                    ));
                }
                let choices = options
                    .iter()
                    .map(|option| {
                        let mut choice = json!({"value":native_value(option)});
                        if let Some(description) = &option.description {
                            choice["description"] = json!(text_description(description)?);
                        }
                        Ok(choice)
                    })
                    .collect::<Result<Vec<Value>, AiMuxError>>()?;
                json!({"type":"choice", "instructions":text_description(instructions)?, "choices":choices})
            }
            DecisionQuestion::Score {
                instructions,
                levels,
                ..
            } => {
                let levels = levels
                    .iter()
                    .map(score_level)
                    .collect::<Result<Vec<_>, _>>()?;
                let mut labels = HashSet::new();
                if levels.iter().any(|l| !labels.insert(&l.label)) {
                    return Err(AiMuxError::InvalidArgument(
                        "duplicate OpenAI score level label".into(),
                    ));
                }
                json!({"type":"score", "instructions":text_description(instructions)?, "levels":levels})
            }
        };
        wire["name"] = json!(question.id());
        questions.push(wire);
    }
    let mut body = json!({"model":model, "input":input, "questions":questions});
    if let Some(openai) = options
        .provider_options
        .as_ref()
        .and_then(|p| p.get("openai"))
    {
        let parsed: ProviderOptions = serde_json::from_value(openai.clone()).map_err(|error| {
            AiMuxError::InvalidArgument(format!("OpenAI decision options: {error}"))
        })?;
        if let Some(identifier) = parsed.safety_identifier {
            if identifier.chars().count() > 128 {
                return Err(AiMuxError::InvalidArgument(
                    "safety_identifier exceeds 128 characters".into(),
                ));
            }
            body["safety_identifier"] = json!(identifier);
        }
    }
    Ok(body)
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
        choice: DecisionValue,
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
    value: DecisionValue,
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
) -> Result<DecisionResult, AiMuxError> {
    let invalid = || {
        AiMuxError::InvalidResponseData("OpenAI decision answers do not match the request".into())
    };
    if data.model.trim().is_empty() || data.answers.len() != request.questions.len() {
        return Err(invalid());
    }
    let mut answers = BTreeMap::new();
    for wire in data.answers {
        let answer = match wire.answer {
            WireAnswerKind::Predicate { probability } => DecisionAnswer::Boolean {
                probability_true: probability,
            },
            WireAnswerKind::Refusal => DecisionAnswer::Refusal,
            WireAnswerKind::Choice {
                choice,
                confidence,
                probabilities,
            } => {
                let Some(DecisionQuestion::Choice { options, .. }) =
                    request.questions.iter().find(|q| q.id() == wire.name)
                else {
                    return Err(invalid());
                };
                let selected = options
                    .iter()
                    .find(|o| native_value(o) == choice)
                    .ok_or_else(invalid)?;
                let mut distribution = BTreeMap::new();
                for entry in probabilities {
                    let option = options
                        .iter()
                        .find(|o| native_value(o) == entry.value)
                        .ok_or_else(invalid)?;
                    if distribution
                        .insert(option.label.clone(), entry.probability)
                        .is_some()
                    {
                        return Err(invalid());
                    }
                }
                DecisionAnswer::Choice {
                    selected: selected.label.clone(),
                    value: selected.value.clone(),
                    probabilities: Some(distribution),
                    confidence: Some(confidence),
                }
            }
            WireAnswerKind::Score {
                score,
                confidence,
                probabilities,
            } => {
                let Some(DecisionQuestion::Score { levels, .. }) =
                    request.questions.iter().find(|q| q.id() == wire.name)
                else {
                    return Err(invalid());
                };
                if probabilities.len() != levels.len() {
                    return Err(invalid());
                }
                let mut distribution = vec![None; levels.len()];
                for entry in probabilities {
                    let Some(level) = levels.get(entry.value) else {
                        return Err(invalid());
                    };
                    if score_level(level)?.label != entry.label
                        || distribution[entry.value]
                            .replace(entry.probability)
                            .is_some()
                    {
                        return Err(invalid());
                    }
                }
                DecisionAnswer::Score {
                    expected_value: score,
                    levels: levels.clone(),
                    probabilities: Some(
                        distribution
                            .into_iter()
                            .collect::<Option<Vec<_>>>()
                            .ok_or_else(invalid)?,
                    ),
                    confidence: Some(confidence),
                }
            }
        };
        if answers.insert(wire.name, answer).is_some() {
            return Err(invalid());
        }
    }
    // Usage is observational metadata. Missing or inconsistent counters must
    // not discard valid decisions; retain raw values and leave unknowns unset.
    let usage = data.usage.map(|raw| {
        let count = |value: &Value| value.as_u64().and_then(|n| u32::try_from(n).ok());
        let input = count(&raw["input_tokens"]);
        let output = count(&raw["output_tokens"]);
        let cached = count(&raw["input_tokens_details"]["cached_tokens"]);
        let written = count(&raw["input_tokens_details"]["cache_write_tokens"]);
        let reasoning = count(&raw["output_tokens_details"]["reasoning_tokens"]);
        Usage {
            input_tokens: TokenUsage {
                total: input,
                no_cache: input
                    .zip(cached)
                    .zip(written)
                    .and_then(|((total, read), write)| total.checked_sub(read)?.checked_sub(write)),
                cache_read: cached,
                cache_write: written,
                ..Default::default()
            },
            output_tokens: TokenUsage {
                total: output,
                text: output
                    .zip(reasoning)
                    .and_then(|(total, reasoning)| total.checked_sub(reasoning)),
                reasoning,
                ..Default::default()
            },
            raw: Some(raw),
        }
    });
    let result = DecisionResult {
        rounding: DecisionRounding::default(),
        answers,
        provider: "openai".into(),
        model: data.model,
        model_version: None,
        probability_source: DecisionProbabilitySource::Native,
        usage,
        latency_ms: None,
        provider_metadata: Some(HashMap::from([("openai".into(), raw.clone())])),
        response: Some(DecisionResponse {
            headers: Some(headers),
            body: Some(raw),
        }),
    };
    result.validate(request)?;
    Ok(result)
}

#[async_trait]
impl DecisionModel for OpenAIDecisionModel {
    fn provider(&self) -> &str {
        "openai"
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn retry_config(&self) -> aimux_core::retry::RetryConfig {
        self.config.retry_config
    }
    fn capabilities(&self) -> DecisionCapabilities {
        DecisionCapabilities {
            supports_images: true,
            max_images: Some(128),
            supports_typed_choices: true,
            rounding: DecisionRounding::default(),
            probability_source: DecisionProbabilitySource::Native,
            supports_boolean: true,
            supports_choice: true,
            supports_score: true,
            returns_distributions: true,
            max_questions: None,
            min_choices: Some(2),
            max_choices: Some(255),
            max_score_levels: None,
        }
    }
    fn config_snapshot(&self) -> ProviderRecord {
        let mut record =
            super::config_snapshot_from_config(self.provider(), self.model_id(), &self.config);
        record.profile = Some(
            json!({"decision_protocol":"openai_decisions", "decision_capabilities":self.capabilities()}),
        );
        record
    }
    async fn do_decide(&self, options: &DecisionCallOptions) -> Result<DecisionResult, AiMuxError> {
        let body = request_body(&self.model_id, options)?;
        let url = format!("{}/decisions", self.config.base_url.trim_end_matches('/'));
        crate::decision_support::post(&self.config, options, &url, body, |raw, headers| {
            let data = serde_json::from_value(raw.clone())
                .map_err(|e| crate::decision_support::invalid(format!("OpenAI decisions: {e}")))?;
            convert_response(data, raw, headers, options)
        })
        .await
    }
}
