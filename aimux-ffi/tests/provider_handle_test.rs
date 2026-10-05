//! Tests for the RFC-0027 provider handle FFI functions:
//! `aimux_provider_handle_new`, `aimux_provider_list_models`,
//! `aimux_provider_model`.
//!
//! Handle lifecycle (create → use → drop) and failures (null args, invalid
//! handle) without touching the network.

mod common;

use std::os::raw::c_char;
use std::ptr;

use aimux_ffi::{
    AIMUX_E_NO_SUCH_PROVIDER, aimux_drop_handle, aimux_error_t, aimux_provider_handle_new,
    aimux_provider_list_models, aimux_provider_model,
};
use common::{c, expect_aimux_error, expect_ffi_error, ok};

fn expect_handle(e: *mut aimux_error_t, h: u64, name: &str) -> u64 {
    ok(e, name);
    assert_ne!(h, 0, "{name}: expected non-zero handle");
    h
}

fn deepseek() -> u64 {
    let mut h = 0;
    let e = aimux_provider_handle_new(
        c("deepseek").as_ptr(),
        c("sk-test-fake").as_ptr(),
        ptr::null(),
        &mut h,
    );
    expect_handle(e, h, "provider_handle_new(deepseek)")
}

// ── aimux_provider_handle_new ────────────────────────────────────────────────

#[test]
fn provider_handle_new_success() {
    aimux_drop_handle(deepseek());
}

#[test]
fn provider_handle_new_null_name() {
    let mut h = 0;
    let e = aimux_provider_handle_new(ptr::null(), c("sk").as_ptr(), ptr::null(), &mut h);
    assert_eq!(h, 0);
    assert_eq!(
        expect_ffi_error(e, "provider_handle_new(null name)"),
        "name: must not be NULL"
    );
}

#[test]
fn provider_handle_new_unknown_provider() {
    let mut h = 0;
    let e = aimux_provider_handle_new(
        c("no-such-provider").as_ptr(),
        c("sk").as_ptr(),
        ptr::null(),
        &mut h,
    );
    assert_eq!(h, 0);
    assert_eq!(
        expect_aimux_error(e, "provider_handle_new(unknown provider)").0,
        AIMUX_E_NO_SUCH_PROVIDER
    );
}

#[test]
fn provider_handle_new_with_config_json() {
    let mut h = 0;
    let e = aimux_provider_handle_new(
        c("deepseek").as_ptr(),
        c("sk-test-fake").as_ptr(),
        c(r#"{"base_url":"https://example.com/v1"}"#).as_ptr(),
        &mut h,
    );
    aimux_drop_handle(expect_handle(e, h, "provider_handle_new(deepseek, config)"));
}

#[test]
fn provider_handle_new_bad_config_json() {
    let mut h = 0;
    let e = aimux_provider_handle_new(
        c("deepseek").as_ptr(),
        c("sk-test-fake").as_ptr(),
        c("not valid json {{{").as_ptr(),
        &mut h,
    );
    assert_eq!(h, 0);
    assert!(
        expect_ffi_error(e, "provider_handle_new(bad config)")
            .starts_with("config_json: invalid JSON:")
    );
}

// ── aimux_provider_model ─────────────────────────────────────────────────────

#[test]
fn provider_model_success() {
    let provider_handle = deepseek();
    let mut model_handle = 0;
    let e = aimux_provider_model(
        provider_handle,
        c("deepseek-chat").as_ptr(),
        &mut model_handle,
    );
    expect_handle(e, model_handle, "provider_model");
    assert_ne!(provider_handle, model_handle);
    aimux_drop_handle(model_handle);
    aimux_drop_handle(provider_handle);
}

#[test]
fn provider_model_null_model_id() {
    let provider_handle = deepseek();
    let mut h = 0;
    let e = aimux_provider_model(provider_handle, ptr::null(), &mut h);
    assert_eq!(h, 0);
    assert_eq!(
        expect_ffi_error(e, "provider_model(null model_id)"),
        "model_id: must not be NULL"
    );
    aimux_drop_handle(provider_handle);
}

#[test]
fn provider_model_invalid_handle() {
    let mut h = 0;
    let e = aimux_provider_model(999999, c("deepseek-chat").as_ptr(), &mut h);
    assert_eq!(h, 0);
    assert_eq!(
        expect_ffi_error(e, "provider_model(invalid handle)"),
        "invalid or expired provider handle"
    );
}

// ── aimux_provider_list_models ───────────────────────────────────────────────

#[test]
fn provider_list_models_invalid_handle() {
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_provider_list_models(999999, &mut out);
    assert!(out.is_null());
    assert_eq!(
        expect_ffi_error(e, "list_models(invalid handle)"),
        "invalid or expired provider handle"
    );
}

#[test]
fn provider_list_models_dropped_handle() {
    let provider_handle = deepseek();
    aimux_drop_handle(provider_handle);
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_provider_list_models(provider_handle, &mut out);
    assert!(out.is_null());
    assert_eq!(
        expect_ffi_error(e, "list_models(dropped handle)"),
        "invalid or expired provider handle"
    );
}

#[test]
fn provider_list_models_handle_zero() {
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_provider_list_models(0, &mut out);
    assert!(out.is_null());
    assert_eq!(
        expect_ffi_error(e, "list_models(handle=0)"),
        "invalid or expired provider handle"
    );
}
