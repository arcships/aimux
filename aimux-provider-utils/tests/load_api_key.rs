//! Rust tests for `load_api_key`, the Rust equivalent of
//! `packages/provider-utils/src/load-api-key.ts`.
//!
//! There is no `load-api-key.test.ts` in the upstream SDK, so these tests are
//! derived directly from the TS `loadApiKey` semantics and the Rust
//! [`aimux_provider_utils::load_api_key`] implementation. Because they mutate
//! process-global environment variables, every test is marked `#[serial]` so
//! they cannot run concurrently and race on the shared env var.

use aimux_core::AiMuxError;
use aimux_provider_utils::api_key::{load_optional_setting, load_setting};
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
fn returns_load_api_key_error_when_neither_provided() {
    // TS: throws LoadAPIKeyError when apiKey is null and the env var is unset.
    cleanup();
    remove_env();
    let err = load_api_key(None, ENV_VAR, "Test API key").unwrap_err();
    assert!(matches!(err, AiMuxError::LoadApiKey { .. }));
}

#[test]
#[serial]
fn error_carries_description_and_env_var_and_mentions_both() {
    // The error guides the user to both the parameter and the env var.
    cleanup();
    remove_env();
    let err = load_api_key(None, ENV_VAR, "Test API key").unwrap_err();
    let AiMuxError::LoadApiKey {
        env_var,
        description,
    } = &err
    else {
        panic!("expected LoadApiKey error, got {err:?}");
    };
    assert_eq!(env_var, ENV_VAR);
    assert_eq!(description, "Test API key");
    let msg = err.to_string();
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
fn empty_string_api_key_is_returned_verbatim_without_env_fallback() {
    // TS: `typeof apiKey === 'string'` returns the value, even "". An explicit
    // empty key must never be replaced by a credential from the environment.
    cleanup();
    set_env("env-key");
    let key = load_api_key(Some(""), ENV_VAR, "Test").unwrap();
    assert_eq!(key, "");
    cleanup();
}

#[test]
#[serial]
fn empty_string_api_key_is_returned_when_env_var_is_unset_too() {
    cleanup();
    remove_env();
    let key = load_api_key(Some(""), ENV_VAR, "Test").unwrap();
    assert_eq!(key, "");
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
fn whitespace_only_api_key_is_returned_as_is() {
    // An explicit value is never trimmed or second-guessed.
    cleanup();
    set_env("env-key");
    let key = load_api_key(Some("   "), ENV_VAR, "Test").unwrap();
    assert_eq!(key, "   ");
    cleanup();
}

#[test]
#[serial]
fn load_setting_prefers_the_value_then_the_environment() {
    cleanup();
    set_env("env-value");
    assert_eq!(
        load_setting(Some("given"), ENV_VAR, "region").unwrap(),
        "given"
    );
    assert_eq!(load_setting(Some(""), ENV_VAR, "region").unwrap(), "");
    assert_eq!(load_setting(None, ENV_VAR, "region").unwrap(), "env-value");
    cleanup();
}

#[test]
#[serial]
fn load_setting_reports_the_missing_setting() {
    cleanup();
    let err = load_setting(None, ENV_VAR, "region").unwrap_err();
    let AiMuxError::LoadSetting { env_var, name } = &err else {
        panic!("expected LoadSetting error, got {err:?}");
    };
    assert_eq!(env_var, ENV_VAR);
    assert_eq!(name, "region");
    assert!(err.to_string().contains("region"));
}

#[test]
#[serial]
fn load_optional_setting_never_fails() {
    cleanup();
    assert_eq!(load_optional_setting(None, ENV_VAR), None);
    assert_eq!(
        load_optional_setting(Some(""), ENV_VAR),
        Some(String::new())
    );
    set_env("env-value");
    assert_eq!(
        load_optional_setting(None, ENV_VAR).as_deref(),
        Some("env-value")
    );
    assert_eq!(
        load_optional_setting(Some("given"), ENV_VAR).as_deref(),
        Some("given")
    );
    cleanup();
}
