//! The two entry points that turn an upstream failure into an `AiMuxError`
//! (`parse_provider_error` for a completed HTTP response, `parse_stream_error`
//! for an SSE `error` event) must agree with each other and must put the
//! structure in *fields*, not in the message string.

use aimux_core::AiMuxError;
use aimux_provider_utils::parse_provider_error;
use aimux_provider_utils::response::{DEFAULT_ERROR_STRUCTURE, parse_stream_error};
use serde_json::json;

/// H4: the error `type` used to be extracted and thrown away (`_error_type`).
/// It is the provider's machine-readable code and now survives as a field.
#[test]
fn parse_provider_error_fills_status_and_provider_code() {
    let body = r#"{"error":{"message":"boom","type":"server_error"}}"#;
    let err = parse_provider_error(500, body, &DEFAULT_ERROR_STRUCTURE);
    let AiMuxError::ApiCall(ref detail) = err else {
        panic!("expected ApiCall, got {err:?}")
    };
    assert_eq!(detail.status_code, Some(500));
    assert_eq!(detail.provider_code.as_deref(), Some("server_error"));
    // The message is the provider's text alone; the status is not written into it.
    assert_eq!(detail.message, "boom");
    assert_eq!(err.to_string(), "API call error: HTTP 500: boom");
    // H1: the status is read from the field, not re-parsed out of the message.
    assert_eq!(err.status_code(), Some(500));
    // The raw body survives alongside the extraction.
    assert_eq!(detail.response_body.as_deref(), Some(body));
}

/// The AI SDK keeps `responseBody` on every error branch, so a consumer never
/// has to ask whether it is populated. Ours does the same: a body that could
/// not be parsed (and therefore *became* the message) is still kept verbatim,
/// and only a genuinely empty body yields `None`.
#[test]
fn response_body_survives_every_parse_outcome() {
    // Unparseable body: it becomes the message *and* stays as raw evidence.
    let body = "upstream exploded";
    let err = parse_provider_error(500, body, &DEFAULT_ERROR_STRUCTURE);
    let AiMuxError::ApiCall(ref detail) = err else {
        panic!("expected ApiCall, got {err:?}")
    };
    assert_eq!(detail.message, body);
    assert_eq!(detail.response_body.as_deref(), Some(body));

    // JSON that the configured path does not resolve to a string: same rule.
    let body = r#"{"code":400,"error":"bad request"}"#;
    let err = parse_provider_error(400, body, &DEFAULT_ERROR_STRUCTURE);
    let AiMuxError::ApiCall(ref detail) = err else {
        panic!("expected ApiCall, got {err:?}")
    };
    assert_eq!(detail.response_body.as_deref(), Some(body));

    // No body at all: nothing to keep.
    let err = parse_provider_error(500, "", &DEFAULT_ERROR_STRUCTURE);
    let AiMuxError::ApiCall(ref detail) = err else {
        panic!("expected ApiCall, got {err:?}")
    };
    assert_eq!(detail.response_body, None);
    assert_eq!(err.to_string(), "API call error: HTTP 500");
}

/// The 429 detail keeps the provider code, so "retry helps"
/// (`rate_limit_exceeded`) and "retry is pointless" (`insufficient_quota`)
/// are finally distinguishable without string matching.
#[test]
fn rate_limited_keeps_the_provider_code() {
    let body = r#"{"error":{"message":"quota exhausted","type":"insufficient_quota"}}"#;
    let err = parse_provider_error(429, body, &DEFAULT_ERROR_STRUCTURE);
    let AiMuxError::ApiCall(ref detail) = err else {
        panic!("expected ApiCall, got {err:?}")
    };
    assert_eq!(detail.provider_code.as_deref(), Some("insufficient_quota"));
    assert_eq!(detail.message, "quota exhausted");
    assert_eq!(detail.response_body.as_deref(), Some(body));
    // Display composes from the fields; no header means no hint — the retry
    // loop's exponential backoff covers it, nothing is fabricated.
    assert_eq!(err.to_string(), "API call error: HTTP 429: quota exhausted");
    assert_eq!(err.retry_after_hint(), None);
}

/// A 401/404 is a `Provider` error classified by the status field — there is
/// no variant to imply anything, and nothing is baked into the message.
#[test]
fn auth_and_not_found_are_classified_by_the_field() {
    let body =
        r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#;
    let err = parse_provider_error(401, body, &DEFAULT_ERROR_STRUCTURE);
    let AiMuxError::ApiCall(ref detail) = err else {
        panic!("expected ApiCall, got {err:?}")
    };
    assert_eq!(
        detail.provider_code.as_deref(),
        Some("invalid_request_error")
    );
    assert_eq!(err.status_code(), Some(401));
    assert_eq!(
        err.to_string(),
        "API call error: HTTP 401: Incorrect API key provided"
    );
    assert!(!err.is_retryable());

    // Empty body: Display composes the status from the field on its own.
    let err = parse_provider_error(404, "", &DEFAULT_ERROR_STRUCTURE);
    assert_eq!(err.to_string(), "API call error: HTTP 404");
    assert_eq!(err.status_code(), Some(404));
}

/// `request_id` set on the detail rides the serialized error.
#[test]
fn request_id_rides_the_detail() {
    let mut err = parse_provider_error(500, "boom", &DEFAULT_ERROR_STRUCTURE);
    if let AiMuxError::ApiCall(d) = &mut err {
        d.request_id = Some("req_abc123".into());
    }
    let AiMuxError::ApiCall(ref detail) = err else {
        panic!("expected ApiCall, got {err:?}")
    };
    assert_eq!(detail.request_id.as_deref(), Some("req_abc123"));
    let json = serde_json::to_string(&err).unwrap();
    let back: AiMuxError = serde_json::from_str(&json).unwrap();
    let AiMuxError::ApiCall(ref detail) = back else {
        panic!("expected ApiCall, got {back:?}")
    };
    assert_eq!(detail.request_id.as_deref(), Some("req_abc123"));
}

#[test]
fn parse_provider_error_fills_rate_limit_fields() {
    let body = r#"{"error":{"message":"slow down","type":"rate_limit_exceeded"}}"#;
    let err = parse_provider_error(429, body, &DEFAULT_ERROR_STRUCTURE);
    // A 429 is an ApiCall error; retryability comes from the stored field,
    // and without a retry-after header there is no hint to store.
    assert_eq!(err.status_code(), Some(429));
    assert!(err.is_retryable());
    assert_eq!(err.retry_after_hint(), None);
}

/// M3: an OpenAI-shaped stream error carries a *string* `code`. Reading it as
/// an HTTP status produced nonsense statuses (and, when it parsed as 429, a
/// hardcoded 1000ms retry). It is a provider code, and there is no status —
/// the response itself was a 200.
#[test]
fn stream_error_string_code_is_a_provider_code_not_a_status() {
    let err = parse_stream_error(&json!({
        "message": "The server had an error processing your request.",
        "type": "server_error",
        "param": null,
        "code": "internal_error",
    }));
    let AiMuxError::ApiCall(ref detail) = err else {
        panic!("expected ApiCall, got {err:?}")
    };
    assert_eq!(detail.provider_code.as_deref(), Some("internal_error"));
    assert_eq!(detail.status_code, None);
    assert_eq!(err.status_code(), None);
}

#[test]
fn stream_error_falls_back_to_type_when_code_is_absent() {
    let err = parse_stream_error(&json!({"message": "boom", "type": "server_error"}));
    let AiMuxError::ApiCall(ref detail) = err else {
        panic!("expected ApiCall, got {err:?}")
    };
    assert_eq!(detail.provider_code.as_deref(), Some("server_error"));
}

/// A numeric `code` in the HTTP range (Google-shaped payloads) *is* a status,
/// and then the stream path maps it exactly like the response path.
#[test]
fn stream_error_numeric_code_is_a_status() {
    for status in [401u16, 404, 429, 500] {
        let stream = parse_stream_error(&json!({"message": "boom", "code": status}));
        let response = parse_provider_error(
            status,
            r#"{"error":{"message":"boom"}}"#,
            &DEFAULT_ERROR_STRUCTURE,
        );
        assert_eq!(
            std::mem::discriminant(&stream),
            std::mem::discriminant(&response),
            "status {status} maps to different variants on the two paths"
        );
        assert_eq!(stream.to_string(), response.to_string(), "status {status}");
    }
}

/// M3: the retry hint is no longer hardcoded when the payload carries one.
#[test]
fn stream_error_reads_the_retry_hint_from_the_payload() {
    let ms = parse_stream_error(&json!({"message": "slow", "code": 429, "retry_after_ms": 250}));
    assert_eq!(ms.retry_after_hint(), Some(250));

    let secs = parse_stream_error(&json!({"message": "slow", "code": 429, "retry_after": 3}));
    assert_eq!(secs.retry_after_hint(), Some(3000));

    // No hint in the payload: none is stored — nothing is fabricated.
    let none = parse_stream_error(&json!({"message": "slow", "code": 429}));
    assert_eq!(none.retry_after_hint(), None);
}

/// Audit §2.2-3: a given status must imply the same retryability whichever
/// entry point produced the error, and the re-mapping helpers must not flip it
/// on the way through.
#[test]
fn retryability_is_invariant_across_the_conversion_paths() {
    for status in [400u16, 401, 403, 404, 422, 429, 500, 502, 503] {
        let response = parse_provider_error(
            status,
            r#"{"error":{"message":"boom","type":"some_code"}}"#,
            &DEFAULT_ERROR_STRUCTURE,
        );
        let stream = parse_stream_error(&json!({"message": "boom", "code": status}));
        assert_eq!(
            response.is_retryable(),
            stream.is_retryable(),
            "status {status}: response path {response:?} vs stream path {stream:?}"
        );
        assert_eq!(response.status_code(), Some(status));
        assert_eq!(stream.status_code(), Some(status));
    }
}

/// §9.1 retry policy: 408/409 join 429/5xx as retryable; a representative
/// ordinary 4xx is not.
#[test]
fn retry_policy_includes_408_and_409() {
    for status in [408u16, 409, 429, 500, 503] {
        assert!(
            parse_provider_error(status, "", &DEFAULT_ERROR_STRUCTURE).is_retryable(),
            "status {status} must be retryable"
        );
    }
    assert!(!parse_provider_error(400, "", &DEFAULT_ERROR_STRUCTURE).is_retryable());
}

/// §9.1 shared non-2xx regression, through the real HTTP loop: every non-2xx
/// goes through the provider error parser (extracted message/code + raw body
/// survive), the observed request id is attached to the same detail, and a
/// hint-less 429 keeps `retry_after_ms=None` — never a fabricated 1000.
#[tokio::test]
async fn http_loop_preserves_parsed_fields_on_429_and_500() {
    use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, RetryConfig, send};
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let body = r#"{"error":{"message":"boom","type":"some_code"}}"#;
    for status in [429u16, 500] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("x-request-id", "req_42")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let request = HttpRequest {
            method: HttpMethod::Post,
            url: server.uri(),
            headers: vec![],
            body: HttpBody::Json(serde_json::json!({})),
            abort_signal: None,
            call_id: None,
            recording_context: None,
        };
        let no_retry = RetryConfig {
            max_retries: 0,
            ..RetryConfig::default()
        };
        let err = send(request, no_retry, &DEFAULT_ERROR_STRUCTURE)
            .await
            .unwrap_err();
        let AiMuxError::ApiCall(ref detail) = err else {
            panic!("expected ApiCall, got {err:?}")
        };
        assert_eq!(detail.status_code, Some(status));
        assert_eq!(detail.message, "boom", "status {status}");
        assert_eq!(detail.provider_code.as_deref(), Some("some_code"));
        assert_eq!(detail.response_body.as_deref(), Some(body));
        assert_eq!(detail.request_id.as_deref(), Some("req_42"));
        assert_eq!(detail.retry_after_ms, None, "no header means no hint");
        assert!(detail.is_retryable);
    }
}

/// A 403 needs no re-mapping helper any more: the observed status *is* the
/// classification, and a message that merely mentions a status is not one.
#[test]
fn observed_status_is_the_classification() {
    let err = parse_provider_error(
        403,
        r#"{"error":{"message":"AccessDenied"}}"#,
        &DEFAULT_ERROR_STRUCTURE,
    );
    assert_eq!(err.status_code(), Some(403));
    assert!(matches!(err, AiMuxError::ApiCall(ref m) if m.message == "AccessDenied"));

    // A 500 whose *message* happens to mention 403 stays a 500.
    let err = parse_provider_error(
        500,
        r#"{"error":{"message":"HTTP 403: not really"}}"#,
        &DEFAULT_ERROR_STRUCTURE,
    );
    assert_eq!(err.status_code(), Some(500));
}
