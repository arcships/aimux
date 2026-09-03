//! Golden snapshots of the `error_value` payload — the externally-tagged serde
//! JSON that PR #91 ships across the FFI boundary to all eight languages.
//!
//! These assert the *exact* JSON for every variant. The payload is a public
//! cross-language contract: a field rename, a variant rename or a shape change
//! breaks every binding, so it must break this test first.

use aimux_core::{AiMuxError, ApiCallError};

fn golden(err: &AiMuxError, expected: &str) {
    let json = serde_json::to_string(err).unwrap();
    assert_eq!(json, expected, "error_value changed for {err:?}");
    // Every payload must deserialize back into the same variant.
    let back: AiMuxError = serde_json::from_str(&json).unwrap();
    assert_eq!(std::mem::discriminant(&back), std::mem::discriminant(err));
    assert_eq!(back.to_string(), err.to_string());
}

/// `ApiCall` carries the full `ApiCallError` field set (unboxed, like
/// async-openai's `ApiError`); the classification is the `status_code`
/// field and the retry verdict the stored `is_retryable`.
#[test]
fn error_value_snapshots_api_call_shapes() {
    golden(
        &AiMuxError::ApiCall(ApiCallError {
            status_code: Some(500),
            provider_code: Some("server_error".into()),
            message: "boom".into(),
            response_body: None,
            is_retryable: true,
            ..Default::default()
        }),
        r#"{"ApiCall":{"status_code":500,"provider_code":"server_error","message":"boom","response_body":null,"request_id":null,"retry_after_ms":null,"is_retryable":true}}"#,
    );
    golden(
        &AiMuxError::ApiCall(ApiCallError {
            status_code: Some(500),
            provider_code: Some("server_error".into()),
            message: "boom".into(),
            response_body: Some(r#"{"error":{"message":"boom","type":"server_error"}}"#.into()),
            is_retryable: true,
            ..Default::default()
        }),
        r#"{"ApiCall":{"status_code":500,"provider_code":"server_error","message":"boom","response_body":"{\"error\":{\"message\":\"boom\",\"type\":\"server_error\"}}","request_id":null,"retry_after_ms":null,"is_retryable":true}}"#,
    );
    // A transport failure (no response arrived): no status, retryable —
    // exactly the AI SDK's handleFetchError shape.
    golden(
        &AiMuxError::ApiCall(ApiCallError {
            message: "connection reset".into(),
            is_retryable: true,
            ..Default::default()
        }),
        r#"{"ApiCall":{"status_code":null,"provider_code":null,"message":"connection reset","response_body":null,"request_id":null,"retry_after_ms":null,"is_retryable":true}}"#,
    );
    // A 429 is an ApiCall error whose classification is the status field —
    // there is no RateLimited variant; the hint rides `retry_after_ms`.
    golden(
        &AiMuxError::ApiCall(ApiCallError {
            status_code: Some(429),
            provider_code: Some("rate_limit_exceeded".into()),
            message: "slow down".into(),
            retry_after_ms: Some(2500),
            is_retryable: true,
            ..Default::default()
        }),
        r#"{"ApiCall":{"status_code":429,"provider_code":"rate_limit_exceeded","message":"slow down","response_body":null,"request_id":null,"retry_after_ms":2500,"is_retryable":true}}"#,
    );
    golden(
        &AiMuxError::TokenExpired("expired".into()),
        r#"{"TokenExpired":"expired"}"#,
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
        // NoSuchTool is pinned in both shapes: `skip_serializing_if` makes
        // the wire payload vary with `available_tools`.
        (
            AiMuxError::NoSuchTool {
                tool_name: "weathr".into(),
                available_tools: Some(vec!["weather".into(), "search".into()]),
            },
            r#"{"NoSuchTool":{"tool_name":"weathr","available_tools":["weather","search"]}}"#,
        ),
        (
            AiMuxError::NoSuchTool {
                tool_name: "weathr".into(),
                available_tools: None,
            },
            r#"{"NoSuchTool":{"tool_name":"weathr"}}"#,
        ),
        (
            AiMuxError::InvalidToolInput {
                tool_name: "weather".into(),
                tool_input: "{".into(),
                cause: "JSON parsing failed".into(),
            },
            r#"{"InvalidToolInput":{"tool_name":"weather","tool_input":"{","cause":"JSON parsing failed"}}"#,
        ),
        (
            AiMuxError::ToolCallRepair {
                original_error: Box::new(AiMuxError::NoSuchTool {
                    tool_name: "weathr".into(),
                    available_tools: None,
                }),
                cause: Box::new(AiMuxError::Other("repair model failed".into())),
            },
            r#"{"ToolCallRepair":{"original_error":{"NoSuchTool":{"tool_name":"weathr"}},"cause":{"Other":"repair model failed"}}}"#,
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
        (AiMuxError::Aborted, r#""Aborted""#),
        (AiMuxError::Other("misc".into()), r#"{"Other":"misc"}"#),
    ] {
        golden(&err, expected);
    }
}

/// The variant set is a cross-language contract of its own: bindings switch on
/// the wire tag. Adding or removing one is a breaking change (15 variants —
/// the per-status avatars `Auth`/`ModelNotFound`/`RateLimited` are gone,
/// `Http`/`Provider` folded into `ApiCall` (a failed exchange is an `ApiCall`
/// error classified by `status_code`, transport failures included), and the
/// legacy catch-all `Tool` — never constructed anywhere — is replaced by the
/// typed `NoSuchTool`/`InvalidToolInput`/`ToolCallRepair`).
/// Compile-time pin: adding an `AiMuxError` variant breaks this match, so
/// the wire-tag list below and every binding's switch must be updated in the
/// same change (the runtime assertion alone cannot see additions).
#[allow(dead_code)]
fn variant_addition_breaks_this_match(error: &AiMuxError) {
    match error {
        AiMuxError::ApiCall(_)
        | AiMuxError::JsonParse(_)
        | AiMuxError::InvalidResponseData(_)
        | AiMuxError::NoSuchTool { .. }
        | AiMuxError::InvalidToolInput { .. }
        | AiMuxError::ToolCallRepair { .. }
        | AiMuxError::InvalidArgument(_)
        | AiMuxError::InvalidPrompt(_)
        | AiMuxError::TokenExpired(_)
        | AiMuxError::UnsupportedFunctionality(_)
        | AiMuxError::NoSuchModel { .. }
        | AiMuxError::NoSuchProvider { .. }
        | AiMuxError::Timeout(_)
        | AiMuxError::Aborted
        | AiMuxError::Other(_) => {}
    }
}

#[test]
fn variant_set_is_exactly_fifteen() {
    let all = [
        AiMuxError::ApiCall(ApiCallError {
            message: "x".into(),
            ..Default::default()
        }),
        AiMuxError::ApiCall(ApiCallError {
            is_retryable: true,
            ..Default::default()
        }),
        AiMuxError::JsonParse("x".into()),
        AiMuxError::InvalidResponseData("x".into()),
        AiMuxError::NoSuchTool {
            tool_name: "x".into(),
            available_tools: None,
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
            }),
            cause: Box::new(AiMuxError::Other("x".into())),
        },
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
        AiMuxError::Aborted,
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
            "InvalidToolInput",
            "JsonParse",
            "NoSuchModel",
            "NoSuchProvider",
            "NoSuchTool",
            "Other",
            "Timeout",
            "TokenExpired",
            "ToolCallRepair",
            "UnsupportedFunctionality",
        ],
        "variant set changed"
    );
}

/// Field additions must stay *deserialization*-compatible: a payload written
/// before the structured fields existed still loads (the `#[serde(default)]`
/// contract, M6). Serialization always emits the full current shape.
#[test]
fn structured_fields_deserialize_from_pre_field_payloads() {
    let old = r#"{"ApiCall":{"message":"boom"}}"#;
    let err: AiMuxError = serde_json::from_str(old).unwrap();
    let AiMuxError::ApiCall(ref detail) = err else {
        panic!("expected ApiCall, got {err:?}")
    };
    assert_eq!(detail.status_code, None);
    assert_eq!(detail.provider_code, None);
    assert_eq!(detail.message, "boom");
    assert!(!detail.is_retryable);
    assert_eq!(err.status_code(), None);

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
    let err = AiMuxError::ApiCall(ApiCallError {
        status_code: Some(429),
        message: "quota exceeded".into(),
        ..Default::default()
    });
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
    let err = AiMuxError::ApiCall(ApiCallError {
        message: "plain failure".into(),
        ..Default::default()
    });
    assert_eq!(err.to_string(), "API call error: plain failure");
}

/// `provider_code` is machine-readable and stays out of the human text.
#[test]
fn provider_code_is_readable_and_not_displayed() {
    let err = AiMuxError::ApiCall(ApiCallError {
        status_code: Some(400),
        provider_code: Some("invalid_request".into()),
        message: "bad input".into(),
        ..Default::default()
    });
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
        AiMuxError::ApiCall(ApiCallError {
            message: "boom".into(),
            ..Default::default()
        })
        .to_string(),
        "API call error: boom"
    );
    assert_eq!(
        AiMuxError::ApiCall(ApiCallError {
            message: "reset".into(),
            is_retryable: true,
            ..Default::default()
        })
        .to_string(),
        "API call error: reset"
    );
    assert_eq!(
        AiMuxError::TokenExpired("expired".into()).to_string(),
        "token expired: expired"
    );
}

/// `AiMuxError` rides in every `Result<T, AiMuxError>`. `ApiCall` carries its
/// detail inline (unboxed, async-openai `ApiError` style); this guard pins the
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
