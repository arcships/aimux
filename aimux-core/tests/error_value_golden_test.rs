//! Golden snapshots of the `error_value` payload — the externally-tagged serde
//! JSON that PR #91 ships across the FFI boundary to all eight languages.
//!
//! These assert the *exact* JSON for every variant. The payload is a public
//! cross-language contract: a field rename, a variant rename or a shape change
//! breaks every binding, so it must break this test first.

use aimux_core::{AiMuxError, ApiCallError, RetryError, RetryErrorReason};

fn api_error(message: &str) -> ApiCallError {
    ApiCallError::new(message, "https://example.test/v1", serde_json::json!({}))
}

fn golden(err: &AiMuxError, expected: &str) {
    let json = serde_json::to_string(err).unwrap();
    assert_eq!(json, expected, "error_value changed for {err:?}");
    // Every payload must deserialize back into the same variant.
    let back: AiMuxError = serde_json::from_str(&json).unwrap();
    assert_eq!(std::mem::discriminant(&back), std::mem::discriminant(err));
    assert_eq!(back.to_string(), err.to_string());
}

/// `ApiCall` carries the full `ApiCallError` field set. The Rust payload is
/// boxed only to keep the enum compact; the classification is `status_code`
/// field and the retry verdict the stored `is_retryable`.
#[test]
fn error_value_snapshots_api_call_shapes() {
    golden(
        &AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: Some(500),
            provider_code: Some("server_error".into()),
            is_retryable: true,
            ..api_error("boom")
        })),
        r#"{"ApiCall":{"url":"https://example.test/v1","request_body_values":{},"status_code":500,"provider_code":"server_error","message":"boom","response_body":null,"response_headers":null,"data":null,"is_retryable":true}}"#,
    );
    golden(
        &AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: Some(500),
            provider_code: Some("server_error".into()),
            response_body: Some(r#"{"error":{"message":"boom","type":"server_error"}}"#.into()),
            is_retryable: true,
            ..api_error("boom")
        })),
        r#"{"ApiCall":{"url":"https://example.test/v1","request_body_values":{},"status_code":500,"provider_code":"server_error","message":"boom","response_body":"{\"error\":{\"message\":\"boom\",\"type\":\"server_error\"}}","response_headers":null,"data":null,"is_retryable":true}}"#,
    );
    // A transport failure (no response arrived): no status, retryable —
    // exactly the AI SDK's handleFetchError shape.
    golden(
        &AiMuxError::ApiCall(Box::new(ApiCallError {
            is_retryable: true,
            ..api_error("connection reset")
        })),
        r#"{"ApiCall":{"url":"https://example.test/v1","request_body_values":{},"status_code":null,"provider_code":null,"message":"connection reset","response_body":null,"response_headers":null,"data":null,"is_retryable":true}}"#,
    );
    // A 429 is an ApiCall error whose classification is the status field —
    // there is no RateLimited variant; the hint remains in response headers.
    golden(
        &AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: Some(429),
            provider_code: Some("rate_limit_exceeded".into()),
            response_headers: Some(std::collections::HashMap::from([(
                "retry-after-ms".into(),
                "2500".into(),
            )])),
            is_retryable: true,
            ..api_error("slow down")
        })),
        r#"{"ApiCall":{"url":"https://example.test/v1","request_body_values":{},"status_code":429,"provider_code":"rate_limit_exceeded","message":"slow down","response_body":null,"response_headers":{"retry-after-ms":"2500"},"data":null,"is_retryable":true}}"#,
    );
    golden(
        &AiMuxError::TokenExpired("expired".into()),
        r#"{"TokenExpired":"expired"}"#,
    );
}

#[test]
fn error_value_snapshot_retry_history() {
    golden(
        &AiMuxError::Retry(RetryError {
            reason: RetryErrorReason::MaxRetriesExceeded,
            errors: vec![
                AiMuxError::ApiCall(Box::new(ApiCallError {
                    is_retryable: true,
                    ..api_error("first")
                })),
                AiMuxError::ApiCall(Box::new(ApiCallError {
                    is_retryable: true,
                    ..api_error("second")
                })),
            ],
        }),
        r#"{"Retry":{"reason":"maxRetriesExceeded","errors":[{"ApiCall":{"url":"https://example.test/v1","request_body_values":{},"status_code":null,"provider_code":null,"message":"first","response_body":null,"response_headers":null,"data":null,"is_retryable":true}},{"ApiCall":{"url":"https://example.test/v1","request_body_values":{},"status_code":null,"provider_code":null,"message":"second","response_body":null,"response_headers":null,"data":null,"is_retryable":true}}]}}"#,
    );
}

#[test]
fn error_value_snapshots_plain_variants() {
    for (err, expected) in [
        (
            AiMuxError::JsonParse("bad json".into()),
            r#"{"JsonParse":"bad json"}"#,
        ),
        (
            AiMuxError::InvalidResponseData("eof".into()),
            r#"{"InvalidResponseData":"eof"}"#,
        ),
        (
            AiMuxError::Tool("tool blew up".into()),
            r#"{"Tool":"tool blew up"}"#,
        ),
        (
            AiMuxError::InvalidArgument("bad arg".into()),
            r#"{"InvalidArgument":"bad arg"}"#,
        ),
        (
            AiMuxError::InvalidPrompt("bad prompt".into()),
            r#"{"InvalidPrompt":"bad prompt"}"#,
        ),
        (
            AiMuxError::TokenExpired("expired".into()),
            r#"{"TokenExpired":"expired"}"#,
        ),
        (
            AiMuxError::UnsupportedFunctionality("no audio".into()),
            r#"{"UnsupportedFunctionality":"no audio"}"#,
        ),
        (
            AiMuxError::NoSuchModel {
                model_id: "gpt-9".into(),
                model_type: "languageModel".into(),
            },
            r#"{"NoSuchModel":{"model_id":"gpt-9","model_type":"languageModel"}}"#,
        ),
        (
            AiMuxError::NoSuchProvider {
                provider_id: "acme".into(),
            },
            r#"{"NoSuchProvider":{"provider_id":"acme"}}"#,
        ),
        (
            AiMuxError::Timeout("total timeout".into()),
            r#"{"Timeout":"total timeout"}"#,
        ),
        (
            AiMuxError::Aborted("request aborted".into()),
            r#"{"Aborted":"request aborted"}"#,
        ),
        (AiMuxError::Other("misc".into()), r#"{"Other":"misc"}"#),
    ] {
        golden(&err, expected);
    }
}

/// The variant set is a cross-language contract of its own: bindings switch on
/// the wire tag. Adding or removing one is a breaking change (14 variants —
/// the per-status avatars `Auth`/`ModelNotFound`/`RateLimited` are gone, and
/// `Http`/`Provider` folded into `ApiCall`: a failed exchange is an `ApiCall`
/// error classified by `status_code`, transport failures included).
#[test]
fn variant_set_is_exactly_fourteen() {
    let all = [
        AiMuxError::ApiCall(Box::new(api_error("x"))),
        AiMuxError::Retry(RetryError {
            reason: RetryErrorReason::MaxRetriesExceeded,
            errors: vec![AiMuxError::ApiCall(Box::new(api_error("x")))],
        }),
        AiMuxError::JsonParse("x".into()),
        AiMuxError::InvalidResponseData("x".into()),
        AiMuxError::Tool("x".into()),
        AiMuxError::InvalidArgument("x".into()),
        AiMuxError::InvalidPrompt("x".into()),
        AiMuxError::TokenExpired("x".into()),
        AiMuxError::UnsupportedFunctionality("x".into()),
        AiMuxError::NoSuchModel {
            model_id: "x".into(),
            model_type: String::new(),
        },
        AiMuxError::NoSuchProvider {
            provider_id: "x".into(),
        },
        AiMuxError::Timeout("x".into()),
        AiMuxError::Aborted("request aborted".into()),
        AiMuxError::Other("x".into()),
    ];
    // The wire tag IS the variant name (externally-tagged serde JSON).
    let mut names: Vec<String> = all
        .iter()
        .map(|e| match serde_json::to_value(e).unwrap() {
            serde_json::Value::Object(m) => m.into_iter().next().unwrap().0,
            serde_json::Value::String(s) => s, // unit variants (Aborted)
            v => panic!("unexpected error_value shape: {v}"),
        })
        .collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names,
        [
            "Aborted",
            "ApiCall",
            "InvalidArgument",
            "InvalidPrompt",
            "InvalidResponseData",
            "JsonParse",
            "NoSuchModel",
            "NoSuchProvider",
            "Other",
            "Retry",
            "Timeout",
            "TokenExpired",
            "Tool",
            "UnsupportedFunctionality",
        ],
        "variant set changed"
    );
}

/// Request context is required. Payloads from the pre-context schema are
/// deliberately rejected instead of silently fabricating an empty URL/body.
#[test]
fn api_call_requires_request_context() {
    let old = r#"{"ApiCall":{"message":"boom"}}"#;
    assert!(serde_json::from_str::<AiMuxError>(old).is_err());

    // The removed variants no longer deserialize — a deliberate breaking
    // change pinned here so it cannot happen silently a second time.
    assert!(
        serde_json::from_str::<AiMuxError>(r#"{"RateLimited":{"retry_after_ms":5000}}"#).is_err()
    );
    assert!(serde_json::from_str::<AiMuxError>(r#"{"Auth":"bad key"}"#).is_err());
    assert!(serde_json::from_str::<AiMuxError>(r#"{"Http":"connection reset"}"#).is_err());
    assert!(serde_json::from_str::<AiMuxError>(r#"{"Provider":{"message":"boom"}}"#).is_err());
    // NoSuchModel went from a plain string to a struct.
    assert!(serde_json::from_str::<AiMuxError>(r#"{"NoSuchModel":"gpt-9"}"#).is_err());
    // The renamed variants' OLD wire names no longer deserialize:
    // Json/Stream/Unsupported/UnknownProvider became
    // JsonParse/InvalidResponseData/UnsupportedFunctionality/NoSuchProvider.
    assert!(serde_json::from_str::<AiMuxError>(r#"{"Json":"bad json"}"#).is_err());
    assert!(serde_json::from_str::<AiMuxError>(r#"{"Stream":"eof"}"#).is_err());
    assert!(serde_json::from_str::<AiMuxError>(r#"{"Unsupported":"no audio"}"#).is_err());
    assert!(serde_json::from_str::<AiMuxError>(r#"{"UnknownProvider":"acme"}"#).is_err());
    assert!(
        serde_json::from_str::<AiMuxError>(
            r#"{"UnknownProvider":{"provider_id":"acme","available":[]}}"#
        )
        .is_err()
    );
}

/// The status lives in the field and *only* there. `Display` composes the
/// familiar `HTTP {status}: ` text at print time, so the human-facing string is
/// unchanged while no consumer has to parse it back out (H1).
#[test]
fn status_lives_in_the_field_and_display_composes_it() {
    let err = AiMuxError::ApiCall(Box::new(ApiCallError {
        status_code: Some(429),
        ..api_error("quota exceeded")
    }));
    assert_eq!(err.status_code(), Some(429));
    let AiMuxError::ApiCall(ref detail) = err else {
        panic!("expected ApiCall, got {err:?}")
    };
    assert_eq!(
        detail.message, "quota exceeded",
        "the stored message must not carry an HTTP prefix"
    );
    assert_eq!(err.to_string(), "API call error: HTTP 429: quota exceeded");

    // Without a status there is nothing to compose.
    let err = AiMuxError::ApiCall(Box::new(api_error("plain failure")));
    assert_eq!(err.to_string(), "API call error: plain failure");
}

/// `provider_code` is machine-readable and stays out of the human text.
#[test]
fn provider_code_is_readable_and_not_displayed() {
    let err = AiMuxError::ApiCall(Box::new(ApiCallError {
        status_code: Some(400),
        provider_code: Some("invalid_request".into()),
        ..api_error("bad input")
    }));
    let AiMuxError::ApiCall(ref detail) = err else {
        panic!("expected ApiCall, got {err:?}")
    };
    assert_eq!(detail.provider_code.as_deref(), Some("invalid_request"));
    assert_eq!(err.to_string(), "API call error: HTTP 400: bad input");
}

/// The human-readable Display strings are asserted across the test suite and by
/// downstream bindings; the field work must not move them.
#[test]
fn display_strings_are_unchanged_by_the_field_shape() {
    assert_eq!(
        AiMuxError::ApiCall(Box::new(api_error("boom"))).to_string(),
        "API call error: boom"
    );
    assert_eq!(
        AiMuxError::ApiCall(Box::new(ApiCallError {
            is_retryable: true,
            ..api_error("reset")
        }))
        .to_string(),
        "API call error: reset"
    );
    assert_eq!(
        AiMuxError::TokenExpired("expired".into()).to_string(),
        "token expired: expired"
    );
}

/// `AiMuxError` rides in every `Result<T, AiMuxError>`. `ApiCall` carries its
/// detail boxed; this guard pins the
/// size so growth is a deliberate decision, and keeps it under clippy's
/// `result_large_err` threshold (128 bytes).
#[test]
fn error_size_is_pinned() {
    assert!(
        std::mem::size_of::<AiMuxError>() <= 128,
        "AiMuxError grew to {} bytes — consider boxing the payload",
        std::mem::size_of::<AiMuxError>()
    );
}
