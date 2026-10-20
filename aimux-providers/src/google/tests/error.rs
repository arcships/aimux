use bytes::Bytes;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{FetchResponse, ResponseHandlerInput};

use crate::google::google_failed_response_handler;

/// `expect(actual).toMatchObject(expected)`: every key of an expected object
/// is present and matches, arrays match element by element with the same
/// length, everything else is equal.
fn assert_error_match_object(actual: &Value, expected: &Value, path: &str) {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => {
            for (key, expected) in expected {
                let actual = actual
                    .get(key)
                    .unwrap_or_else(|| panic!("missing key {path}.{key}"));
                assert_error_match_object(actual, expected, &format!("{path}.{key}"));
            }
        }
        (Value::Array(actual), Value::Array(expected)) => {
            assert_eq!(actual.len(), expected.len(), "array length at {path}");
            for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                assert_error_match_object(actual, expected, &format!("{path}[{index}]"));
            }
        }
        _ => assert_eq!(actual, expected, "value at {path}"),
    }
}

mod google_failed_response_handler {
    use super::*;

    /// upstream: @ai-sdk/google src/google-error.test.ts › googleFailedResponseHandler › preserves google.rpc.RetryInfo in APICallError.data
    #[tokio::test]
    async fn preserves_google_rpc_retry_info_in_api_call_error_data() {
        let response_body = include_str!("fixtures/google-429-retry-info.json");
        let url = "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent";

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let result = google_failed_response_handler()
            .handle(ResponseHandlerInput {
                url: url.to_owned(),
                request_body_values: json!({ "contents": [] }),
                response: FetchResponse::from_bytes(
                    StatusCode::TOO_MANY_REQUESTS,
                    headers,
                    url::Url::parse(url).unwrap(),
                    Bytes::from_static(response_body.as_bytes()),
                ),
                abort_signal: None,
                max_json_response_bytes: None,
            })
            .await
            .expect("the handler produces an error value");

        let AiMuxError::ApiCall(error) = result.value else {
            panic!("expected an APICallError, got {:?}", result.value)
        };
        assert_error_match_object(
            error.data.as_ref().expect("APICallError.data"),
            &json!({
                "error": {
                    "details": [
                        {
                            "@type": "type.googleapis.com/google.rpc.QuotaFailure",
                        },
                        {
                            "@type": "type.googleapis.com/google.rpc.RetryInfo",
                            "retryDelay": "34.4s",
                        },
                    ],
                },
            }),
            "data",
        );
    }
}
