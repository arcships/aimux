//! Native OpenAI Decisions API. Text contract; no generation fallback.
use std::collections::{BTreeMap, HashMap};

use aimux_core::decision_model::*;
use aimux_core::recording::ProviderRecord;
use aimux_core::types::{TokenUsage, Usage};
use aimux_core::{AiMuxError, ApiCallError};
use aimux_provider_utils::HttpRequest;
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderOptions {
    safety_identifier: Option<String>,
}

fn request_body(model: &str, options: &DecisionCallOptions) -> Result<Value, AiMuxError> {
    options.validate()?;
    let input = match &options.state {
        Value::String(text) => text.clone(),
        Value::Object(_) | Value::Array(_) => options.state.to_string(),
        _ => {
            return Err(AiMuxError::InvalidArgument(
                "OpenAI decision state must be text, an object, or an array".into(),
            ));
        }
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
                        let mut choice = json!({"value":option.label});
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
                    .map(|level| Ok(json!({"label":text_description(level)?})))
                    .collect::<Result<Vec<Value>, AiMuxError>>()?;
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
        choice: String,
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
    value: String,
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
                let mut distribution = BTreeMap::new();
                for entry in probabilities {
                    if distribution
                        .insert(entry.value, entry.probability)
                        .is_some()
                    {
                        return Err(invalid());
                    }
                }
                DecisionAnswer::Choice {
                    selected: choice,
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
                    if level != &DecisionDescription::Text(entry.label)
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
        let mut headers = super::model::build_auth_headers(&self.config);
        if let Some(extra) = &options.headers {
            headers.extend(extra.clone());
        }
        let response = aimux_provider_utils::post_json_to_api(
            HttpRequest::new(&url, headers.into_iter().collect(), options),
            body.clone(),
            aimux_provider_utils::create_json_response_handler::<WireResponse>(),
            super::openai_failed_response_handler(),
        )
        .await?;
        let raw = response.raw_value.unwrap_or(Value::Null);
        let headers = response.response_headers;
        convert_response(response.value, raw.clone(), headers.clone(), options).map_err(|error| {
            AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(200),
                response_body: Some(raw.to_string()),
                response_headers: Some(headers),
                data: Some(raw),
                ..ApiCallError::new(error.to_string(), url, body)
            }))
        })
    }
}
