//! Regression tests for issue #17: FFI constructors and prompt parsing must
//! preserve detailed error information.
//!
//! - Problem A: `prompt_json` parse errors must carry the `serde_json::Error`
//!   detail (like the adjacent `opts_json` branch already did).
//! - Problem B: constructor failures (unknown provider, bad config JSON,
//!   null arguments) are recorded in a thread-local slot and read back with
//!   `aimux_last_error()` as a full error JSON envelope.
//!
//! All tests are offline: they only exercise argument parsing and config
//! construction, never the network.

use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::Mutex;

use serde_json::Value;

use aimux_ffi::{
    aimux_azure_new, aimux_drop_handle, aimux_free_string, aimux_generate_text, aimux_last_error,
    aimux_openai_new, aimux_provider_new, aimux_stream_text,
};

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

/// Take the last error as an owned `String` (frees the FFI pointer).
fn take_last_error() -> Option<String> {
    let p = aimux_last_error();
    if p.is_null() {
        return None;
    }
    // SAFETY: aimux_last_error returns a NUL-terminated string owned by us.
    let s = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string();
    // SAFETY: p was produced by CString::into_raw inside aimux_last_error.
    unsafe { aimux_free_string(p) };
    Some(s)
}

fn take_envelope() -> Option<Value> {
    take_last_error().map(|json| {
        serde_json::from_str(&json).expect("aimux_last_error must return a valid JSON envelope")
    })
}

/// A model handle that can be used with `aimux_generate_text` / `aimux_stream_text`.
fn valid_handle() -> u64 {
    let h = aimux_openai_new(c("sk-test-fake-key").as_ptr(), c("gpt-4o-mini").as_ptr());
    assert!(h != 0, "expected a valid handle for the test fixture");
    h
}

// ── Problem A: prompt_json serde detail ─────────────────────────────────────

#[test]
fn generate_text_invalid_prompt_json_carries_serde_detail() {
    let h = valid_handle();
    let out = aimux_generate_text(h, c("hello").as_ptr(), ptr::null());
    assert!(!out.is_null(), "expected an error JSON string");
    // SAFETY: aimux_generate_text returns a NUL-terminated string we own.
    let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
    unsafe { aimux_free_string(out) };

    let env: Value = serde_json::from_str(&json).expect("error must be a JSON envelope");
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
    assert!(!out.is_null());
    // SAFETY: as above.
    let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
    unsafe { aimux_free_string(out) };
    assert!(json.contains("invalid prompt_json"), "got: {json}");
    aimux_drop_handle(h);
}

static STREAM_ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());

extern "C" fn on_part(_json: *const std::os::raw::c_char) {}
extern "C" fn on_done() {}

extern "C" fn on_error(json: *const std::os::raw::c_char) {
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

// ── Problem B: constructor failures via aimux_last_error ────────────────────

#[test]
fn unknown_provider_reports_full_error_envelope() {
    let h = aimux_provider_new(
        c("this-provider-does-not-exist").as_ptr(),
        c("sk-x").as_ptr(),
        c("gpt-4o-mini").as_ptr(),
        ptr::null(),
    );
    assert_eq!(h, 0, "unknown provider must fail");

    let env = take_envelope().expect("last_error must be set after a failed constructor");
    let msg = env["error"].as_str().unwrap();
    assert!(msg.contains("unknown provider"), "got: {msg}");
    assert!(msg.contains("available providers"), "got: {msg}");
    assert_eq!(env["error_type"], "UnknownProvider");
    assert!(env["status_code"].is_null());
}

#[test]
fn invalid_config_json_reports_json_error_type() {
    let h = aimux_provider_new(
        c("openai").as_ptr(),
        c("sk-x").as_ptr(),
        c("gpt-4o-mini").as_ptr(),
        c("{bad json").as_ptr(),
    );
    assert_eq!(h, 0, "malformed config_json must fail");

    let env = take_envelope().expect("last_error must be set");
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
    let h = aimux_openai_new(ptr::null(), ptr::null());
    assert_eq!(h, 0);

    let env = take_envelope().expect("last_error must be set");
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
    // boundary with an InvalidArgument envelope (the Azure provider's own
    // InvalidArgument can't be reached from the FFI layer because the
    // argument is mandatory).
    let h = aimux_azure_new(
        c("k").as_ptr(),
        ptr::null(),
        c("deploy").as_ptr(),
        ptr::null(),
    );
    assert_eq!(h, 0);

    let env = take_envelope().expect("last_error must be set");
    assert_eq!(env["error_type"], "InvalidArgument");
}

// ── TLS semantics ───────────────────────────────────────────────────────────

#[test]
fn successful_constructor_clears_previous_error() {
    // 1. Fail and leave the error unread.
    assert_eq!(
        aimux_provider_new(
            c("no-such-provider").as_ptr(),
            c("k").as_ptr(),
            c("m").as_ptr(),
            ptr::null(),
        ),
        0
    );
    assert!(take_last_error().is_some(), "precondition: error recorded");

    // 2. Fail again so the slot is repopulated, then succeed WITHOUT reading.
    assert_eq!(
        aimux_provider_new(
            c("also-no-such").as_ptr(),
            c("k").as_ptr(),
            c("m").as_ptr(),
            ptr::null(),
        ),
        0
    );
    let h = aimux_provider_new(
        c("groq").as_ptr(),
        c("sk-test").as_ptr(),
        c("llama-3.3-70b").as_ptr(),
        ptr::null(),
    );
    assert!(
        h != 0,
        "a registered provider with a fake key must construct"
    );

    // 3. The success must have cleared the stale error.
    assert!(
        take_last_error().is_none(),
        "successful constructor must clear the previous error"
    );
    aimux_drop_handle(h);
}

#[test]
fn last_error_is_overwritten_and_read_once() {
    aimux_provider_new(
        c("no-such-a").as_ptr(),
        c("k").as_ptr(),
        c("m").as_ptr(),
        ptr::null(),
    );
    aimux_provider_new(
        c("no-such-b").as_ptr(),
        c("k").as_ptr(),
        c("m").as_ptr(),
        ptr::null(),
    );

    // Last write wins.
    let env = take_envelope().expect("last_error must be set");
    assert!(
        env["error"].as_str().unwrap().contains("no-such-b"),
        "got: {env}"
    );

    // Read-and-clear: a second read returns NULL.
    assert!(take_last_error().is_none(), "second read must be NULL");
}

#[test]
fn last_error_is_thread_local() {
    let t1 = std::thread::spawn(|| {
        aimux_provider_new(
            c("no-such-thread-a").as_ptr(),
            c("k").as_ptr(),
            c("m").as_ptr(),
            ptr::null(),
        );
        take_last_error().expect("thread 1 must see its own error")
    });
    let t2 = std::thread::spawn(|| {
        aimux_provider_new(
            c("no-such-thread-b").as_ptr(),
            c("k").as_ptr(),
            c("m").as_ptr(),
            ptr::null(),
        );
        take_last_error().expect("thread 2 must see its own error")
    });

    let (e1, e2) = (t1.join().unwrap(), t2.join().unwrap());
    assert!(e1.contains("no-such-thread-a"), "thread 1 got: {e1}");
    assert!(e2.contains("no-such-thread-b"), "thread 2 got: {e2}");
}

#[test]
fn no_error_returns_null_without_a_constructor_call() {
    // The main thread never failed a constructor here; the slot must be empty
    // (nothing to read, nothing to free).
    assert!(take_last_error().is_none());
}
