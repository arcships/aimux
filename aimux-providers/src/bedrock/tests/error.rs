//! Ports of `@ai-sdk/amazon-bedrock` 5.0.100 `src/amazon-bedrock-error.test.ts`.
//!
//! `amazonBedrockFailedResponseHandler` is `bedrock_failed_response_handler`
//! in `bedrock/mod.rs`.

use bytes::Bytes;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use url::Url;

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{FetchResponse, ResponseHandlerInput};

use crate::bedrock::bedrock_failed_response_handler;

const URL: &str = "https://bedrock-runtime.us-east-1.amazonaws.com/model/test/invoke";

/// A `400 Bad Request` JSON response with `body`.
fn make_response(body: &Value) -> FetchResponse {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    FetchResponse::from_bytes(
        StatusCode::BAD_REQUEST,
        headers,
        Url::parse(URL).expect("valid URL"),
        Bytes::from(serde_json::to_vec(body).expect("JSON body")),
    )
}

/// The message of the `APICallError` the handler builds for `body`.
async fn get_error_message(body: &Value) -> String {
    let output = bedrock_failed_response_handler()
        .handle(ResponseHandlerInput {
            url: URL.to_string(),
            request_body_values: json!({}),
            response: make_response(body),
            abort_signal: None,
            max_json_response_bytes: None,
        })
        .await
        .expect("the handler produces an error value");
    match output.value {
        AiMuxError::ApiCall(error) => error.message,
        other => panic!("expected an APICallError, got {other:?}"),
    }
}

mod amazon_bedrock_failed_response_handler {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-error.test.ts › amazonBedrockFailedResponseHandler › preserves the provider message when the error type is omitted
    #[tokio::test]
    async fn preserves_the_provider_message_when_the_error_type_is_omitted() {
        assert_eq!(
            get_error_message(&json!({ "message": "boom" })).await,
            "boom"
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-error.test.ts › amazonBedrockFailedResponseHandler › prefixes the provider message when the error type is present
    #[tokio::test]
    async fn prefixes_the_provider_message_when_the_error_type_is_present() {
        assert_eq!(
            get_error_message(&json!({ "type": "ValidationException", "message": "boom" })).await,
            "ValidationException: boom"
        );
    }
}
