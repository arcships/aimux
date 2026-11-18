//! Jev decision model over the SystemOne wire protocol.

use async_trait::async_trait;
use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_core::decision_model::*;
use aimux_provider_utils::HttpRequest;

use super::provider::JevConfig;
use super::systemone::{ResponseProfile, WireResponse, convert_response, request_body};

/// Failed-response handler of the Jev endpoint: `message`, and `type`
/// else `code` as the provider code, from the body's `error` object, or from
/// the top level when there is none. This is the mapping the endpoint used
/// with the removed shared handler; it is not an AI SDK schema.
fn jev_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    crate::shared::message_type_error_response_handler()
}

/// Jev rounds probabilities and scores to two decimals.
const JEV_ROUNDING: DecisionRounding = DecisionRounding {
    probability_decimals: Some(2),
    score_decimals: Some(2),
};

/// TypeSafe's criteria limits, checked before any request.
const MAX_CHOICES: usize = 255;
const MAX_SCORE_LEVELS: usize = 10;

pub struct JevDecisionModel {
    config: JevConfig,
    model_id: String,
}

impl JevDecisionModel {
    pub(super) fn from_config(config: JevConfig, model_id: String) -> Self {
        Self { config, model_id }
    }
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
                return Err(AiMuxError::invalid_argument(format!(
                    "question {id:?} exceeds TypeSafe criteria limits"
                )));
            }
        }
        let body = request_body(&self.model_id, options);
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
            jev_failed_response_handler(),
        )
        .await?;
        let raw = response.raw_value.unwrap_or(Value::Null);
        let response_headers = response.response_headers;
        let result = convert_response(
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
                &error,
                200,
                &self.config.endpoint,
                body,
                raw,
                response_headers,
            )
        })
    }
}
