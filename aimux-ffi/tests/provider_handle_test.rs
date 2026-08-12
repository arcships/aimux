//! Tests for the RFC-0027 provider handle FFI functions:
//! `aimux_provider_handle_new`, `aimux_provider_list_models`,
//! `aimux_provider_model`.
//!
//! Handle lifecycle (create → use → drop) and failures (null args, invalid
//! handle) without touching the network. Failures use sentinel returns +
//! `AimuxError *err` (not JSON envelopes).

use std::ffi::CString;
use std::ptr;

use aimux_ffi::{
    AIMUX_OK, CAimuxError, aimux_drop_handle, aimux_provider_handle_new,
    aimux_provider_list_models, aimux_provider_model,
};

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn clear_err() -> CAimuxError {
    CAimuxError {
        code: AIMUX_OK,
        status: -1,
        retry_ms: -1,
        message: std::ptr::null_mut(),
        error_value: std::ptr::null_mut(),
        retryable: 0,
        reserved: 0,
    }
}

fn expect_handle(h: u64, name: &str) -> u64 {
    assert_ne!(h, 0, "{name}: expected non-zero handle");
    h
}

fn expect_fail_u64(h: u64, err: &CAimuxError, name: &str) {
    assert_eq!(h, 0, "{name}: expected 0");
    assert_ne!(err.code, AIMUX_OK, "{name}: expected error code");
}

fn expect_fail_ptr(ptr: *mut std::os::raw::c_char, err: &CAimuxError, name: &str) {
    assert!(ptr.is_null(), "{name}: expected NULL");
    assert_ne!(err.code, AIMUX_OK, "{name}: expected error code");
}

// ── aimux_provider_handle_new ────────────────────────────────────────────────

#[test]
fn provider_handle_new_success() {
    let mut err = clear_err();
    let name = c("deepseek");
    let key = c("sk-test-fake");
    let handle = expect_handle(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), ptr::null(), &mut err),
        "provider_handle_new(deepseek)",
    );
    aimux_drop_handle(handle);
}

#[test]
fn provider_handle_new_null_name() {
    let mut err = clear_err();
    let key = c("sk-test-fake");
    expect_fail_u64(
        aimux_provider_handle_new(ptr::null(), key.as_ptr(), ptr::null(), &mut err),
        &err,
        "provider_handle_new(null name)",
    );
}

#[test]
fn provider_handle_new_unknown_provider() {
    let mut err = clear_err();
    let name = c("no-such-provider");
    let key = c("sk-test-fake");
    expect_fail_u64(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), ptr::null(), &mut err),
        &err,
        "provider_handle_new(unknown provider)",
    );
}

#[test]
fn provider_handle_new_with_config_json() {
    let mut err = clear_err();
    let name = c("deepseek");
    let key = c("sk-test-fake");
    let config = c(r#"{"base_url":"https://example.com/v1"}"#);
    let handle = expect_handle(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), config.as_ptr(), &mut err),
        "provider_handle_new(deepseek, config)",
    );
    aimux_drop_handle(handle);
}

#[test]
fn provider_handle_new_bad_config_json() {
    let mut err = clear_err();
    let name = c("deepseek");
    let key = c("sk-test-fake");
    let config = c("not valid json {{{");
    expect_fail_u64(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), config.as_ptr(), &mut err),
        &err,
        "provider_handle_new(bad config)",
    );
}

// ── aimux_provider_model ─────────────────────────────────────────────────────

#[test]
fn provider_model_success() {
    let mut err = clear_err();
    let name = c("deepseek");
    let key = c("sk-test-fake");
    let provider_handle = expect_handle(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), ptr::null(), &mut err),
        "provider_handle_new",
    );

    let model_id = c("deepseek-chat");
    let model_handle = expect_handle(
        aimux_provider_model(provider_handle, model_id.as_ptr(), &mut err),
        "provider_model",
    );

    aimux_drop_handle(model_handle);
    aimux_drop_handle(provider_handle);
}

#[test]
fn provider_model_null_model_id() {
    let mut err = clear_err();
    let name = c("deepseek");
    let key = c("sk-test-fake");
    let provider_handle = expect_handle(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), ptr::null(), &mut err),
        "provider_handle_new",
    );
    expect_fail_u64(
        aimux_provider_model(provider_handle, ptr::null(), &mut err),
        &err,
        "provider_model(null model_id)",
    );
    aimux_drop_handle(provider_handle);
}

#[test]
fn provider_model_invalid_handle() {
    let mut err = clear_err();
    let model_id = c("deepseek-chat");
    expect_fail_u64(
        aimux_provider_model(999999, model_id.as_ptr(), &mut err),
        &err,
        "provider_model(invalid handle)",
    );
}

// ── aimux_provider_list_models ───────────────────────────────────────────────

#[test]
fn provider_list_models_invalid_handle() {
    let mut err = clear_err();
    let ptr = aimux_provider_list_models(999999, &mut err);
    expect_fail_ptr(ptr, &err, "list_models(invalid handle)");
}

#[test]
fn provider_list_models_dropped_handle() {
    let mut err = clear_err();
    let name = c("deepseek");
    let key = c("sk-test-fake");
    let provider_handle = expect_handle(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), ptr::null(), &mut err),
        "provider_handle_new",
    );
    aimux_drop_handle(provider_handle);
    let ptr = aimux_provider_list_models(provider_handle, &mut err);
    expect_fail_ptr(ptr, &err, "list_models(dropped handle)");
}

#[test]
fn provider_list_models_handle_zero() {
    let mut err = clear_err();
    let ptr = aimux_provider_list_models(0, &mut err);
    expect_fail_ptr(ptr, &err, "list_models(handle=0)");
}

// ── Handle lifecycle round-trip ──────────────────────────────────────────────

#[test]
fn handle_lifecycle_create_model_drop_all() {
    let mut err = clear_err();
    let name = c("deepseek");
    let key = c("sk-test-fake");
    let provider_handle = expect_handle(
        aimux_provider_handle_new(name.as_ptr(), key.as_ptr(), ptr::null(), &mut err),
        "create provider handle",
    );
    let model_id = c("deepseek-chat");
    let model_handle = expect_handle(
        aimux_provider_model(provider_handle, model_id.as_ptr(), &mut err),
        "create model from provider handle",
    );
    assert_ne!(provider_handle, model_handle);
    aimux_drop_handle(model_handle);
    aimux_drop_handle(provider_handle);
}
