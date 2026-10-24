//! Upstream `xai-error.test.ts`.

use aimux_core::ApiCallError;
use aimux_core::error::AiMuxError;
use aimux_provider_utils::{FetchResponse, ResponseHandlerInput};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use reqwest::{StatusCode, Url};
use serde_json::{Value, json};

use crate::xai::xai_failed_response_handler;

/// The upstream `makeResponse` helper: a 400 with a JSON body.
fn make_response(url: &str, body: &Value) -> FetchResponse {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    FetchResponse::from_bytes(
        StatusCode::BAD_REQUEST,
        headers,
        Url::parse(url).expect("valid URL"),
        serde_json::to_vec(body).expect("body serializes").into(),
    )
}

/// `xaiFailedResponseHandler({ url, requestBodyValues: {}, response })`, the
/// error value it returns.
async fn failed_response_error(url: &str, body: &Value) -> ApiCallError {
    let output = xai_failed_response_handler()
        .handle(ResponseHandlerInput {
            url: url.to_string(),
            request_body_values: json!({}),
            response: make_response(url, body),
            abort_signal: None,
            max_json_response_bytes: None,
        })
        .await
        .expect("the failed-response handler builds an error");
    match output.value {
        AiMuxError::ApiCall(error) => *error,
        other => panic!("expected an APICallError, got {other:?}"),
    }
}

mod xai_failed_response_handler {
    use super::*;

    /// upstream: @ai-sdk/xai src/xai-error.test.ts › xaiFailedResponseHandler › extracts message from the nested API error shape
    #[tokio::test]
    async fn extracts_message_from_the_nested_api_error_shape() {
        let error = failed_response_error(
            "https://api.x.ai/v1/images/generations",
            &json!({
                "error": {
                    "message": "Invalid value: temperature must be between 0 and 2",
                    "type": "invalid_request_error",
                    "code": "invalid_value",
                },
            }),
        )
        .await;

        assert_eq!(
            error.message,
            "Invalid value: temperature must be between 0 and 2"
        );
    }

    /// upstream: @ai-sdk/xai src/xai-error.test.ts › xaiFailedResponseHandler › extracts message and code from responses api error shape
    #[tokio::test]
    async fn extracts_message_and_code_from_responses_api_error_shape() {
        let error = failed_response_error(
            "https://api.x.ai/v1/responses",
            &json!({
                "code": "Client specified an invalid argument",
                "error": "Invalid request content: Each message must have at least one content element.",
            }),
        )
        .await;

        assert_eq!(
            error.message,
            "Client specified an invalid argument: Invalid request content: Each message must have at least one content element."
        );
    }

    /// upstream: @ai-sdk/xai src/xai-error.test.ts › xaiFailedResponseHandler › extracts message from text to speech error shape
    #[tokio::test]
    async fn extracts_message_from_text_to_speech_error_shape() {
        let error = failed_response_error(
            "https://api.x.ai/v1/tts",
            &json!({
                "error": "speed must be between 0.7 and 1.5",
            }),
        )
        .await;

        assert_eq!(error.message, "speed must be between 0.7 and 1.5");
    }
}
