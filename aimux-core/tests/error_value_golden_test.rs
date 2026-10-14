//! Golden snapshots of the `error_value` payload — the `name`-tagged serde
//! JSON (the AI SDK's error names) that ships across the FFI boundary to all
//! eight languages.
//!
//! These assert the *exact* JSON for every variant. The payload is a public
//! cross-language contract: a field rename, a variant rename or a shape change
//! breaks every binding, so it must break this test first.

use aimux_core::error::ToolChoiceViolationError;
use aimux_core::result::GenerateContent;
use aimux_core::tool::LanguageModelV4ToolChoice;
use aimux_core::types::FinishReasonUnified;
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
        (
            AiMuxError::ToolCallNotFoundForApproval {
                tool_call_id: "call-1".into(),
                approval_id: "approval-1".into(),
            },
            r#"{"name":"AI_ToolCallNotFoundForApprovalError","toolCallId":"call-1","approvalId":"approval-1"}"#,
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
            AiMuxError::MissingToolResults {
                tool_call_ids: vec!["call-1".into()],
            },
            r#"{"name":"AI_MissingToolResultsError","toolCallIds":["call-1"]}"#,
        ),
        (
            AiMuxError::ToolChoiceViolation(Box::new(ToolChoiceViolationError {
                tool_choice: LanguageModelV4ToolChoice::Tool {
                    tool_name: "weather".into(),
                },
                finish_reason: FinishReasonUnified::Stop,
                provider: "openai.chat".into(),
                model_id: "gpt-4o".into(),
                content: vec![GenerateContent::Text {
                    text: "sunny".into(),
                    provider_metadata: None,
                }],
            })),
            r#"{"name":"AI_ToolChoiceViolationError","toolChoice":{"type":"tool","toolName":"weather"},"finishReason":"stop","provider":"openai.chat","modelId":"gpt-4o","content":[{"type":"text","text":"sunny"}]}"#,
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

/// The AI SDK's `ToolCallNotFoundForApprovalError` message, verbatim
/// (ai 7.0.127 `src/error/tool-call-not-found-for-approval-error.ts:22`).
#[test]
fn tool_call_not_found_for_approval_message_matches_upstream() {
    let err = AiMuxError::ToolCallNotFoundForApproval {
        tool_call_id: "call-1".into(),
        approval_id: "approval-1".into(),
    };
    assert_eq!(
        err.to_string(),
        r#"Tool call "call-1" not found for approval request "approval-1"."#
    );
}

/// The AI SDK's `MissingToolResultsError` message, verbatim
/// (ai 7.0.127 `src/error/missing-tool-result-error.ts:15-20`).
#[test]
fn missing_tool_results_message_matches_upstream() {
    let error = |ids: &[&str]| AiMuxError::MissingToolResults {
        tool_call_ids: ids.iter().map(|id| (*id).to_owned()).collect(),
    };
    assert_eq!(
        error(&["call-1"]).to_string(),
        "Tool result is missing for tool call call-1."
    );
    assert_eq!(
        error(&["call-1", "call-2"]).to_string(),
        "Tool results are missing for tool calls call-1, call-2."
    );
}

/// The AI SDK's `ToolChoiceViolationError` messages, verbatim
/// (ai 7.0.127 `src/error/tool-choice-violation-error.ts:56-58`).
#[test]
fn tool_choice_violation_message_matches_upstream() {
    let error = |tool_choice| {
        AiMuxError::ToolChoiceViolation(Box::new(ToolChoiceViolationError {
            tool_choice,
            finish_reason: FinishReasonUnified::Stop,
            provider: "p".into(),
            model_id: "m".into(),
            content: vec![],
        }))
    };
    assert_eq!(
        error(LanguageModelV4ToolChoice::Required).to_string(),
        "Model response did not contain a tool call even though tool choice was required."
    );
    assert_eq!(
        error(LanguageModelV4ToolChoice::Tool {
            tool_name: "weather".into()
        })
        .to_string(),
        "Model response did not contain a call to the required tool 'weather'."
    );
}

/// The variant set is a cross-language contract of its own: bindings switch on
/// the wire tag, and the FFI / Node / Python error mappings match on every
/// variant. Adding or removing one is a breaking change.
/// Compile-time pin: adding an `AiMuxError` variant breaks this match, so
/// the wire-tag list below and every binding's switch must be updated in the
/// same change (the runtime assertion alone cannot see additions).
#[allow(dead_code)]
fn variant_addition_breaks_this_match(error: &AiMuxError) {
    match error {
        AiMuxError::ApiCall(_)
        | AiMuxError::Retry(_)
        | AiMuxError::JsonParse(_)
        | AiMuxError::InvalidResponseData(_)
        | AiMuxError::ToolCallNotFoundForApproval { .. }
        | AiMuxError::NoSuchTool { .. }
        | AiMuxError::InvalidToolInput { .. }
        | AiMuxError::ToolCallRepair { .. }
        | AiMuxError::InvalidArgument(_)
        | AiMuxError::InvalidPrompt(_)
        | AiMuxError::LoadApiKey { .. }
        | AiMuxError::LoadSetting { .. }
        | AiMuxError::MissingToolResults { .. }
        | AiMuxError::ToolChoiceViolation(_)
        | AiMuxError::NoOutputGenerated(_)
        | AiMuxError::TokenExpired(_)
        | AiMuxError::UnsupportedFunctionality(_)
        | AiMuxError::NoSuchModel { .. }
        | AiMuxError::NoSuchProvider { .. }
        | AiMuxError::Timeout(_)
        | AiMuxError::Aborted(_)
        | AiMuxError::Other(_) => {}
    }
}

#[test]
fn variant_set_is_exactly_the_wire_tag_list() {
    let all = [
        AiMuxError::ApiCall(Box::new(api_error("x"))),
        AiMuxError::Retry(RetryError {
            reason: RetryErrorReason::MaxRetriesExceeded,
            errors: vec![AiMuxError::ApiCall(Box::new(api_error("x")))],
        }),
        AiMuxError::JsonParse("x".into()),
        AiMuxError::InvalidResponseData("x".into()),
        AiMuxError::ToolCallNotFoundForApproval {
            tool_call_id: "x".into(),
            approval_id: "x".into(),
        },
        AiMuxError::NoSuchTool {
            tool_name: "x".into(),
            available_tools: None,
            tool_input: None,
        },
        AiMuxError::InvalidToolInput {
            tool_name: "x".into(),
            tool_input: "{}".into(),
            cause: "x".into(),
        },
        AiMuxError::ToolCallRepair {
            original_error: Box::new(AiMuxError::NoSuchTool {
                tool_name: "x".into(),
                available_tools: None,
                tool_input: None,
            }),
            cause: Box::new(AiMuxError::Other("x".into())),
        },
        AiMuxError::InvalidArgument("x".into()),
        AiMuxError::InvalidPrompt("x".into()),
        AiMuxError::LoadApiKey {
            env_var: "X".into(),
            description: "x".into(),
        },
        AiMuxError::LoadSetting {
            env_var: "X".into(),
            name: "x".into(),
        },
        AiMuxError::MissingToolResults {
            tool_call_ids: vec!["x".into()],
        },
        AiMuxError::ToolChoiceViolation(Box::new(ToolChoiceViolationError {
            tool_choice: LanguageModelV4ToolChoice::Required,
            finish_reason: FinishReasonUnified::Stop,
            provider: "x".into(),
            model_id: "x".into(),
            content: vec![],
        })),
        AiMuxError::NoOutputGenerated("x".into()),
        AiMuxError::TokenExpired("x".into()),
        AiMuxError::UnsupportedFunctionality("x".into()),
        AiMuxError::NoSuchModel {
            model_id: "x".into(),
            model_type: String::new(),
        },
        AiMuxError::NoSuchProvider {
            provider_id: "x".into(),
            model_id: "x".into(),
            model_type: String::new(),
            available_providers: vec![],
        },
        AiMuxError::Timeout("x".into()),
        AiMuxError::Aborted("request aborted".into()),
        AiMuxError::Other("x".into()),
    ];
    // The wire tag is the AI SDK's error name, under `name`.
    let mut names: Vec<String> = all
        .iter()
        .map(|e| match serde_json::to_value(e).unwrap() {
            serde_json::Value::Object(m) => m["name"].as_str().unwrap().to_owned(),
            v => panic!("unexpected error_value shape: {v}"),
        })
        .collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names,
        [
            "AI_APICallError",
            "AI_Error",
            "AI_InvalidArgumentError",
            "AI_InvalidPromptError",
            "AI_InvalidResponseDataError",
            "AI_InvalidToolInputError",
            "AI_JSONParseError",
            "AI_LoadAPIKeyError",
            "AI_LoadSettingError",
            "AI_MissingToolResultsError",
            "AI_NoOutputGeneratedError",
            "AI_NoSuchModelError",
            "AI_NoSuchProviderError",
            "AI_NoSuchToolError",
            "AI_RetryError",
            "AI_TokenExpiredError",
            "AI_ToolCallNotFoundForApprovalError",
            "AI_ToolCallRepairError",
            "AI_ToolChoiceViolationError",
            "AI_UnsupportedFunctionalityError",
            "AbortError",
            "TimeoutError",
        ],
        "variant set changed"
    );
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
