//! Rust tests for `load_api_key`, the Rust equivalent of
//! `packages/provider-utils/src/load-api-key.ts`.
//!
//! There is no `load-api-key.test.ts` in the upstream SDK, so these tests are
//! derived directly from the TS `loadApiKey` semantics and the Rust
//! [`aimux_provider_utils::load_api_key`] implementation. Because they mutate
//! process-global environment variables, every test is marked `#[serial]` so
//! they cannot run concurrently and race on the shared env var.

use aimux_core::AiMuxError;
use aimux_provider_utils::load_api_key;
use serial_test::serial;

/// Unique env var name used by these tests, to avoid colliding with any real
/// API key the developer might have set.
const ENV_VAR: &str = "AIMUX_TEST_LOAD_API_KEY_VAR";

// `std::env::set_var` / `remove_var` are `unsafe` as of Rust 1.85 (edition
// 2024) because mutating the process environment is not thread-safe. These
// tests are `#[serial]`, so the mutation is safe in practice.
fn set_env(value: &str) {
    unsafe { std::env::set_var(ENV_VAR, value) }
}

fn remove_env() {
    unsafe { std::env::remove_var(ENV_VAR) }
}

fn cleanup() {
    remove_env();
}

#[test]
#[serial]
fn returns_api_key_when_provided() {
    // TS: `if (typeof apiKey === 'string') return apiKey;`
    cleanup();
    let key = load_api_key(Some("explicit-key"), ENV_VAR, "Test").unwrap();
    assert_eq!(key, "explicit-key");
}

#[test]
#[serial]
fn reads_api_key_from_environment_variable() {
    // TS: `apiKey = process.env[environmentVariableName]; ... return apiKey;`
    cleanup();
    set_env("env-key");
    let key = load_api_key(None, ENV_VAR, "Test").unwrap();
    assert_eq!(key, "env-key");
    cleanup();
}

#[test]
#[serial]
fn returns_invalid_argument_when_neither_provided() {
    // TS: throws LoadAPIKeyError when apiKey is null and the env var is unset.
    cleanup();
    remove_env();
    let err = load_api_key(None, ENV_VAR, "Test API key").unwrap_err();
    assert!(matches!(err, AiMuxError::InvalidArgument(_)));
}

#[test]
#[serial]
fn error_message_mentions_description_and_env_var() {
    // The error message guides the user to both the parameter and the env var.
    cleanup();
    remove_env();
    let err = load_api_key(None, ENV_VAR, "Test API key").unwrap_err();
    let msg = match err {
        AiMuxError::InvalidArgument(m) => m,
        other => panic!("expected InvalidArgument error, got {other:?}"),
    };
    assert!(
        msg.contains("Test API key"),
        "message should mention description: {msg}"
    );
    assert!(
        msg.contains(ENV_VAR),
        "message should mention env var: {msg}"
    );
}

#[test]
#[serial]
fn empty_string_api_key_falls_back_to_env_var() {
    // Rust behaviour (stricter than TS): an empty `api_key` string is treated
    // as "not provided" and falls through to the environment variable.
    cleanup();
    set_env("env-key");
    let key = load_api_key(Some(""), ENV_VAR, "Test").unwrap();
    assert_eq!(key, "env-key");
    cleanup();
}

#[test]
#[serial]
fn empty_string_api_key_errors_when_env_var_also_unset() {
    cleanup();
    remove_env();
    let err = load_api_key(Some(""), ENV_VAR, "Test").unwrap_err();
    assert!(matches!(err, AiMuxError::InvalidArgument(_)));
}

#[test]
#[serial]
fn explicit_api_key_takes_precedence_over_env_var() {
    // When both are set, the explicit parameter wins (the env var is never read).
    cleanup();
    set_env("env-key");
    let key = load_api_key(Some("explicit-key"), ENV_VAR, "Test").unwrap();
    assert_eq!(key, "explicit-key");
    cleanup();
}

#[test]
#[serial]
fn whitespace_only_api_key_falls_back_to_env_var() {
    // A whitespace-only string is non-empty, so the Rust impl returns it as-is
    // (it does not trim). This documents that behaviour.
    cleanup();
    set_env("env-key");
    let key = load_api_key(Some("   "), ENV_VAR, "Test").unwrap();
    assert_eq!(key, "   ");
    cleanup();
}
