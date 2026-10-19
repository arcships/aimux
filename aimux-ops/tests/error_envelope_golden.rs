use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aimux_core::error::{AiMuxError, ApiCallError, RetryError, RetryErrorReason};
use aimux_core::recording::RecordingError;
use aimux_ops::envelope::{BindingError, ErrorDomain, ErrorEnvelope, OpsError, ProtocolError};
use aimux_ops::handle::HandleKind;
use serde_json::{Value, json};

fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../contract-tests/ops/errors")
        .join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

const AIMUX: &[&str] = &[
    "other",
    "json-parse",
    "invalid-response-data",
    "tool-call-not-found-for-approval",
    "invalid-argument",
    "invalid-prompt",
    "missing-tool-results",
    "token-expired",
    "unsupported-functionality",
    "no-such-model",
    "no-such-provider",
    "api-call",
    "timeout",
    "aborted",
    "retry",
    "no-such-tool",
    "invalid-tool-input",
    "tool-call-repair",
    "no-output-generated",
    "load-api-key",
    "load-setting",
    "tool-choice-violation",
];

fn recording_errors() -> Vec<(&'static str, OpsError)> {
    vec![
        (
            "recording-init",
            RecordingError::Init {
                path: "recordings/session.jsonl".into(),
                source: std::io::Error::other("permission denied"),
            }
            .into(),
        ),
        (
            "recording-open-file",
            RecordingError::OpenFile {
                path: "recordings/session.jsonl".into(),
                source: std::io::Error::other("permission denied"),
            }
            .into(),
        ),
        (
            "recording-spawn",
            RecordingError::Spawn {
                source: std::io::Error::other("thread limit"),
            }
            .into(),
        ),
        ("recording-writer-gone", RecordingError::WriterGone.into()),
        (
            "recording-flush-timeout",
            RecordingError::FlushTimeout.into(),
        ),
        (
            "recording-write",
            RecordingError::Write("disk full".into()).into(),
        ),
    ]
}

fn binding_errors() -> Vec<(&'static str, OpsError)> {
    vec![
        (
            "binding-null-pointer",
            BindingError::NullPointer {
                argument: "params_json",
            }
            .into(),
        ),
        (
            "binding-invalid-utf8",
            BindingError::InvalidUtf8 { argument: "op" }.into(),
        ),
        (
            "binding-invalid-wire-json",
            BindingError::InvalidWireJson {
                argument: "params_json".into(),
                message: "expected object".into(),
            }
            .into(),
        ),
        (
            "binding-invalid-handle",
            BindingError::InvalidHandle {
                expected: vec![HandleKind::Language, HandleKind::Embedding],
                actual: Some(HandleKind::Provider),
            }
            .into(),
        ),
        ("binding-reentrant-call", BindingError::ReentrantCall.into()),
        (
            "binding-result-serialization",
            BindingError::ResultSerialization("invalid result".into()).into(),
        ),
        (
            "binding-callback-failure",
            BindingError::CallbackFailure("callback stopped".into()).into(),
        ),
    ]
}

fn protocol_errors() -> Vec<(&'static str, OpsError)> {
    vec![
        (
            "protocol-invalid-op",
            ProtocolError::InvalidOp("missing.op".into()).into(),
        ),
        (
            "protocol-invalid-params",
            ProtocolError::InvalidParams("expected object".into()).into(),
        ),
        (
            "protocol-unsupported-ops-version",
            ProtocolError::UnsupportedOpsVersion("2.0".into()).into(),
        ),
        (
            "protocol-concurrent-consumer",
            ProtocolError::ConcurrentConsumer.into(),
        ),
        (
            "protocol-invalid-state",
            ProtocolError::InvalidState("already finished".into()).into(),
        ),
        (
            "protocol-resource-limit",
            ProtocolError::ResourceLimit("input queue full".into()).into(),
        ),
        (
            "protocol-handle-exhausted",
            ProtocolError::HandleExhausted.into(),
        ),
    ]
}

fn assert_golden(name: &str, error: OpsError) {
    let expected = fixture(name);
    let envelope = error.envelope(now());
    assert_eq!(serde_json::to_value(&envelope).unwrap(), expected, "{name}");
    assert_eq!(error.code(), envelope.code, "{name}");
    let decoded: ErrorEnvelope = serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(decoded, envelope, "{name}");
    assert_eq!(serde_json::to_value(decoded).unwrap(), expected, "{name}");
    assert!(expected.get("retryable").is_none(), "{name}");
    assert!(expected["error"].get("type").is_none(), "{name}");
    assert!(expected["error"]["name"].is_string(), "{name}");
}

#[test]
fn every_aimux_variant_matches_golden_and_round_trips() {
    for slug in AIMUX {
        let name = format!("aimux-{slug}");
        let error: AiMuxError = serde_json::from_value(fixture(&name)["error"].clone()).unwrap();
        assert_golden(&name, error.into());
    }
}

#[test]
fn every_recording_variant_matches_golden_and_round_trips() {
    for (name, error) in recording_errors() {
        assert_golden(name, error);
    }
}

#[test]
fn every_binding_variant_matches_golden_and_round_trips() {
    for (name, error) in binding_errors() {
        assert_golden(name, error);
    }
}

#[test]
fn every_protocol_variant_matches_golden_and_round_trips() {
    for (name, error) in protocol_errors() {
        assert_golden(name, error);
    }
}

#[test]
fn codes_preserve_existing_groups_and_have_disjoint_ranges() {
    let mut codes: BTreeMap<i32, BTreeSet<String>> = BTreeMap::new();
    for slug in AIMUX {
        let expected = fixture(&format!("aimux-{slug}"));
        let error: AiMuxError = serde_json::from_value(expected["error"].clone()).unwrap();
        let error = OpsError::from(error);
        assert!((1..=20).contains(&error.code()));
        assert_ne!(error.code(), 4, "retired code must stay unused");
        codes
            .entry(error.code())
            .or_default()
            .insert((*slug).into());
    }
    assert_eq!(codes.len(), 19);
    let aliases: BTreeMap<_, _> = codes
        .into_iter()
        .filter(|(_, names)| names.len() > 1)
        .collect();
    assert_eq!(
        aliases,
        BTreeMap::from([
            (
                3,
                BTreeSet::from([
                    "invalid-response-data".into(),
                    "tool-call-not-found-for-approval".into(),
                    "tool-choice-violation".into(),
                ])
            ),
            (
                6,
                BTreeSet::from(["invalid-prompt".into(), "missing-tool-results".into()])
            ),
        ])
    );
    for (errors, range) in [
        (recording_errors(), 100..=105),
        (binding_errors(), 200..=206),
        (protocol_errors(), 300..=306),
    ] {
        let mut seen = BTreeSet::new();
        for (name, error) in errors {
            assert!(range.contains(&error.code()), "{name}");
            assert!(seen.insert(error.code()), "duplicate code for {name}");
        }
        assert_eq!(seen, range.collect());
    }
}

#[test]
fn unknown_variants_and_nested_nulls_survive_without_reclassification() {
    for kind in ["aimux", "recording", "binding", "protocol"] {
        let mut expected = fixture("unknown-aimux");
        expected["kind"] = json!(kind);
        let envelope: ErrorEnvelope = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(envelope.code, 21);
        assert_eq!(envelope.error["name"], "AI_FutureError");
        assert!(envelope.error["providerData"]["explicitNull"].is_null());
        assert_eq!(serde_json::to_value(envelope).unwrap(), expected);
    }
}

fn api(headers: &[(&str, &str)]) -> ApiCallError {
    ApiCallError {
        response_headers: Some(
            headers
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect::<HashMap<_, _>>(),
        ),
        is_retryable: true,
        ..ApiCallError::new("retry later", "https://example.test/v1", json!({}))
    }
}

#[test]
fn optional_payload_fields_distinguish_absent_and_explicit_null() {
    let absent = OpsError::from(AiMuxError::ApiCall(Box::new(ApiCallError::new(
        "reset",
        "https://example.test/v1",
        Value::Null,
    ))))
    .envelope(now());
    let mut api = ApiCallError::new("reset", "https://example.test/v1", Value::Null);
    api.data = Some(Value::Null);
    let explicit_null = OpsError::from(AiMuxError::ApiCall(Box::new(api))).envelope(now());
    assert!(absent.error.get("data").is_none());
    assert_eq!(explicit_null.error.get("data"), Some(&Value::Null));
    for envelope in [absent, explicit_null] {
        for key in [
            "statusCode",
            "providerCode",
            "responseBody",
            "responseHeaders",
        ] {
            assert!(envelope.error.get(key).is_none(), "{key}");
        }
        let wire = serde_json::to_value(&envelope).unwrap();
        assert!(wire.get("retryAfterMs").is_none());
        let decoded: ErrorEnvelope = serde_json::from_value(wire).unwrap();
        assert_eq!(decoded, envelope);
    }
}

#[test]
fn invalid_handle_without_actual_kind_omits_the_field() {
    let error = OpsError::from(BindingError::InvalidHandle {
        expected: vec![HandleKind::Language],
        actual: None,
    })
    .envelope(now());
    assert_eq!(error.kind, ErrorDomain::Binding);
    assert_eq!(error.code, 203);
    assert_eq!(
        error.error,
        json!({"name":"InvalidHandle", "expected":["Language"]})
    );
}

#[test]
fn retry_hint_uses_the_captured_clock_and_distinguishes_zero() {
    let error = OpsError::from(AiMuxError::ApiCall(Box::new(api(&[(
        "Retry-After",
        "Tue, 14 Nov 2023 22:13:22 GMT",
    )]))));
    let envelope = error.envelope(now());
    assert_eq!(envelope.retry_after_ms, Some(2000));
    assert_eq!(
        error
            .envelope(now() + Duration::from_secs(1))
            .retry_after_ms,
        Some(1000)
    );
    let encoded = serde_json::to_string(&envelope).unwrap();
    let decoded: ErrorEnvelope = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.retry_after_ms, Some(2000));
    let zero = OpsError::from(AiMuxError::ApiCall(Box::new(api(&[(
        "retry-after-ms",
        "0",
    )]))))
    .envelope(now());
    assert_eq!(zero.retry_after_ms, Some(0));
    assert_eq!(serde_json::to_value(zero).unwrap()["retryAfterMs"], 0);
}

#[test]
fn only_direct_api_calls_get_top_level_retry_hints() {
    let attempt = AiMuxError::ApiCall(Box::new(api(&[("retry-after-ms", "1500")])));
    for error in [
        AiMuxError::Retry(RetryError {
            reason: RetryErrorReason::MaxRetriesExceeded,
            errors: vec![attempt.clone()],
        }),
        AiMuxError::ToolCallRepair {
            original_error: Box::new(attempt),
            cause: Box::new(AiMuxError::other("repair failed")),
        },
        AiMuxError::Timeout("timeout".into()),
        AiMuxError::Aborted("aborted".into()),
    ] {
        let payload = serde_json::to_value(&error).unwrap();
        let envelope = OpsError::from(error).envelope(now());
        assert_eq!(envelope.retry_after_ms, None);
        assert_eq!(envelope.error, payload);
        assert!(
            serde_json::to_value(envelope)
                .unwrap()
                .get("retryAfterMs")
                .is_none()
        );
    }
}

#[test]
fn settings_errors_redact_secret_canary() {
    const VARIABLE: &str = "AIMUX_OPS_REDACTION_CANARY";
    const SECRET: &str = "sk-redaction-canary-do-not-expose";
    const CHILD: &str = "AIMUX_OPS_REDACTION_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let key = aimux_provider_utils::api_key::load_api_key(None, VARIABLE, "test provider")
            .unwrap_err();
        let setting = aimux_provider_utils::api_key::load_setting(None, VARIABLE, "test setting")
            .unwrap_err();
        assert!(matches!(key, AiMuxError::LoadApiKey { .. }));
        assert!(matches!(setting, AiMuxError::LoadSetting { .. }));
        for error in [key, setting] {
            let nested = AiMuxError::ToolCallRepair {
                original_error: Box::new(error.clone()),
                cause: Box::new(AiMuxError::Retry(RetryError {
                    reason: RetryErrorReason::MaxRetriesExceeded,
                    errors: vec![error.clone()],
                })),
            };
            for error in [error, nested] {
                let wire = serde_json::to_string(&OpsError::from(error).envelope(now())).unwrap();
                assert!(!wire.contains(SECRET), "secret leaked in envelope: {wire}");
            }
        }
        return;
    }

    // Separate processes avoid mutating the environment of parallel tests.
    let run = |value: Option<std::ffi::OsString>| {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "settings_errors_redact_secret_canary"])
            .env(CHILD, "1")
            .env_remove(VARIABLE);
        if let Some(value) = value {
            command.env(VARIABLE, value);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(None);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let mut invalid = SECRET.as_bytes().to_vec();
        invalid.push(0xff);
        run(Some(std::ffi::OsString::from_vec(invalid)));
    }
}
