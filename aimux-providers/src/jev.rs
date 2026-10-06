//! Official TypeSafe System One API for Jev decision models.

use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::decision_model::*;
use aimux_core::types::{TokenUsage, Usage};
use aimux_core::{AiMuxError, ApiCallError, Provider};
use aimux_provider_utils::{HttpRequest, load_api_key};

const JEV_ROUNDING: DecisionRounding = DecisionRounding {
    probability_decimals: Some(2),
    score_decimals: Some(2),
};

#[derive(Clone)]
pub struct JevConfig {
    pub api_key: String,
    /// Full POST URL for the official API, or an explicitly configured proxy.
    pub endpoint: String,
    pub headers: HashMap<String, String>,
    pub retry_config: aimux_core::retry::RetryConfig,
    pub probability_source: DecisionProbabilitySource,
}

impl JevConfig {
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            endpoint: "https://api.typesafe.ai/v1/systemone".into(),
            headers: HashMap::new(),
            retry_config: Default::default(),
            probability_source: DecisionProbabilitySource::Native,
        }
    }

    /// # Errors
    /// Returns `InvalidArgument` if `TYPESAFE_API_KEY` is unavailable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        Ok(Self::new(load_api_key(None, "TYPESAFE_API_KEY", "Jev")?))
    }

    /// Override the complete endpoint (for example a trusted API proxy).
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }
}

pub struct JevProvider {
    config: JevConfig,
}

impl JevProvider {
    #[must_use]
    pub fn new(config: JevConfig) -> Self {
        Self { config }
    }
}

impl Provider for JevProvider {
    fn name(&self) -> &str {
        "jev"
    }

    fn decision_model(&self, model_id: &str) -> Result<Box<dyn DecisionModel>, AiMuxError> {
        if model_id.trim().is_empty() {
            return Err(AiMuxError::InvalidArgument(
                "decision model ID cannot be empty".into(),
            ));
        }
        Ok(Box::new(JevDecisionModel {
            config: self.config.clone(),
            model_id: model_id.into(),
        }))
    }
}

pub struct JevDecisionModel {
    config: JevConfig,
    model_id: String,
}

fn request_body(model_id: &str, options: &DecisionCallOptions) -> Result<Value, AiMuxError> {
    options.validate()?;
    if !matches!(
        options.state,
        Value::String(_) | Value::Object(_) | Value::Array(_)
    ) {
        return Err(AiMuxError::InvalidArgument(
            "TypeSafe state must be a string, object, or array".into(),
        ));
    }
    if options
        .provider_options
        .as_ref()
        .is_some_and(|options| !options.is_empty())
    {
        return Err(AiMuxError::UnsupportedFunctionality(
            "Jev decision provider options are not defined".into(),
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
                if options.len() > 255 {
                    return Err(AiMuxError::InvalidArgument(format!(
                        "question {id:?} exceeds TypeSafe's 255 choice limit"
                    )));
                }
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
                if levels.len() > 10 {
                    return Err(AiMuxError::InvalidArgument(format!(
                        "question {id:?} exceeds Jev score limits"
                    )));
                }
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
struct WireResponse {
    model: String,
    answers: BTreeMap<String, WireAnswer>,
    usage: WireUsage,
}

#[derive(Deserialize)]
struct WireUsage {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

fn convert_response(
    data: WireResponse,
    raw: Value,
    headers: HashMap<String, String>,
    source: DecisionProbabilitySource,
) -> Result<DecisionResult, AiMuxError> {
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
            } => {
                let selected_probability = probabilities.get(&choice).ok_or_else(|| {
                    AiMuxError::InvalidResponseData(format!(
                        "Jev choice {id:?} has no selected probability"
                    ))
                })?;
                if probabilities
                    .values()
                    .any(|probability| probability > selected_probability)
                {
                    return Err(AiMuxError::InvalidResponseData(format!(
                        "Jev choice {id:?} is not a highest-probability option"
                    )));
                }
                DecisionAnswer::Choice {
                    selected: choice,
                    probabilities: Some(probabilities),
                    confidence: Some(confidence),
                }
            }
            WireAnswer::Score {
                score,
                legend,
                probabilities,
                confidence,
            } => {
                if legend.len() != probabilities.len() {
                    return Err(AiMuxError::InvalidResponseData(format!(
                        "Jev score {id:?} has mismatched legend and probability keys"
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
                                    "Jev score {id:?} has non-contiguous legend"
                                ))
                            })?
                            .clone(),
                    );
                    distribution.push(*probabilities.get(&key).ok_or_else(|| {
                        AiMuxError::InvalidResponseData(format!(
                            "Jev score {id:?} has missing probability"
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
        rounding: JEV_ROUNDING,
        answers,
        provider: "jev".into(),
        model: data.model,
        model_version: None,
        probability_source: source,
        usage,
        latency_ms: None,
        provider_metadata: Some(HashMap::from([("jev".into(), raw.clone())])),
        response: Some(DecisionResponse {
            headers: Some(headers),
            body: Some(raw),
        }),
    })
}

#[async_trait]
impl DecisionModel for JevDecisionModel {
    fn provider(&self) -> &str {
        "jev"
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn config_snapshot(&self) -> aimux_core::recording::ProviderRecord {
        let mut snapshot =
            aimux_core::recording::ProviderRecord::minimal(self.provider(), self.model_id());
        snapshot.base_url = Some(self.config.endpoint.clone());
        snapshot.api_key_source = if self.config.api_key.is_empty() {
            "none"
        } else {
            "explicit"
        }
        .into();
        snapshot.profile = Some(json!({"decision_capabilities": self.capabilities()}));
        snapshot.provider_options = Some(json!({"headers": self.config.headers}));
        snapshot
    }
    fn retry_config(&self) -> aimux_core::retry::RetryConfig {
        self.config.retry_config
    }
    fn capabilities(&self) -> DecisionCapabilities {
        DecisionCapabilities {
            rounding: JEV_ROUNDING,
            probability_source: self.config.probability_source,
            supports_boolean: true,
            supports_choice: true,
            supports_score: true,
            returns_distributions: true,
            max_questions: None,
            min_choices: Some(1),
            max_choices: Some(255),
            max_score_levels: Some(10),
        }
    }

    async fn do_decide(&self, options: &DecisionCallOptions) -> Result<DecisionResult, AiMuxError> {
        let body = request_body(&self.model_id, options)?;
        let mut headers = self.config.headers.clone();
        if !self.config.api_key.is_empty() {
            headers.insert(
                "Authorization".into(),
                format!("Bearer {}", self.config.api_key),
            );
        }
        if let Some(extra) = &options.headers {
            headers.extend(extra.clone());
        }
        let response = aimux_provider_utils::post_json_to_api(
            HttpRequest::new(
                &self.config.endpoint,
                headers.into_iter().collect(),
                options,
            ),
            body.clone(),
            aimux_provider_utils::create_json_response_handler::<WireResponse>(),
            aimux_provider_utils::create_standard_json_error_response_handler(),
        )
        .await?;
        let raw = response.raw_value.unwrap_or(Value::Null);
        let response_headers = response.response_headers;
        let result = convert_response(
            response.value,
            raw.clone(),
            response_headers.clone(),
            self.config.probability_source,
        )
        .and_then(|result| {
            result.validate(options)?;
            Ok(result)
        });
        result.map_err(|error| {
            AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(200),
                response_body: Some(raw.to_string()),
                response_headers: Some(response_headers),
                data: Some(raw),
                ..ApiCallError::new(error.to_string(), self.config.endpoint.clone(), body)
            }))
        })
    }
}
