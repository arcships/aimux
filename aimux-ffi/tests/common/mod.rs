//! Shared helpers for the aimux-ffi integration tests: read and release the
//! `aimux_error_t` a failed call returns.
#![allow(dead_code)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use aimux_ffi::{
    AIMUX_E_ABORTED, AIMUX_E_FFI_CALLBACK_FAILURE, AIMUX_E_FFI_NULL_POINTER, AIMUX_E_OTHER,
    AIMUX_E_RECORDING_INIT, AIMUX_E_RECORDING_WRITE, aimux_error_code, aimux_error_free,
    aimux_error_message, aimux_error_t, aimux_free_string,
};

pub fn c(s: &str) -> CString {
    CString::new(s).expect("no interior NUL in test literals")
}

/// Own the string a getter returns; empty for NULL.
pub fn take(p: *mut c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: an owned NUL-terminated C string from an aimux getter.
    let s = unsafe { CStr::from_ptr(p) }
        .to_str()
        .unwrap_or("")
        .to_string();
    unsafe { aimux_free_string(p) };
    s
}

/// Message of a returned error, which is then freed.
pub fn msg(e: *mut aimux_error_t) -> String {
    assert!(!e.is_null(), "expected a returned error");
    let m = take(aimux_error_message(e));
    aimux_error_free(e);
    m
}

/// Assert success (NULL), printing the message otherwise.
pub fn ok(e: *mut aimux_error_t, name: &str) {
    if !e.is_null() {
        panic!("{name}: expected success, got: {}", msg(e));
    }
}

/// AiMuxError code and message; the returned error is freed.
pub fn expect_aimux_error(e: *mut aimux_error_t, name: &str) -> (i32, String) {
    assert!(!e.is_null(), "{name}: expected a returned error");
    let code = aimux_error_code(e);
    if !(AIMUX_E_OTHER..=AIMUX_E_ABORTED).contains(&code) {
        panic!(
            "{name}: expected an AiMuxError code, got {code}: {}",
            msg(e)
        );
    }
    let out = (code, take(aimux_error_message(e)));
    assert!(!out.1.is_empty(), "{name}: expected a message");
    aimux_error_free(e);
    out
}

/// RecordingError code and message; the returned error is freed.
pub fn expect_recording_error(e: *mut aimux_error_t, name: &str) -> (i32, String) {
    assert!(!e.is_null(), "{name}: expected a returned error");
    let code = aimux_error_code(e);
    if !(AIMUX_E_RECORDING_INIT..=AIMUX_E_RECORDING_WRITE).contains(&code) {
        panic!(
            "{name}: expected a RecordingError code, got {code}: {}",
            msg(e)
        );
    }
    let out = (code, take(aimux_error_message(e)));
    assert!(!out.1.is_empty(), "{name}: expected a message");
    aimux_error_free(e);
    out
}

/// C ABI failure code and non-empty message; the returned error is freed.
pub fn expect_ffi_error(e: *mut aimux_error_t, name: &str) -> String {
    assert!(!e.is_null(), "{name}: expected a returned error");
    let code = aimux_error_code(e);
    assert!(
        (AIMUX_E_FFI_NULL_POINTER..=AIMUX_E_FFI_CALLBACK_FAILURE).contains(&code),
        "{name}: expected a C ABI failure code, got {code}"
    );
    let m = msg(e);
    assert!(!m.is_empty(), "{name}: expected a message");
    m
}

/// Any failure with a non-empty message; the returned error is freed.
pub fn expect_failure(e: *mut aimux_error_t, name: &str) -> String {
    assert!(!e.is_null(), "{name}: expected a returned error");
    let m = msg(e);
    assert!(!m.is_empty(), "{name}: expected a message");
    m
}
