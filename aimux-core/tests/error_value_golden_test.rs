//! Golden snapshots of the `error_value` payload — the `name`-tagged serde
//! JSON (the AI SDK's error names) that ships across the FFI boundary to all
//! eight languages.
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
        r#"{"name":"AI_APICallError","url":"https://example.test/v1","requestBodyValues":{},"statusCode":500,"providerCode":"server_error","message":"boom","isRetryable":true}"#,
    );
    golden(
        &AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: Some(500),
            provider_code: Some("server_error".into()),
            response_body: Some(r#"{"error":{"message":"boom","type":"server_error"}}"#.into()),
            is_retryable: true,
            ..api_error("boom")
        })),
        r#"{"name":"AI_APICallError","url":"https://example.test/v1","requestBodyValues":{},"statusCode":500,"providerCode":"server_error","message":"boom","responseBody":"{\"error\":{\"message\":\"boom\",\"type\":\"server_error\"}}","isRetryable":true}"#,
    );
    // A transport failure (no response arrived): no status, retryable —
    // exactly the AI SDK's handleFetchError shape.
    golden(
        &AiMuxError::ApiCall(Box::new(ApiCallError {
            is_retryable: true,
            ..api_error("connection reset")
        })),
        r#"{"name":"AI_APICallError","url":"https://example.test/v1","requestBodyValues":{},"message":"connection reset","isRetryable":true}"#,
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
        r#"{"name":"AI_APICallError","url":"https://example.test/v1","requestBodyValues":{},"statusCode":429,"providerCode":"rate_limit_exceeded","message":"slow down","responseHeaders":{"retry-after-ms":"2500"},"isRetryable":true}"#,
    );
    golden(
        &AiMuxError::TokenExpired("expired".into()),
        r#"{"name":"AI_TokenExpiredError","message":"expired"}"#,
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
        r#"{"name":"AI_RetryError","reason":"maxRetriesExceeded","errors":[{"name":"AI_APICallError","url":"https://example.test/v1","requestBodyValues":{},"message":"first","isRetryable":true},{"name":"AI_APICallError","url":"https://example.test/v1","requestBodyValues":{},"message":"second","isRetryable":true}]}"#,
    );
}

#[test]
fn error_value_snapshots_plain_variants() {
    for (err, expected) in [
        (
            AiMuxError::JsonParse("bad json".into()),
            r#"{"name":"AI_JSONParseError","message":"bad json"}"#,
        ),
        (
            AiMuxError::InvalidResponseData("eof".into()),
            r#"{"name":"AI_InvalidResponseDataError","message":"eof"}"#,
        ),
        // NoSuchTool is pinned in both shapes: `skip_serializing_if` makes
        // the wire payload vary with `available_tools`.
        (
            AiMuxError::NoSuchTool {
                tool_name: "weathr".into(),
                available_tools: Some(vec!["weather".into(), "search".into()]),
                tool_input: Some(r#""hello""#.into()),
            },
            r#"{"name":"AI_NoSuchToolError","toolName":"weathr","availableTools":["weather","search"],"toolInput":"\"hello\""}"#,
        ),
        (
            AiMuxError::NoSuchTool {
                tool_name: "weathr".into(),
                available_tools: None,
                tool_input: None,
            },
            r#"{"name":"AI_NoSuchToolError","toolName":"weathr"}"#,
        ),
        (
            AiMuxError::InvalidToolInput {
                tool_name: "weather".into(),
                tool_input: "{".into(),
                cause: "JSON parsing failed".into(),
            },
            r#"{"name":"AI_InvalidToolInputError","toolName":"weather","toolInput":"{","cause":"JSON parsing failed"}"#,
        ),
        (
            AiMuxError::ToolCallRepair {
                original_error: Box::new(AiMuxError::NoSuchTool {
                    tool_name: "weathr".into(),
                    available_tools: None,
                    tool_input: None,
                }),
                cause: Box::new(AiMuxError::Other("repair model failed".into())),
            },
            r#"{"name":"AI_ToolCallRepairError","originalError":{"name":"AI_NoSuchToolError","toolName":"weathr"},"cause":{"name":"AI_Error","message":"repair model failed"}}"#,
        ),
        (
            AiMuxError::InvalidArgument("bad arg".into()),
            r#"{"name":"AI_InvalidArgumentError","message":"bad arg"}"#,
        ),
        (
            AiMuxError::InvalidPrompt("bad prompt".into()),
            r#"{"name":"AI_InvalidPromptError","message":"bad prompt"}"#,
        ),
        (
            AiMuxError::TokenExpired("expired".into()),
            r#"{"name":"AI_TokenExpiredError","message":"expired"}"#,
        ),
        (
            AiMuxError::UnsupportedFunctionality("no audio".into()),
            r#"{"name":"AI_UnsupportedFunctionalityError","message":"no audio"}"#,
        ),
        (
            AiMuxError::LoadApiKey {
                env_var: "OPENAI_API_KEY".into(),
                description: "OpenAI".into(),
            },
            r#"{"name":"AI_LoadAPIKeyError","envVar":"OPENAI_API_KEY","description":"OpenAI"}"#,
        ),
        (
            AiMuxError::LoadSetting {
                env_var: "AWS_REGION".into(),
                name: "region".into(),
            },
            r#"{"name":"AI_LoadSettingError","envVar":"AWS_REGION","settingName":"region"}"#,
        ),
        (
            AiMuxError::NoSuchModel {
                model_id: "gpt-9".into(),
                model_type: "languageModel".into(),
            },
            r#"{"name":"AI_NoSuchModelError","modelId":"gpt-9","modelType":"languageModel"}"#,
        ),
        (
            AiMuxError::NoSuchProvider {
                provider_id: "acme".into(),
                model_id: "acme".into(),
                model_type: "languageModel".into(),
                available_providers: vec!["other".into()],
            },
            r#"{"name":"AI_NoSuchProviderError","providerId":"acme","modelId":"acme","modelType":"languageModel","availableProviders":["other"]}"#,
        ),
        (
            AiMuxError::Timeout("total timeout".into()),
            r#"{"name":"TimeoutError","message":"total timeout"}"#,
        ),
        (
            AiMuxError::Aborted("request aborted".into()),
            r#"{"name":"AbortError","message":"request aborted"}"#,
        ),
        (
            AiMuxError::Other("misc".into()),
            r#"{"name":"AI_Error","message":"misc"}"#,
        ),
    ] {
        golden(&err, expected);
    }
}

/// Request context is required: a payload without `url` / `requestBodyValues`
/// is rejected instead of silently fabricating an empty URL/body.
#[test]
fn api_call_requires_request_context() {
    let incomplete = r#"{"name":"AI_APICallError","message":"boom","isRetryable":false}"#;
    assert!(serde_json::from_str::<AiMuxError>(incomplete).is_err());
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
