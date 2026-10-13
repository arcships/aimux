//! Official TypeSafe System One API for Jev decision models.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;

use std::sync::Arc;

use aimux_core::decision_model::*;
use aimux_core::{AiMuxError, Provider};
use aimux_provider_utils::{HttpRequest, load_api_key};

use crate::decision_support::DecisionProbabilitySource;
use crate::systemone::{ResponseProfile, WireResponse};

/// Jev rounds probabilities and scores to two decimals.
const JEV_ROUNDING: DecisionRounding = DecisionRounding {
    probability_decimals: Some(2),
    score_decimals: Some(2),
};

/// TypeSafe's criteria limits, checked before any request.
const MAX_CHOICES: usize = 255;
const MAX_SCORE_LEVELS: usize = 10;

#[derive(Clone)]
pub struct JevConfig {
    pub api_key: String,
    /// Full POST URL for the official API, or an explicitly configured proxy.
    pub endpoint: String,
    pub headers: HashMap<String, String>,
    pub probability_source: DecisionProbabilitySource,
}

impl JevConfig {
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            endpoint: "https://api.typesafe.ai/v1/systemone".into(),
            headers: HashMap::new(),
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
    crate::__unsupported_required_model!(language_model, LanguageModel, "languageModel");
    crate::__unsupported_required_model!(embedding_model, EmbeddingModel, "embeddingModel");
    crate::__unsupported_required_model!(image_model, ImageModel, "imageModel");

    fn decision_model(&self, model_id: &str) -> Option<Result<Arc<dyn DecisionModel>, AiMuxError>> {
        if model_id.trim().is_empty() {
            return Some(Err(AiMuxError::InvalidArgument(
                "decision model ID cannot be empty".into(),
            )));
        }
        Some(Ok(Arc::new(JevDecisionModel {
            config: self.config.clone(),
            model_id: model_id.into(),
        })))
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
    fn supported_question_types(&self) -> &[DecisionQuestionType] {
        DecisionQuestionType::ALL
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
        for (id, question) in &options.questions {
            let over_limit = match question {
                DecisionQuestion::Choice { criteria, .. } => criteria.len() > MAX_CHOICES,
                DecisionQuestion::Score { criteria, .. } => criteria.len() > MAX_SCORE_LEVELS,
                DecisionQuestion::Boolean { .. } => false,
            };
            if over_limit {
                return Err(AiMuxError::InvalidArgument(format!(
                    "question {id:?} exceeds TypeSafe criteria limits"
                )));
            }
        }
        let body = crate::systemone::request_body(&self.model_id, options);
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
            options,
            response.value,
            raw.clone(),
            response_headers.clone(),
            ResponseProfile {
                provider: self.provider(),
                rounding: Some(JEV_ROUNDING),
                probability_source: self.config.probability_source,
            },
        );
        result.map_err(|error| {
            aimux_provider_utils::invalid_response_api_call(
                error.to_string(),
                200,
                &self.config.endpoint,
                body,
                raw,
                response_headers,
            )
        })
    }
}
