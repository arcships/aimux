//! Integration coverage for the single-exchange API helpers.

use futures::StreamExt;
use serde::Deserialize;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::{AbortSignal, AiMuxError};
use aimux_provider_utils::{
    HttpBody, HttpRequest, ProviderErrorParts, create_binary_response_handler,
    create_event_source_response_handler, create_json_error_response_handler,
    create_json_response_handler, get_from_api, post_json_to_api, post_to_api,
};

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Reply {
    value: String,
}

fn request(url: String) -> HttpRequest {
    HttpRequest {
        url,
        headers: vec![("authorization".into(), "Bearer test".into())],
        abort_signal: None,
        call_id: None,
        recording_context: None,

        ..Default::default()
    }
}

fn failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    create_json_error_response_handler(|data| {
        let error = data.get("error").unwrap_or(data);
        ProviderErrorParts {
            message: error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("request failed")
                .to_owned(),
            provider_code: error
                .get("code")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        }
    })
}

#[tokio::test]
async fn post_json_dispatches_typed_success_handler() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/call"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-test", "yes")
                .set_body_json(json!({"value": "ok"})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let output = post_json_to_api(
        request(format!("{}/call", server.uri())),
        json!({"prompt": "hello"}),
        create_json_response_handler::<Reply>(),
        failed_response_handler(),
    )
    .await
    .unwrap();

    assert_eq!(output.value, Reply { value: "ok".into() });
    assert_eq!(output.raw_value, Some(json!({"value": "ok"})));
    assert_eq!(output.response_headers.get("x-test"), Some(&"yes".into()));
}

#[tokio::test]
async fn failed_handler_preserves_provider_context_and_redacts_secrets() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/call"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("set-cookie", "session=secret")
                .insert_header("retry-after", "1")
                .set_body_json(json!({
                    "error": {
                        "message": "slow down",
                        "code": "rate_limit",
                        "token": "must-not-leak"
                    }
                })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let error = post_json_to_api(
        request(format!("{}/call?api_key=secret", server.uri())),
        json!({"api_key": "secret", "prompt": "hello"}),
        create_json_response_handler::<Reply>(),
        failed_response_handler(),
    )
    .await
    .unwrap_err();

    let AiMuxError::ApiCall(error) = error else {
        panic!("expected ApiCallError")
    };
    assert_eq!(error.status_code, Some(429));
    assert_eq!(error.message, "slow down");
    assert_eq!(error.provider_code.as_deref(), Some("rate_limit"));
    assert!(error.is_retryable);
    assert!(!error.url.contains("api_key"));
    assert_eq!(error.request_body_values["api_key"], "[REDACTED]");
    assert_eq!(
        error.response_headers.as_ref().unwrap().get("set-cookie"),
        Some(&"[REDACTED]".into())
    );
    assert_eq!(error.data.as_ref().unwrap()["error"]["token"], "[REDACTED]");
}

#[tokio::test]
async fn invalid_success_json_is_contextual_api_call_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .expect(1)
        .mount(&server)
        .await;

    let error = post_json_to_api(
        request(server.uri()),
        json!({"prompt": "hello"}),
        create_json_response_handler::<Reply>(),
        failed_response_handler(),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(error, AiMuxError::ApiCall(ref e) if e.status_code == Some(200)
        && e.response_body.as_deref() == Some("not json")
        && e.message.starts_with("Invalid JSON response:"))
    );
}

#[tokio::test]
async fn binary_handler_accepts_an_empty_success_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(Vec::<u8>::new()))
        .expect(1)
        .mount(&server)
        .await;

    let output = post_to_api(
        request(server.uri()),
        HttpBody::Empty,
        create_binary_response_handler(),
        failed_response_handler(),
    )
    .await
    .unwrap();

    assert!(output.value.is_empty());
}

#[tokio::test]
async fn event_source_handler_deserializes_each_data_event() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"value\":\"one\"}\n\ndata: {\"value\":\"two\"}\n\ndata: [DONE]\n\n",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let output = post_json_to_api(
        request(server.uri()),
        json!({}),
        create_event_source_response_handler::<Reply>(),
        failed_response_handler(),
    )
    .await
    .unwrap();
    let values = output
        .value
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(
        values,
        vec![
            Reply {
                value: "one".into()
            },
            Reply {
                value: "two".into()
            }
        ]
    );
}

#[tokio::test]
async fn raw_post_and_get_share_the_same_dispatch_contract() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/binary"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes([1, 2, 3]))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"value": "get"})))
        .expect(1)
        .mount(&server)
        .await;

    let binary = post_to_api(
        request(format!("{}/binary", server.uri())),
        HttpBody::Bytes(vec![9], "application/octet-stream".into()),
        create_binary_response_handler(),
        failed_response_handler(),
    )
    .await
    .unwrap();
    assert_eq!(binary.value.as_ref(), &[1, 2, 3]);

    let json = get_from_api(
        request(format!("{}/json", server.uri())),
        create_json_response_handler::<Reply>(),
        failed_response_handler(),
    )
    .await
    .unwrap();
    assert_eq!(json.value.value, "get");
}

#[tokio::test]
async fn caller_abort_is_not_reclassified_as_transport_failure() {
    let signal = AbortSignal::new();
    signal.abort();
    let mut request = request("http://127.0.0.1:1/unreachable".into());
    request.abort_signal = Some(signal);

    let error = post_json_to_api(
        request,
        json!({}),
        create_json_response_handler::<Reply>(),
        failed_response_handler(),
    )
    .await
    .unwrap_err();

    assert!(matches!(error, AiMuxError::Aborted(_)));
}

#[tokio::test]
async fn provider_utils_performs_exactly_one_exchange() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({
            "error": {"message": "retry me", "code": "unavailable"}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let error = post_json_to_api(
        request(server.uri()),
        json!({}),
        create_json_response_handler::<Reply>(),
        failed_response_handler(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AiMuxError::ApiCall(ref e) if e.is_retryable));
}

#[tokio::test]
async fn oversize_error_body_is_truncated_not_replaced() {
    let server = MockServer::start().await;
    // >64 KiB error body: the provider's bytes must survive (truncated),
    // not be replaced by a size-limit error.
    let huge = format!(
        "{{\"error\":{{\"message\":\"real provider failure\",\"detail\":\"{}\"}}}}",
        "x".repeat(70 * 1024)
    );
    Mock::given(method("POST"))
        .and(path("/call"))
        .respond_with(
            ResponseTemplate::new(400)
                .insert_header("content-type", "application/json")
                .set_body_string(huge),
        )
        .expect(1)
        .mount(&server)
        .await;

    let error = post_json_to_api::<Reply>(
        request(format!("{}/call", server.uri())),
        json!({"input": 1}),
        create_json_response_handler(),
        failed_response_handler(),
    )
    .await
    .unwrap_err();

    let AiMuxError::ApiCall(detail) = error else {
        panic!("expected ApiCall");
    };
    assert_eq!(detail.status_code, Some(400));
    assert!(
        !detail.message.contains("exceeded maximum size"),
        "size-limit error must not replace the provider error: {}",
        detail.message
    );
    let body = detail.response_body.as_deref().expect("body kept");
    assert!(body.starts_with("{\"error\""), "provider bytes kept");
    assert!(body.ends_with("…(truncated)"), "truncation marked");
    assert!(body.len() < 70 * 1024, "body actually truncated");
}

/// A JSON success body over the configured cap must fail with the
/// size-limit `ApiCallError`, not be buffered in full. Uses
/// `HttpRequest::max_json_response_bytes` (rather than waiting for a
/// multi-hundred-MB fixture) to exercise the same code path a provider hits
/// against the real 64 MiB default.
#[tokio::test]
async fn oversize_json_success_body_is_rejected() {
    let server = MockServer::start().await;
    let huge = format!("{{\"value\":\"{}\"}}", "x".repeat(2048));
    Mock::given(method("POST"))
        .and(path("/call"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(huge),
        )
        .expect(1)
        .mount(&server)
        .await;

    let error = post_json_to_api::<Reply>(
        HttpRequest {
            max_json_response_bytes: Some(1024),
            ..request(format!("{}/call", server.uri()))
        },
        json!({"input": 1}),
        create_json_response_handler(),
        failed_response_handler(),
    )
    .await
    .unwrap_err();

    let AiMuxError::ApiCall(detail) = error else {
        panic!("expected ApiCall");
    };
    assert!(
        detail
            .message
            .contains("exceeded maximum size of 1024 bytes"),
        "expected the size-limit error, got: {}",
        detail.message
    );
}
