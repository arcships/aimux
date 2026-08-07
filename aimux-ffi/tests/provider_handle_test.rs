//! Tests for the RFC-0027 provider handle FFI functions:
//! `aimux_provider_handle_new`, `aimux_provider_list_models`,
//! `aimux_provider_model`.
//!
//! These tests exercise handle lifecycle (create → use → drop) and error
//! envelopes (null args, invalid handle) without touching the network —
//! `provider_handle_new` only builds a config + provider; `list_models` on an
//! invalid/expired handle returns an error envelope; `provider_model` on a
//! valid handle builds a model (no network).

use std::ffi::CString;
use std::os::raw::c_char;

use aimux_ffi::{
    aimux_drop_handle, aimux_free_string, aimux_provider_handle_new, aimux_provider_list_models,
    aimux_provider_model,
};

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

/// Copy an FFI JSON result into an owned `String`, freeing the native pointer.
fn take_json(json_ptr: *mut c_char, name: &str) -> String {
    assert!(!json_ptr.is_null(), "{name}: null result pointer");
    let json = unsafe { std::ffi::CStr::from_ptr(json_ptr) }
        .to_str()
        .unwrap()
        .to_string();
    unsafe { aimux_free_string(json_ptr) };
    json
}

/// Parse `{"handle":<u64>}` from a constructor result, returning the handle.
fn extract_handle(json_ptr: *mut c_char, name: &str) -> u64 {
    let json = take_json(json_ptr, name);
    let value: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("{name}: result is not valid JSON ({e}): {json}"));
    assert!(
        value.get("error").is_none(),
        "{name}: expected success, got error envelope: {json}"
    );
    value
        .get("handle")
        .and_then(|h| h.as_u64())
        .unwrap_or_else(|| panic!("{name}: expected {{\"handle\":<u64>}}, got {json}"))
}

/// Assert a result is an error envelope (`{"error":...}`).
fn expect_error(json_ptr: *mut c_char, name: &str) {
    let json = take_json(json_ptr, name);
    let value: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("{name}: result is not valid JSON ({e}): {json}"));
    assert!(
        value.get("error").is_some(),
        "{name}: expected error envelope, got {json}"
    );
}

// ── aimux_provider_handle_new ────────────────────────────────────────────────

#[test]
fn provider_handle_new_success() {
    // deepseek is registry-backed; a fake key is fine (no network on construct).
    let name = c("deepseek");
    let key = c("sk-test-fake");
    let handle = extract_handle(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), std::ptr::null()),
        "provider_handle_new(deepseek)",
    );
    assert!(handle != 0, "handle should be non-zero");
    aimux_drop_handle(handle);
}

#[test]
fn provider_handle_new_null_name() {
    let key = c("sk-test-fake");
    expect_error(
        aimux_provider_handle_new(std::ptr::null(), key.as_ptr(), std::ptr::null()),
        "provider_handle_new(null name)",
    );
}

#[test]
fn provider_handle_new_unknown_provider() {
    let name = c("no-such-provider");
    let key = c("sk-test-fake");
    expect_error(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), std::ptr::null()),
        "provider_handle_new(unknown provider)",
    );
}

#[test]
fn provider_handle_new_with_config_json() {
    let name = c("deepseek");
    let key = c("sk-test-fake");
    let config = c(r#"{"base_url":"https://example.com/v1"}"#);
    let handle = extract_handle(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), config.as_ptr()),
        "provider_handle_new(deepseek, config)",
    );
    assert!(handle != 0);
    aimux_drop_handle(handle);
}

#[test]
fn provider_handle_new_bad_config_json() {
    let name = c("deepseek");
    let key = c("sk-test-fake");
    let config = c("not valid json {{{");
    expect_error(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), config.as_ptr()),
        "provider_handle_new(bad config)",
    );
}

// ── aimux_provider_model ─────────────────────────────────────────────────────

#[test]
fn provider_model_success() {
    let name = c("deepseek");
    let key = c("sk-test-fake");
    let provider_handle = extract_handle(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), std::ptr::null()),
        "provider_handle_new",
    );

    let model_id = c("deepseek-chat");
    let model_handle = extract_handle(
        aimux_provider_model(provider_handle, model_id.as_ptr()),
        "provider_model",
    );
    assert!(model_handle != 0);

    aimux_drop_handle(model_handle);
    aimux_drop_handle(provider_handle);
}

#[test]
fn provider_model_null_model_id() {
    let name = c("deepseek");
    let key = c("sk-test-fake");
    let provider_handle = extract_handle(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), std::ptr::null()),
        "provider_handle_new",
    );
    expect_error(
        aimux_provider_model(provider_handle, std::ptr::null()),
        "provider_model(null model_id)",
    );
    aimux_drop_handle(provider_handle);
}

#[test]
fn provider_model_invalid_handle() {
    let model_id = c("deepseek-chat");
    expect_error(
        aimux_provider_model(999999, model_id.as_ptr()),
        "provider_model(invalid handle)",
    );
}

// ── aimux_provider_list_models ───────────────────────────────────────────────

#[test]
fn provider_list_models_invalid_handle() {
    // Handle 999999 was never registered → InvalidHandle error envelope.
    expect_error(
        aimux_provider_list_models(999999),
        "list_models(invalid handle)",
    );
}

#[test]
fn provider_list_models_dropped_handle() {
    let name = c("deepseek");
    let key = c("sk-test-fake");
    let provider_handle = extract_handle(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), std::ptr::null()),
        "provider_handle_new",
    );
    // Drop the handle, then try to list_models on it → should error.
    aimux_drop_handle(provider_handle);
    expect_error(
        aimux_provider_list_models(provider_handle),
        "list_models(dropped handle)",
    );
}

#[test]
fn provider_list_models_handle_zero() {
    // Handle 0 is reserved for "failure / invalid".
    expect_error(aimux_provider_list_models(0), "list_models(handle=0)");
}

// ── Handle lifecycle round-trip ──────────────────────────────────────────────

#[test]
fn handle_lifecycle_create_model_drop_all() {
    let name = c("deepseek");
    let key = c("sk-test-fake");
    let provider_handle = extract_handle(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), std::ptr::null()),
        "create provider handle",
    );
    let model_id = c("deepseek-chat");
    let model_handle = extract_handle(
        aimux_provider_model(provider_handle, model_id.as_ptr()),
        "create model from provider handle",
    );
    // Both handles are valid and distinct.
    assert!(provider_handle != 0);
    assert!(model_handle != 0);
    assert!(provider_handle != model_handle);

    // Dropping both should not panic.
    aimux_drop_handle(model_handle);
    aimux_drop_handle(provider_handle);

    // After drop, the provider handle is invalid.
    expect_error(
        aimux_provider_list_models(provider_handle),
        "list_models after drop",
    );
}
