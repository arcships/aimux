//! Regression tests for issue #17: FFI constructors and prompt parsing must
//! preserve detailed error information (`aimux_error_t` transport).
//!
//! All tests are offline: they only exercise argument parsing and config
//! construction, never the network.

mod common;

use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_ffi::{
    AIMUX_E_INVALID_ARGUMENT, AIMUX_E_NO_SUCH_PROVIDER, aimux_abort_signal_abort,
    aimux_abort_signal_drop, aimux_abort_signal_new, aimux_azure_new, aimux_cohere_reranking_new,
    aimux_consume_stream_text, aimux_drop_handle, aimux_error_t, aimux_generate_object,
    aimux_generate_text, aimux_google_image_new, aimux_google_video_new, aimux_init_proxy,
    aimux_moa_new, aimux_openai_embedding_new, aimux_openai_files_new, aimux_openai_image_new,
    aimux_openai_new, aimux_openai_speech_new, aimux_openai_transcription_new, aimux_provider_new,
    aimux_register_providers, aimux_router_new, aimux_stream_text, aimux_tavily_search_new,
};
use common::{c, expect_aimux_error, expect_ffi_error, ok};

fn valid_handle() -> u64 {
    let mut h = 0;
    ok(
        aimux_openai_new(
            c("sk-test-fake-key").as_ptr(),
            c("gpt-4o-mini").as_ptr(),
            &mut h,
        ),
        "openai_new",
    );
    assert_ne!(h, 0);
    h
}

// ── Problem A: prompt_json serde detail ─────────────────────────────────────

#[test]
fn generate_text_invalid_prompt_json_carries_serde_detail() {
    let h = valid_handle();
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_generate_text(h, c("hello").as_ptr(), ptr::null(), &mut out);
    assert!(out.is_null(), "failure leaves *out_json NULL");
    // Malformed JSON text is this layer's finding, not an AiMuxError — a C
    // caller must not go looking at provider responses (JsonParse) for it.
    let m = expect_ffi_error(e, "generate_text");
    assert!(
        m.starts_with("prompt_json: invalid JSON:"),
        "expected serde detail in message, got: {m}"
    );
    aimux_drop_handle(h);
}

/// Wire-vs-schema split: text that does not parse is a C ABI failure;
/// text that parses but is not a prompt is AiMuxError::InvalidArgument.
#[test]
fn generate_text_prompt_json_wire_vs_schema() {
    let h = valid_handle();
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_generate_text(h, c("{not json").as_ptr(), ptr::null(), &mut out);
    assert!(expect_ffi_error(e, "wire").starts_with("prompt_json: invalid JSON:"));
    // Well-formed JSON of the wrong shape (a number is not a prompt).
    let e = aimux_generate_text(h, c("42").as_ptr(), ptr::null(), &mut out);
    let (code, m) = expect_aimux_error(e, "schema");
    assert_eq!(code, AIMUX_E_INVALID_ARGUMENT);
    assert!(m.contains("prompt_json:"), "{m}");
    assert!(out.is_null());
    aimux_drop_handle(h);
}

/// RFC-0042 §7.2 at the C boundary: a prompt part with an unknown `type` is
/// InvalidArgument naming the type, not "data did not match any variant of
/// untagged enum ModelPrompt".
#[test]
fn generate_text_unknown_part_type_is_named() {
    let h = valid_handle();
    let mut out: *mut c_char = ptr::null_mut();
    for prompt in [
        r#"[{"role":"user","content":[{"type":"no-such-part","text":"hi"}]}]"#,
        r#"{"prompt":[{"role":"user","content":[{"type":"no-such-part","text":"hi"}]}]}"#,
    ] {
        let e = aimux_generate_text(h, c(prompt).as_ptr(), ptr::null(), &mut out);
        let (code, m) = expect_aimux_error(e, prompt);
        assert_eq!(code, AIMUX_E_INVALID_ARGUMENT);
        assert!(
            m.contains("prompt_json:") && m.contains("unknown variant `no-such-part`"),
            "{m}"
        );
        assert!(out.is_null());
    }
    aimux_drop_handle(h);
}

#[test]
fn generate_text_null_prompt_json_fails() {
    let h = valid_handle();
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_generate_text(h, ptr::null(), ptr::null(), &mut out);
    assert_eq!(
        expect_ffi_error(e, "generate_text"),
        "prompt_json: must not be NULL"
    );
    aimux_drop_handle(h);
}

/// Options are camelCase JSON; a key this version does not read (aimux 0.5's
/// `max_retries`, a nested `total_ms`, a typo) fails the call with
/// InvalidArgument naming it, before any request.
#[test]
fn generate_text_rejects_unknown_option_keys() {
    let h = valid_handle();
    let mut out: *mut c_char = ptr::null_mut();
    for (opts, key) in [
        (r#"{"max_retries":0}"#, "max_retries"),
        (r#"{"timeout":{"total_ms":1000}}"#, "total_ms"),
        (r#"{"maxRetries":0,"temprature":0.2}"#, "temprature"),
    ] {
        let e = aimux_generate_text(h, c("\"hi\"").as_ptr(), c(opts).as_ptr(), &mut out);
        let (code, m) = expect_aimux_error(e, opts);
        assert_eq!(code, AIMUX_E_INVALID_ARGUMENT, "{opts}");
        assert!(
            m.contains("opts_json:") && m.contains(&format!("unknown field `{key}`")),
            "{opts}: {m}"
        );
        assert!(out.is_null());
    }
    aimux_drop_handle(h);
}

/// A NULL out-parameter is reported before anything else runs.
#[test]
fn null_out_json_is_an_ffi_error() {
    let h = valid_handle();
    let e = aimux_generate_text(h, c("\"hi\"").as_ptr(), ptr::null(), ptr::null_mut());
    assert_eq!(
        expect_ffi_error(e, "generate_text"),
        "out_json: must not be NULL"
    );
    let e = aimux_openai_new(c("k").as_ptr(), c("m").as_ptr(), ptr::null_mut());
    assert_eq!(
        expect_ffi_error(e, "openai_new"),
        "out_handle: must not be NULL"
    );
    aimux_drop_handle(h);
}

// ── Stream polarity + abort ─────────────────────────────────────────────────

static PART_COUNT: AtomicUsize = AtomicUsize::new(0);

extern "C-unwind" fn on_part(_json: *const c_char, _ctx: *mut std::ffi::c_void) {
    PART_COUNT.fetch_add(1, Ordering::SeqCst);
}
extern "C-unwind" fn on_done(_ctx: *mut std::ffi::c_void) {
    // not used in failure tests
}

#[test]
fn stream_text_invalid_prompt_returns_error() {
    let h = valid_handle();
    let e = aimux_stream_text(
        h,
        c("not-json").as_ptr(),
        ptr::null(),
        Some(on_part),
        Some(on_done),
        ptr::null_mut(),
    );
    assert!(expect_ffi_error(e, "stream_text").starts_with("prompt_json: invalid JSON:"));
    aimux_drop_handle(h);
}

// ── Constructors ────────────────────────────────────────────────────────────

#[test]
fn unknown_provider_fills_unknown_provider_code() {
    let mut h = 0;
    let e = aimux_provider_new(
        c("not-a-real-provider-xyz").as_ptr(),
        c("sk-x").as_ptr(),
        c("model").as_ptr(),
        ptr::null(),
        &mut h,
    );
    assert_eq!(h, 0);
    assert_eq!(
        expect_aimux_error(e, "provider_new").0,
        AIMUX_E_NO_SUCH_PROVIDER
    );
}

#[test]
fn null_openai_args_are_ffi_errors() {
    let mut h = 0;
    let e = aimux_openai_new(ptr::null(), ptr::null(), &mut h);
    assert_eq!(h, 0);
    assert_eq!(
        expect_ffi_error(e, "openai_new"),
        "api_key: must not be NULL"
    );
}

#[test]
fn invalid_config_json_reports_error() {
    let mut h = 0;
    let e = aimux_provider_new(
        c("openai").as_ptr(),
        c("sk-x").as_ptr(),
        c("gpt-4o").as_ptr(),
        c("{not json").as_ptr(),
        &mut h,
    );
    assert_eq!(h, 0);
    assert!(expect_ffi_error(e, "provider_new").starts_with("config_json: invalid JSON:"));
}

#[test]
fn azure_null_deployment_is_an_ffi_error() {
    let mut h = 0;
    let e = aimux_azure_new(
        c("sk").as_ptr(),
        c("res").as_ptr(),
        ptr::null(),
        ptr::null(),
        &mut h,
    );
    assert_eq!(h, 0);
    assert_eq!(
        expect_ffi_error(e, "azure_new"),
        "deployment: must not be NULL"
    );
}

/// `register_providers`: malformed text is a C ABI failure; a well-formed
/// document the registry rejects is AiMuxError::InvalidArgument.
#[test]
fn register_providers_wire_vs_schema() {
    let e = aimux_register_providers(c("{not json").as_ptr());
    assert!(
        expect_ffi_error(e, "register_providers (wire)").starts_with("config_json: invalid JSON:")
    );
    let e = aimux_register_providers(c(r#"{"providers": 42}"#).as_ptr());
    let (code, m) = expect_aimux_error(e, "register_providers (schema)");
    assert_eq!(code, AIMUX_E_INVALID_ARGUMENT);
    assert!(m.contains("config_json:"), "{m}");
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

// ── R4-1: multimodal constructors honor the error contract ─────────────────
//
// The multimodal provider construction methods (embedding / speech / image /
// transcription / files / reranking / video / search) return the model
// *directly* rather than `Result`: they only stash the model id + config and
// perform no validation or I/O, so construction is infallible (the type system
// enforces this — the FFI compiles, so none of these calls can yield `Err`).
// The only failure path in these FFI constructors is a null / invalid-UTF-8
// argument — a C ABI failure.
//
// Note: an *empty* C string (`""`) is a valid non-null argument and does NOT
// fail here — it constructs a model with a blank key (the error surfaces later,
// at network time). Only null / invalid-UTF-8 pointers are failure triggers.

type Ctor3 = extern "C" fn(*const c_char, *const c_char, *mut u64) -> *mut aimux_error_t;

/// Model-id-bearing multimodal constructors: `(api_key, model_id, out) -> err`.
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
fn multimodal_null_api_key_is_an_ffi_error() {
    let model = c("text-embedding-3-small");
    for &(name, ctor) in MODEL_ID_CTORS {
        let mut h = 0;
        let e = ctor(ptr::null(), model.as_ptr(), &mut h);
        assert_eq!(h, 0, "{name}");
        assert_eq!(expect_ffi_error(e, name), "api_key: must not be NULL");
    }
    // Single-key constructor (no model_id).
    let mut h = 0;
    let e = aimux_openai_files_new(ptr::null(), &mut h);
    assert_eq!(h, 0);
    assert_eq!(
        expect_ffi_error(e, "openai_files"),
        "api_key: must not be NULL"
    );
    // Tavily ignores model_id but still validates api_key.
    let e = aimux_tavily_search_new(ptr::null(), ptr::null(), &mut h);
    assert_eq!(h, 0);
    assert_eq!(
        expect_ffi_error(e, "tavily_search"),
        "api_key: must not be NULL"
    );
}

#[test]
fn multimodal_null_model_id_is_an_ffi_error() {
    let key = c("sk-test-fake-key");
    for &(name, ctor) in MODEL_ID_CTORS {
        let mut h = 0;
        let e = ctor(key.as_ptr(), ptr::null(), &mut h);
        assert_eq!(h, 0, "{name}");
        assert_eq!(expect_ffi_error(e, name), "model_id: must not be NULL");
    }
}

#[test]
fn multimodal_invalid_utf8_api_key_is_an_ffi_error() {
    // 0xFF is invalid UTF-8.
    let bad_key = CString::new(b"sk-\xff".to_vec()).unwrap();
    let model = c("text-embedding-3-small");
    for &(name, ctor) in MODEL_ID_CTORS {
        let mut h = 0;
        let e = ctor(bad_key.as_ptr(), model.as_ptr(), &mut h);
        assert_eq!(h, 0, "{name}");
        assert_eq!(expect_ffi_error(e, name), "api_key: must be valid UTF-8");
    }
}

#[test]
fn multimodal_empty_key_construction_is_infallible() {
    // The R4-1 finding suspected construction could fail (e.g. on a blank key)
    // and bypass the error contract. It cannot: the multimodal provider
    // constructors return the model directly (not `Result`), so even an empty
    // api_key — a valid non-null C string — yields a non-zero handle and NULL.
    // The error would only surface later, at network time. This test pins
    // that behavior so a future "fix" cannot silently turn a success path into
    // a failure.
    let mut h = 0;
    ok(
        aimux_openai_embedding_new(c("").as_ptr(), c("text-embedding-3-small").as_ptr(), &mut h),
        "openai_embedding_new (empty key)",
    );
    assert_ne!(h, 0, "empty api_key still constructs (infallible)");
    aimux_drop_handle(h);
}

// ── Arity smoke tests for new FFI symbols (M11/M12/M6) ────────────────────────

#[test]
fn generate_object_bad_handle_fails() {
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_generate_object(99999, c("{}").as_ptr(), ptr::null(), &mut out);
    assert!(out.is_null(), "expected NULL on bad handle");
    // A dead handle is this layer's finding, and it says which handle.
    assert_eq!(
        expect_ffi_error(e, "generate_object"),
        "invalid or expired model handle"
    );
}

#[test]
fn consume_stream_text_bad_handle_fails() {
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_consume_stream_text(99999, c("{}").as_ptr(), ptr::null(), &mut out);
    assert!(out.is_null(), "expected NULL on bad handle");
    assert_eq!(
        expect_ffi_error(e, "consume_stream_text"),
        "invalid or expired model handle"
    );
}

#[test]
fn init_proxy_valid_json_succeeds() {
    ok(
        aimux_init_proxy(c(r#"{"http_url": null}"#).as_ptr()),
        "init_proxy (idempotent)",
    );
}

#[test]
fn init_proxy_null_json_fails() {
    assert_eq!(
        expect_ffi_error(aimux_init_proxy(ptr::null()), "init_proxy"),
        "config_json: must not be NULL"
    );
}

/// A NULL callback is a null pointer like any other required argument: this
/// layer reports it (with the parameter name) instead of dereferencing it.
#[test]
fn stream_text_null_callback_is_an_ffi_error() {
    let h = valid_handle();
    let e = aimux_stream_text(
        h,
        c("\"hi\"").as_ptr(),
        ptr::null(),
        None,
        Some(on_done),
        ptr::null_mut(),
    );
    assert_eq!(expect_ffi_error(e, "on_part"), "on_part: must not be NULL");
    let e = aimux_stream_text(
        h,
        c("\"hi\"").as_ptr(),
        ptr::null(),
        Some(on_part),
        None,
        ptr::null_mut(),
    );
    assert_eq!(expect_ffi_error(e, "on_done"), "on_done: must not be NULL");
    aimux_drop_handle(h);
}

/// The router and MoA configs are camelCase, as the header documents, and
/// strict: aimux 0.5's `provider_name` / `fail_mode` fail naming the key
/// instead of leaving the default in place.
#[test]
fn composite_configs_reject_unknown_keys() {
    let h = valid_handle();
    let children = [h];
    let mut router = 0;
    ok(
        aimux_router_new(
            children.as_ptr(),
            children.len(),
            c(r#"{"router":"rule","fallback":"none","providerName":"r","modelId":"m"}"#).as_ptr(),
            &mut router,
        ),
        "router_new",
    );
    assert_ne!(router, 0);
    aimux_drop_handle(router);

    for (config, key) in [
        (r#"{"provider_name":"r"}"#, "provider_name"),
        (r#"{"modelId":"m","model_id":"m"}"#, "model_id"),
    ] {
        let mut router = 0;
        let e = aimux_router_new(
            children.as_ptr(),
            children.len(),
            c(config).as_ptr(),
            &mut router,
        );
        let (code, m) = expect_aimux_error(e, config);
        assert_eq!(code, AIMUX_E_INVALID_ARGUMENT, "{config}");
        assert!(
            m.contains("config_json:") && m.contains(&format!("unknown field `{key}`")),
            "{config}: {m}"
        );
        assert_eq!(router, 0);
    }

    let mut moa = 0;
    let e = aimux_moa_new(
        ptr::null(),
        0,
        h,
        c(r#"{"fail_mode":"fail_fast"}"#).as_ptr(),
        &mut moa,
    );
    let (code, m) = expect_aimux_error(e, "moa_new");
    assert_eq!(code, AIMUX_E_INVALID_ARGUMENT);
    assert!(
        m.contains("config_json:") && m.contains("unknown field `fail_mode`"),
        "{m}"
    );
    assert_eq!(moa, 0);
    aimux_drop_handle(h);
}
