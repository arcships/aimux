//! Official TypeSafe System One API for Jev decision models.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Value, json};

use aimux_core::decision_model::*;
use aimux_core::{AiMuxError, ApiCallError, Provider};
use aimux_provider_utils::{HttpRequest, load_api_key};

use crate::systemone::{ResponseProfile, WireResponse};

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
        if options
            .provider_options
            .as_ref()
            .is_some_and(|opts| !opts.is_empty())
        {
            return Err(AiMuxError::UnsupportedFunctionality(
                "Jev decision provider options are not defined".into(),
            ));
        }
        for question in &options.questions {
            let over_limit = match question {
                DecisionQuestion::Choice { options, .. } => options.len() > 255,
                DecisionQuestion::Score { levels, .. } => levels.len() > 10,
                DecisionQuestion::Boolean { .. } => false,
            };
            if over_limit {
                return Err(AiMuxError::InvalidArgument(format!(
                    "question {:?} exceeds TypeSafe criteria limits",
                    question.id()
                )));
            }
        }
        let body = crate::systemone::request_body(&self.model_id, options)?;
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
        let result = crate::systemone::convert_response(
            response.value,
            raw.clone(),
            response_headers.clone(),
            ResponseProfile {
                provider: self.provider(),
                rounding: JEV_ROUNDING,
                probability_source: self.config.probability_source,
            },
        )
        .and_then(|result| {
            result.validate(options)?;
            for (id, answer) in &result.answers {
                if let DecisionAnswer::Choice {
                    selected,
                    probabilities: Some(probabilities),
                    ..
                } = answer
                {
                    // Jev's selected choice is an argmax; ties are valid.
                    let selected_probability = probabilities[selected];
                    if probabilities.values().any(|p| *p > selected_probability) {
                        return Err(AiMuxError::InvalidResponseData(format!(
                            "Jev choice {id:?} is not a highest-probability option"
                        )));
                    }
                }
            }
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
