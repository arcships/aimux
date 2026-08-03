//! Regression tests for issue #17: FFI constructors and prompt parsing must
//! preserve detailed error information.
//!
//! - Problem A: `prompt_json` parse errors must carry the `serde_json::Error`
//!   detail (like the adjacent `opts_json` branch already did).
//! - Problem B: constructor failures (unknown provider, bad config JSON,
//!   null arguments) are returned directly in the constructor's JSON result
//!   (`{"handle":<u64>}` on success, the standard error envelope on failure)
//!   — no side-channel retrieval, so there are no thread-affinity semantics
//!   to test.
//!
//! All tests are offline: they only exercise argument parsing and config
//! construction, never the network.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::sync::Mutex;

use serde_json::Value;

use aimux_ffi::{
    aimux_azure_new, aimux_drop_handle, aimux_free_string, aimux_generate_text, aimux_openai_new,
    aimux_provider_new, aimux_stream_text,
};

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

/// Copy an FFI JSON result into an owned `String`, freeing the pointer.
fn take_json(ptr: *mut c_char) -> String {
    assert!(!ptr.is_null(), "FFI call returned a null result pointer");
    // SAFETY: FFI functions return NUL-terminated strings owned by us.
    let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
    // SAFETY: ptr was produced by CString::into_raw inside the FFI layer.
    unsafe { aimux_free_string(ptr) };
    s
}

/// Parse a constructor result, expecting the error envelope.
fn expect_error_envelope(ptr: *mut c_char) -> Value {
    let json = take_json(ptr);
    let env: Value = serde_json::from_str(&json).expect("result must be a JSON envelope");
    assert!(
        env.get("error").is_some(),
        "expected error envelope, got: {json}"
    );
    env
}

/// Parse a constructor result, expecting `{"handle":<u64>}`, and return the
/// handle.
fn expect_handle(ptr: *mut c_char) -> u64 {
    let json = take_json(ptr);
    let env: Value = serde_json::from_str(&json).expect("result must be JSON");
    assert!(
        env.get("error").is_none(),
        "expected success, got error envelope: {json}"
    );
    let h = env["handle"].as_u64().expect("handle must be a u64");
    assert!(h != 0, "handle must never be 0 on success");
    h
}

/// A model handle usable with `aimux_generate_text` / `aimux_stream_text`.
fn valid_handle() -> u64 {
    expect_handle(aimux_openai_new(
        c("sk-test-fake-key").as_ptr(),
        c("gpt-4o-mini").as_ptr(),
    ))
}

// ── Problem A: prompt_json serde detail ─────────────────────────────────────

#[test]
fn generate_text_invalid_prompt_json_carries_serde_detail() {
    let h = valid_handle();
    let out = aimux_generate_text(h, c("hello").as_ptr(), ptr::null());
    let env: Value = serde_json::from_str(&take_json(out)).expect("error must be a JSON envelope");
    let msg = env["error"].as_str().expect("envelope has error field");
    assert!(
        msg.starts_with("invalid prompt_json:"),
        "expected serde detail in message, got: {msg}"
    );
    assert!(
        msg.contains("line") && msg.contains("column"),
        "expected serde line/column detail, got: {msg}"
    );
    assert_eq!(env["error_type"], "Other");
    assert!(env["status_code"].is_null());

    aimux_drop_handle(h);
}

#[test]
fn generate_text_null_prompt_json_reports_invalid_prompt() {
    let h = valid_handle();
    let out = aimux_generate_text(h, ptr::null(), ptr::null());
    let json = take_json(out);
    assert!(json.contains("invalid prompt_json"), "got: {json}");
    aimux_drop_handle(h);
}

static STREAM_ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());

extern "C" fn on_part(_json: *const c_char) {}
extern "C" fn on_done() {}

extern "C" fn on_error(json: *const c_char) {
    // SAFETY: the FFI layer guarantees the pointer is valid during the call.
    let s = unsafe { CStr::from_ptr(json) }
        .to_str()
        .unwrap()
        .to_string();
    STREAM_ERRORS.lock().unwrap().push(s);
}

#[test]
fn stream_text_invalid_prompt_json_carries_serde_detail() {
    let h = valid_handle();
    STREAM_ERRORS.lock().unwrap().clear();

    aimux_stream_text(
        h,
        c("hello").as_ptr(),
        ptr::null(),
        on_part,
        on_done,
        on_error,
    );

    let errors = STREAM_ERRORS.lock().unwrap().clone();
    assert_eq!(errors.len(), 1, "expected exactly one on_error callback");
    let env: Value = serde_json::from_str(&errors[0]).expect("error must be a JSON envelope");
    let msg = env["error"].as_str().expect("envelope has error field");
    assert!(
        msg.starts_with("invalid prompt_json:"),
        "expected serde detail in message, got: {msg}"
    );
    assert!(
        msg.contains("line"),
        "expected serde line detail, got: {msg}"
    );

    aimux_drop_handle(h);
}

// ── Problem B: constructor failures in the returned envelope ────────────────

#[test]
fn unknown_provider_reports_full_error_envelope() {
    let env = expect_error_envelope(aimux_provider_new(
        c("this-provider-does-not-exist").as_ptr(),
        c("sk-x").as_ptr(),
        c("gpt-4o-mini").as_ptr(),
        ptr::null(),
    ));
    let msg = env["error"].as_str().unwrap();
    assert!(msg.contains("unknown provider"), "got: {msg}");
    assert!(msg.contains("available providers"), "got: {msg}");
    assert_eq!(env["error_type"], "UnknownProvider");
    assert!(env["status_code"].is_null());
}

#[test]
fn invalid_config_json_reports_json_error_type() {
    let env = expect_error_envelope(aimux_provider_new(
        c("openai").as_ptr(),
        c("sk-x").as_ptr(),
        c("gpt-4o-mini").as_ptr(),
        c("{bad json").as_ptr(),
    ));
    assert!(
        env["error"]
            .as_str()
            .unwrap()
            .starts_with("invalid config_json:"),
        "got: {env}"
    );
    assert_eq!(env["error_type"], "Json");
}

#[test]
fn null_arguments_report_invalid_argument_type() {
    let env = expect_error_envelope(aimux_openai_new(ptr::null(), ptr::null()));
    assert_eq!(env["error_type"], "InvalidArgument");
    assert!(
        env["error"]
            .as_str()
            .unwrap()
            .contains("null or invalid UTF-8"),
        "got: {env}"
    );
}

#[test]
fn azure_missing_required_argument_reports_invalid_argument() {
    // resource_name is a required FFI argument; passing NULL fails at the
    // boundary with an InvalidArgument envelope.
    let env = expect_error_envelope(aimux_azure_new(
        c("k").as_ptr(),
        ptr::null(),
        c("deploy").as_ptr(),
        ptr::null(),
    ));
    assert_eq!(env["error_type"], "InvalidArgument");
}

// ── Envelope semantics ──────────────────────────────────────────────────────

#[test]
fn success_envelope_has_handle_and_no_error() {
    let h = expect_handle(aimux_provider_new(
        c("groq").as_ptr(),
        c("sk-test").as_ptr(),
        c("llama-3.3-70b").as_ptr(),
        ptr::null(),
    ));
    aimux_drop_handle(h);
}

#[test]
fn each_call_carries_its_own_error() {
    // Unlike a last-error slot, results cannot be overwritten, cleared, or
    // consumed by unrelated calls: each return value is self-contained.
    let a = expect_error_envelope(aimux_provider_new(
        c("no-such-a").as_ptr(),
        c("k").as_ptr(),
        c("m").as_ptr(),
        ptr::null(),
    ));
    let b = expect_error_envelope(aimux_provider_new(
        c("no-such-b").as_ptr(),
        c("k").as_ptr(),
        c("m").as_ptr(),
        ptr::null(),
    ));
    assert!(
        a["error"].as_str().unwrap().contains("no-such-a"),
        "got: {a}"
    );
    assert!(
        b["error"].as_str().unwrap().contains("no-such-b"),
        "got: {b}"
    );
}

#[test]
fn concurrent_constructors_see_their_own_results() {
    // The failure-detail path needs no thread pinning: spawn constructors on
    // separate threads and verify each observes exactly its own error.
    let threads: Vec<_> = (0..8)
        .map(|i| {
            std::thread::spawn(move || {
                let name = format!("no-such-thread-{i}");
                let env = expect_error_envelope(aimux_provider_new(
                    c(&name).as_ptr(),
                    c("k").as_ptr(),
                    c("m").as_ptr(),
                    ptr::null(),
                ));
                assert!(
                    env["error"].as_str().unwrap().contains(&name),
                    "thread {i} got: {env}"
                );
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
}
