//! Regression tests for issue #17: FFI constructors and prompt parsing must
//! preserve detailed error information (C AimuxError out-param transport).
//!
//! All tests are offline: they only exercise argument parsing and config
//! construction, never the network.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_ffi::{
    AIMUX_E_INVALID_ARGUMENT, AIMUX_E_JSON_PARSE, AIMUX_E_NO_SUCH_PROVIDER, AIMUX_E_OTHER,
    AIMUX_OK, CAimuxError, aimux_abort_signal_abort, aimux_abort_signal_drop,
    aimux_abort_signal_new, aimux_azure_new, aimux_cohere_reranking_new, aimux_consume_stream_text,
    aimux_drop_handle, aimux_free_string, aimux_generate_object, aimux_generate_text,
    aimux_google_image_new, aimux_google_video_new, aimux_init_proxy, aimux_openai_embedding_new,
    aimux_openai_files_new, aimux_openai_image_new, aimux_openai_new, aimux_openai_speech_new,
    aimux_openai_transcription_new, aimux_provider_new, aimux_stream_text, aimux_tavily_search_new,
};

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn clear_err() -> CAimuxError {
    CAimuxError {
        code: AIMUX_OK,
        status: -1,
        retry_ms: -1,
        message: ptr::null_mut(),
        error_value: std::ptr::null_mut(),
        reserved: [std::ptr::null_mut(); 1],
    }
}

fn msg(err: &CAimuxError) -> String {
    if err.message.is_null() {
        return String::new();
    }
    // SAFETY: on failure, message is an owned NUL-terminated C string.
    unsafe { CStr::from_ptr(err.message) }
        .to_str()
        .unwrap_or("")
        .to_string()
}

/// Release err.message per the contract (aimux_free_string; NULL is safe).
fn free_err(err: &mut CAimuxError) {
    unsafe { aimux_free_string(err.message) };
    err.message = ptr::null_mut();
}

/// Expect failure: return is sentinel, err is filled.
fn expect_fail_u64(h: u64, err: &CAimuxError) {
    assert_eq!(h, 0, "expected handle 0 on failure");
    assert_ne!(err.code, AIMUX_OK, "expected non-OK code, got {}", err.code);
    assert!(!msg(err).is_empty(), "expected non-empty message");
}

fn expect_fail_ptr(ptr: *mut c_char, err: &CAimuxError) {
    assert!(ptr.is_null(), "expected NULL string on failure");
    assert_ne!(err.code, AIMUX_OK, "expected non-OK code");
    assert!(!msg(err).is_empty());
}

fn expect_handle(h: u64) -> u64 {
    assert_ne!(h, 0, "expected non-zero handle");
    h
}

fn valid_handle() -> u64 {
    let mut err = clear_err();
    let h = aimux_openai_new(
        c("sk-test-fake-key").as_ptr(),
        c("gpt-4o-mini").as_ptr(),
        &mut err,
    );
    expect_handle(h)
}

// ── Problem A: prompt_json serde detail ─────────────────────────────────────

#[test]
fn generate_text_invalid_prompt_json_carries_serde_detail() {
    let h = valid_handle();
    let mut err = clear_err();
    let out = aimux_generate_text(h, c("hello").as_ptr(), ptr::null(), &mut err);
    expect_fail_ptr(out, &err);
    let m = msg(&err);
    assert!(
        m.starts_with("invalid prompt_json:") || m.contains("invalid prompt"),
        "expected serde detail in message, got: {m}"
    );
    assert_eq!(err.code, AIMUX_E_JSON_PARSE, "parse failures map to Json");
    free_err(&mut err);
    aimux_drop_handle(h);
}

#[test]
fn generate_text_null_prompt_json_fails() {
    let h = valid_handle();
    let mut err = clear_err();
    let out = aimux_generate_text(h, ptr::null(), ptr::null(), &mut err);
    expect_fail_ptr(out, &err);
    assert_eq!(
        err.code, AIMUX_E_INVALID_ARGUMENT,
        "null args map to InvalidArgument"
    );
    free_err(&mut err);
    aimux_drop_handle(h);
}

// ── Stream polarity + abort ─────────────────────────────────────────────────

static PART_COUNT: AtomicUsize = AtomicUsize::new(0);

extern "C" fn on_part(_json: *const c_char, _ctx: *mut std::ffi::c_void) {
    PART_COUNT.fetch_add(1, Ordering::SeqCst);
}
extern "C" fn on_done(_ctx: *mut std::ffi::c_void) {
    // not used in failure tests
}

#[test]
fn stream_text_invalid_prompt_returns_zero_and_fills_err() {
    let h = valid_handle();
    let mut err = clear_err();
    let rc = aimux_stream_text(
        h,
        c("not-json").as_ptr(),
        ptr::null(),
        on_part,
        on_done,
        ptr::null_mut(),
        &mut err,
    );
    assert_eq!(rc, 0, "stream must return 0 on failure");
    assert_eq!(err.code, AIMUX_E_JSON_PARSE);
    assert!(!msg(&err).is_empty());
    free_err(&mut err);
    aimux_drop_handle(h);
}

// ── Constructors ────────────────────────────────────────────────────────────

#[test]
fn unknown_provider_fills_unknown_provider_code() {
    let mut err = clear_err();
    let h = aimux_provider_new(
        c("not-a-real-provider-xyz").as_ptr(),
        c("sk-x").as_ptr(),
        c("model").as_ptr(),
        ptr::null(),
        &mut err,
    );
    expect_fail_u64(h, &err);
    assert_eq!(err.code, AIMUX_E_NO_SUCH_PROVIDER);
    free_err(&mut err);
}

#[test]
fn null_openai_args_invalid_argument() {
    let mut err = clear_err();
    let h = aimux_openai_new(ptr::null(), ptr::null(), &mut err);
    expect_fail_u64(h, &err);
    assert_eq!(err.code, AIMUX_E_INVALID_ARGUMENT);
    free_err(&mut err);
}

#[test]
fn invalid_config_json_reports_error() {
    let mut err = clear_err();
    let h = aimux_provider_new(
        c("openai").as_ptr(),
        c("sk-x").as_ptr(),
        c("gpt-4o").as_ptr(),
        c("{not json").as_ptr(),
        &mut err,
    );
    expect_fail_u64(h, &err);
    assert_eq!(err.code, AIMUX_E_JSON_PARSE);
    free_err(&mut err);
}

#[test]
fn azure_null_deployment_invalid_argument() {
    let mut err = clear_err();
    let h = aimux_azure_new(
        c("sk").as_ptr(),
        c("res").as_ptr(),
        ptr::null(),
        ptr::null(),
        &mut err,
    );
    expect_fail_u64(h, &err);
    assert_eq!(err.code, AIMUX_E_INVALID_ARGUMENT);
    free_err(&mut err);
}

#[test]
fn success_leaves_err_untouched() {
    // The load-bearing clause: on success the callee does NOT touch *err.
    // Prefill with sentinel garbage and assert every field survives.
    let mut err = CAimuxError {
        code: AIMUX_E_OTHER,
        status: 42,
        retry_ms: 7,
        message: ptr::null_mut(),
        error_value: std::ptr::null_mut(),
        reserved: [std::ptr::null_mut(); 1],
    };
    let h = aimux_openai_new(c("sk-test").as_ptr(), c("gpt-4o-mini").as_ptr(), &mut err);
    assert_ne!(h, 0);
    assert_eq!(err.code, AIMUX_E_OTHER, "success must not write err.code");
    assert_eq!(err.status, 42);
    assert_eq!(err.retry_ms, 7);
    assert!(err.message.is_null());
    aimux_drop_handle(h);
}

// Keep abort helpers linked (smoke).
#[test]
fn abort_signal_new_drop_smoke() {
    let a = aimux_abort_signal_new();
    assert_ne!(a, 0);
    aimux_abort_signal_abort(a);
    aimux_abort_signal_drop(a);
    let _ = PART_COUNT.load(Ordering::SeqCst);
}

// ── R4-1: multimodal constructors honor the AimuxError contract ─────────────
//
// The multimodal provider construction methods (embedding / speech / image /
// transcription / files / reranking / video / search) return the model
// *directly* rather than `Result`: they only stash the model id + config and
// perform no validation or I/O, so construction is infallible (the type system
// enforces this — the FFI compiles, so none of these calls can yield `Err`).
// The only failure path in these FFI constructors is a null / invalid-UTF-8
// argument, handled by `fail_invalid_args` → returns 0 and fills
// `err.code = AIMUX_E_INVALID_ARGUMENT`.
//
// Note: an *empty* C string (`""`) is a valid non-null argument and does NOT
// fail here — it constructs a model with a blank key (the error surfaces later,
// at network time). Only null / invalid-UTF-8 pointers are failure triggers.

type Ctor3 = extern "C" fn(*const c_char, *const c_char, *mut CAimuxError) -> u64;

/// Model-id-bearing multimodal constructors: `(api_key, model_id, err) -> u64`.
static MODEL_ID_CTORS: &[(&str, Ctor3)] = &[
    ("openai_embedding", aimux_openai_embedding_new),
    ("openai_speech", aimux_openai_speech_new),
    ("openai_image", aimux_openai_image_new),
    ("google_image", aimux_google_image_new),
    ("openai_transcription", aimux_openai_transcription_new),
    ("cohere_reranking", aimux_cohere_reranking_new),
    ("google_video", aimux_google_video_new),
];

#[test]
fn multimodal_null_api_key_fills_invalid_argument() {
    let model = c("text-embedding-3-small");
    for &(name, ctor) in MODEL_ID_CTORS {
        let mut err = clear_err();
        let h = ctor(ptr::null(), model.as_ptr(), &mut err);
        expect_fail_u64(h, &err);
        assert_eq!(err.code, AIMUX_E_INVALID_ARGUMENT, "{name}: null api_key");
        free_err(&mut err);
    }
    // Single-key constructor (no model_id).
    let mut err = clear_err();
    let h = aimux_openai_files_new(ptr::null(), &mut err);
    expect_fail_u64(h, &err);
    assert_eq!(
        err.code, AIMUX_E_INVALID_ARGUMENT,
        "openai_files: null api_key"
    );
    free_err(&mut err);
    // Tavily ignores model_id but still validates api_key.
    let mut err = clear_err();
    let h = aimux_tavily_search_new(ptr::null(), ptr::null(), &mut err);
    expect_fail_u64(h, &err);
    assert_eq!(
        err.code, AIMUX_E_INVALID_ARGUMENT,
        "tavily_search: null api_key"
    );
    free_err(&mut err);
}

#[test]
fn multimodal_null_model_id_fills_invalid_argument() {
    let key = c("sk-test-fake-key");
    for &(name, ctor) in MODEL_ID_CTORS {
        let mut err = clear_err();
        let h = ctor(key.as_ptr(), ptr::null(), &mut err);
        expect_fail_u64(h, &err);
        assert_eq!(err.code, AIMUX_E_INVALID_ARGUMENT, "{name}: null model_id");
        free_err(&mut err);
    }
}

#[test]
fn multimodal_invalid_utf8_api_key_fills_invalid_argument() {
    // 0xFF is invalid UTF-8 → cstr_to_string returns None → fail_invalid_args.
    let bad_key = CString::new(b"sk-\xff".to_vec()).unwrap();
    let model = c("text-embedding-3-small");
    for &(name, ctor) in MODEL_ID_CTORS {
        let mut err = clear_err();
        let h = ctor(bad_key.as_ptr(), model.as_ptr(), &mut err);
        expect_fail_u64(h, &err);
        assert_eq!(
            err.code, AIMUX_E_INVALID_ARGUMENT,
            "{name}: invalid utf-8 api_key"
        );
        free_err(&mut err);
    }
}

#[test]
fn multimodal_empty_key_construction_is_infallible() {
    // The R4-1 finding suspected construction could fail (e.g. on a blank key)
    // and bypass the error contract. It cannot: the multimodal provider
    // constructors return the model directly (not `Result`), so even an empty
    // api_key — a valid non-null C string — yields a non-zero handle and leaves
    // `err` untouched. The error would only surface later, at network time.
    // This test pins that behavior so a future "fix" cannot silently turn a
    // success path into a failure.
    let mut err = clear_err();
    let h = aimux_openai_embedding_new(
        c("").as_ptr(),
        c("text-embedding-3-small").as_ptr(),
        &mut err,
    );
    assert_ne!(h, 0, "empty api_key still constructs (infallible)");
    assert_eq!(err.code, AIMUX_OK, "err untouched on success");
    aimux_drop_handle(h);
}

// ── Arity smoke tests for new FFI symbols (M11/M12/M6) ────────────────────────

#[test]
fn generate_object_bad_handle_fails() {
    let mut err = clear_err();
    let out = aimux_generate_object(99999, c("{}").as_ptr(), ptr::null(), &mut err);
    assert!(out.is_null(), "expected NULL on bad handle");
    assert_eq!(err.code, AIMUX_E_INVALID_ARGUMENT);
    free_err(&mut err);
}

#[test]
fn consume_stream_text_bad_handle_fails() {
    let mut err = clear_err();
    let out = aimux_consume_stream_text(99999, c("{}").as_ptr(), ptr::null(), &mut err);
    assert!(out.is_null(), "expected NULL on bad handle");
    assert_eq!(err.code, AIMUX_E_INVALID_ARGUMENT);
    free_err(&mut err);
}

#[test]
fn init_proxy_valid_json_succeeds() {
    let mut err = clear_err();
    let rc = aimux_init_proxy(c(r#"{"http_url": null}"#).as_ptr(), &mut err);
    assert!(
        rc == 0 || rc == 1,
        "init_proxy should return 0 or 1, got {rc}"
    );
    // err should be untouched on success (rc != 0) — don't free if untouched.
    if rc == 0 {
        free_err(&mut err);
    }
}

#[test]
fn init_proxy_null_json_fails() {
    let mut err = clear_err();
    let rc = aimux_init_proxy(ptr::null(), &mut err);
    assert_eq!(rc, 0, "null config_json must fail");
    assert_eq!(err.code, AIMUX_E_INVALID_ARGUMENT);
    free_err(&mut err);
}
