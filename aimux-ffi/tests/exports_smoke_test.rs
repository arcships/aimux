//! FFI export smoke harness (issue #119).
//!
//! A no-network smoke traversal of the C ABI surface in `aimux-ffi/src/lib.rs`.
//! It does NOT verify protocol behaviour — wiremock round-trips live in
//! `aimux-providers/tests/e2e_test.rs` — it verifies that every exported
//! symbol can be called from a C-style caller without panicking and honours
//! the ABI contract:
//!
//! - **Constructor class** (`*_new` / `*_new_with_base` / composites): fake
//!   API key + `base_url` pointing at a loopback port nothing listens on
//!   (`http://127.0.0.1:1`) → must return a non-zero handle, which is then
//!   released with [`aimux_ffi::aimux_drop_handle`]. Constructors that cannot
//!
//! Note: multimodal session smoke calls use the default RetryConfig
//! (2 retries) against a refused port; under `--test-threads=1` this adds
//! roughly a minute of backoff sleep — run the default parallel profile.
//!   take a base URL (e.g. `aimux_provider_from_env`) are only checked for a
//!   clean handle-or-error outcome.
//! - **Utility class**: called directly with minimal/empty arguments; asserts
//!   a sane return value, never a panic.
//! - **Session class** (generate / stream / embed / speech / image /
//!   transcription / rerank / video / search / files): handle + minimal
//!   options against the unreachable base URL → must return the failure
//!   sentinel with `CAimuxError` filled (a clean error path). Text calls pass
//!   `{"max_retries":0}` to skip the retry backoff; the multimodal option
//!   structs expose no retry override, so those tests run as separate
//!   `#[test]`s to parallelise the default backoff.
//! - Every `CAimuxError.message` / `error_value` and every returned JSON
//!   string is released with [`aimux_ffi::aimux_free_string`].
//!
//! ## Coverage
//!
//! 96 of the 96 `#[unsafe(no_mangle)]` exports in `src/lib.rs` are exercised
//! (the constructor and utility classes in full; the session class one
//! representative call per export — every export is still touched):
//! - constructors: 49 (9 native protocol pairs + multimodal pairs +
//!   provider/trace/router/moa/abort/mock-replay factories)
//! - session-class calls: 25 (text generate ×4, stream ×4, multimodal ×8,
//!   transcription session ×5, provider discovery + catalogue + codex ×4)
//! - utility: 22 (handle/string lifecycle, session + trace queries,
//!   recording ×6, logging, proxy, provider registration)

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_ffi::{
    AIMUX_E_JSON_PARSE, AIMUX_OK, CAimuxError, aimux_abort_signal_abort, aimux_abort_signal_drop,
    aimux_abort_signal_new, aimux_anthropic_aws_new, aimux_anthropic_aws_new_with_base,
    aimux_anthropic_new, aimux_anthropic_new_with_base, aimux_azure_new, aimux_azure_new_with_base,
    aimux_bedrock_new, aimux_bedrock_new_with_base, aimux_codex_refresh,
    aimux_cohere_embedding_new, aimux_cohere_embedding_new_with_base, aimux_cohere_new,
    aimux_cohere_new_with_base, aimux_cohere_reranking_new, aimux_cohere_reranking_new_with_base,
    aimux_consume_stream_text, aimux_drop_handle, aimux_embed, aimux_file_upload,
    aimux_free_string, aimux_generate_object, aimux_generate_text, aimux_generate_text_as_openai,
    aimux_get_model_specs, aimux_google_embedding_new, aimux_google_embedding_new_with_base,
    aimux_google_image_new, aimux_google_image_new_with_base, aimux_google_video_new,
    aimux_google_video_new_with_base, aimux_image_generate, aimux_init_logging, aimux_init_proxy,
    aimux_init_recording, aimux_init_recording_ring, aimux_init_recording_ring_default,
    aimux_list_sessions, aimux_mistral_new, aimux_mistral_new_with_base, aimux_moa_new,
    aimux_mock_replay_new, aimux_openai_embedding_new, aimux_openai_embedding_new_with_base,
    aimux_openai_files_new, aimux_openai_files_new_with_base, aimux_openai_image_new,
    aimux_openai_image_new_with_base, aimux_openai_new, aimux_openai_new_with_base,
    aimux_openai_speech_new, aimux_openai_speech_new_with_base, aimux_openai_transcription_new,
    aimux_openai_transcription_new_with_base, aimux_provider_from_env, aimux_provider_handle_new,
    aimux_provider_list_models, aimux_provider_model, aimux_provider_new, aimux_recording_flush,
    aimux_recording_stop, aimux_recording_try_flush, aimux_register_providers, aimux_rerank,
    aimux_router_new, aimux_search, aimux_session_calls, aimux_session_infer_init,
    aimux_session_store_init, aimux_speech_generate, aimux_stream_text,
    aimux_stream_text_as_openai, aimux_stream_text_as_openai_with_abort,
    aimux_stream_text_with_abort, aimux_tavily_search_new, aimux_tavily_search_new_with_base,
    aimux_trace_aggregate, aimux_trace_clear, aimux_trace_export_jsonl, aimux_trace_new,
    aimux_trace_new_audited, aimux_trace_session_chain, aimux_trace_session_trajectory,
    aimux_transcription_generate, aimux_transcription_input_done, aimux_transcription_next_part,
    aimux_transcription_push_audio, aimux_transcription_session_drop,
    aimux_transcription_session_new, aimux_vertex_new, aimux_vertex_new_with_base,
    aimux_video_generate, aimux_xai_new, aimux_xai_new_with_base,
};

/// Loopback port that nothing listens on: connections are refused (fast,
/// deterministic) on every platform. Through an HTTP proxy it surfaces as a
/// proxy error — either way a clean transport error, never a hang.
const UNREACHABLE: &str = "http://127.0.0.1:1";

const FAKE_KEY: &str = "sk-ffi-smoke-fake-key";

// ── harness helpers ─────────────────────────────────────────────────────────

fn c(s: &str) -> CString {
    CString::new(s).expect("CString::new: no interior NUL in harness literals")
}

fn fresh_err() -> CAimuxError {
    CAimuxError {
        code: AIMUX_OK,
        status: -1,
        retry_ms: -1,
        message: ptr::null_mut(),
        error_value: ptr::null_mut(),
        reserved: [ptr::null_mut(); 1],
    }
}

fn err_message(err: &CAimuxError) -> String {
    if err.message.is_null() {
        return String::new();
    }
    // SAFETY: `message` is an owned NUL-terminated string filled by the FFI
    // layer for this error slot (aimux-error.h contract).
    unsafe { CStr::from_ptr(err.message) }
        .to_str()
        .unwrap_or("")
        .to_string()
}

/// Release the strings the FFI layer allocated inside `err`.
fn release_err(err: &mut CAimuxError) {
    // SAFETY: both pointers, when non-null, were produced by `into_cstring_raw`
    // on the Rust side and are owned by this error slot.
    unsafe {
        aimux_free_string(err.message);
        aimux_free_string(err.error_value);
    }
    err.message = ptr::null_mut();
    err.error_value = ptr::null_mut();
}

fn expect_handle(h: u64, name: &str) -> u64 {
    assert_ne!(h, 0, "{name}: expected a non-zero handle");
    h
}

/// The smoke contract for failure returns: failure sentinel + `err` filled
/// with a non-OK code and a non-empty, freeable message.
fn expect_clean_failure(code: i32, message: &str, name: &str) {
    assert_ne!(code, AIMUX_OK, "{name}: expected a non-OK error code");
    assert!(
        !message.is_empty(),
        "{name}: expected a non-empty error message"
    );
}

fn expect_ptr_failure(out: *mut c_char, err: &CAimuxError, name: &str) {
    assert!(out.is_null(), "{name}: expected NULL on failure");
    expect_clean_failure(err.code, &err_message(err), name);
}

fn expect_i32_failure(rc: i32, err: &CAimuxError, name: &str) {
    assert_eq!(rc, 0, "{name}: expected 0 on failure");
    expect_clean_failure(err.code, &err_message(err), name);
}

fn expect_u64_failure(h: u64, err: &CAimuxError, name: &str) {
    assert_eq!(h, 0, "{name}: expected handle 0 on failure");
    expect_clean_failure(err.code, &err_message(err), name);
}

/// For constructors whose inputs cannot be fully controlled (env-var key):
/// either a live handle or a clean failure is acceptable — a panic is not.
fn expect_handle_or_clean_failure(h: u64, err: &CAimuxError, name: &str) {
    if h != 0 {
        return;
    }
    expect_clean_failure(err.code, &err_message(err), name);
}

/// Read a returned JSON string, assert it is non-empty, and free it.
fn read_and_free_json(out: *mut c_char, name: &str) -> String {
    assert!(!out.is_null(), "{name}: expected a non-NULL JSON string");
    // SAFETY: `out` is an owned NUL-terminated C string returned by the FFI.
    let s = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    assert!(!s.is_empty(), "{name}: expected a non-empty JSON payload");
    // SAFETY: `out` was allocated by the FFI layer for the caller to free.
    unsafe { aimux_free_string(out) };
    s
}

/// An OpenAI model handle bound to the unreachable base URL (session-class
/// smoke input; `max_retries:0` is passed per-call via opts).
fn unreachable_model(err: *mut CAimuxError) -> u64 {
    aimux_openai_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("gpt-4o-mini").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        err,
    )
}

// ── no-op stream callbacks (must never re-enter the FFI layer) ──────────────

static PARTS_SEEN: AtomicUsize = AtomicUsize::new(0);
static DONE_SEEN: AtomicUsize = AtomicUsize::new(0);

extern "C" fn smoke_on_part(_json: *const c_char, _ctx: *mut c_void) {
    PARTS_SEEN.fetch_add(1, Ordering::Relaxed);
}

extern "C" fn smoke_on_done(_ctx: *mut c_void) {
    DONE_SEEN.fetch_add(1, Ordering::Relaxed);
}

// ── constructor class (49 exports) ──────────────────────────────────────────

#[test]
fn constructor_exports_build_and_release_handles() {
    // Collect every created handle so the release path is exercised too.
    let mut handles: Vec<u64> = Vec::new();
    let key = c(FAKE_KEY);
    let secret = c("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
    let region = c("us-east-1");
    let token = c("ya29.ffi-smoke-token");
    let project = c("ffi-smoke-project");
    let location = c("us-central1");
    let resource = c("ffi-smoke-resource");
    let base = c(UNREACHABLE);
    let mut err = fresh_err();

    // Simple key constructors + with_base variants.
    handles.push(expect_handle(
        aimux_openai_new(key.as_ptr(), c("gpt-4o-mini").as_ptr(), &mut err),
        "openai_new",
    ));
    handles.push(expect_handle(
        aimux_openai_new_with_base(
            key.as_ptr(),
            c("gpt-4o-mini").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "openai_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_anthropic_new(
            key.as_ptr(),
            c("claude-3-5-sonnet-latest").as_ptr(),
            &mut err,
        ),
        "anthropic_new",
    ));
    handles.push(expect_handle(
        aimux_anthropic_new_with_base(
            key.as_ptr(),
            c("claude-3-5-sonnet-latest").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "anthropic_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_cohere_new(key.as_ptr(), c("command-r-plus").as_ptr(), &mut err),
        "cohere_new",
    ));
    handles.push(expect_handle(
        aimux_cohere_new_with_base(
            key.as_ptr(),
            c("command-r-plus").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "cohere_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_mistral_new(key.as_ptr(), c("mistral-small-latest").as_ptr(), &mut err),
        "mistral_new",
    ));
    handles.push(expect_handle(
        aimux_mistral_new_with_base(
            key.as_ptr(),
            c("mistral-small-latest").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "mistral_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_xai_new(key.as_ptr(), c("grok-3").as_ptr(), &mut err),
        "xai_new",
    ));
    handles.push(expect_handle(
        aimux_xai_new_with_base(key.as_ptr(), c("grok-3").as_ptr(), base.as_ptr(), &mut err),
        "xai_new_with_base",
    ));

    // Credential constructors.
    handles.push(expect_handle(
        aimux_anthropic_aws_new(
            key.as_ptr(),
            region.as_ptr(),
            c("anthropic.claude-3-5-sonnet-20240620-v1:0").as_ptr(),
            &mut err,
        ),
        "anthropic_aws_new",
    ));
    handles.push(expect_handle(
        aimux_anthropic_aws_new_with_base(
            key.as_ptr(),
            region.as_ptr(),
            c("anthropic.claude-3-5-sonnet-20240620-v1:0").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "anthropic_aws_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_azure_new(
            key.as_ptr(),
            resource.as_ptr(),
            c("gpt-4o").as_ptr(),
            ptr::null(),
            &mut err,
        ),
        "azure_new",
    ));
    handles.push(expect_handle(
        aimux_azure_new_with_base(
            key.as_ptr(),
            base.as_ptr(),
            c("gpt-4o").as_ptr(),
            ptr::null(),
            &mut err,
        ),
        "azure_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_bedrock_new(
            key.as_ptr(),
            secret.as_ptr(),
            region.as_ptr(),
            c("anthropic.claude-3-5-sonnet-20240620-v1:0").as_ptr(),
            &mut err,
        ),
        "bedrock_new",
    ));
    handles.push(expect_handle(
        aimux_bedrock_new_with_base(
            key.as_ptr(),
            secret.as_ptr(),
            region.as_ptr(),
            c("anthropic.claude-3-5-sonnet-20240620-v1:0").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "bedrock_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_vertex_new(
            token.as_ptr(),
            project.as_ptr(),
            location.as_ptr(),
            c("gemini-2.0-flash").as_ptr(),
            &mut err,
        ),
        "vertex_new",
    ));
    handles.push(expect_handle(
        aimux_vertex_new_with_base(
            token.as_ptr(),
            project.as_ptr(),
            location.as_ptr(),
            c("gemini-2.0-flash").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "vertex_new_with_base",
    ));

    // Multimodal constructors.
    handles.push(expect_handle(
        aimux_openai_embedding_new(key.as_ptr(), c("text-embedding-3-small").as_ptr(), &mut err),
        "openai_embedding_new",
    ));
    handles.push(expect_handle(
        aimux_openai_embedding_new_with_base(
            key.as_ptr(),
            c("text-embedding-3-small").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "openai_embedding_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_cohere_embedding_new(key.as_ptr(), c("embed-english-v3.0").as_ptr(), &mut err),
        "cohere_embedding_new",
    ));
    handles.push(expect_handle(
        aimux_cohere_embedding_new_with_base(
            key.as_ptr(),
            c("embed-english-v3.0").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "cohere_embedding_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_google_embedding_new(key.as_ptr(), c("text-embedding-004").as_ptr(), &mut err),
        "google_embedding_new",
    ));
    handles.push(expect_handle(
        aimux_google_embedding_new_with_base(
            key.as_ptr(),
            c("text-embedding-004").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "google_embedding_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_openai_speech_new(key.as_ptr(), c("tts-1").as_ptr(), &mut err),
        "openai_speech_new",
    ));
    handles.push(expect_handle(
        aimux_openai_speech_new_with_base(
            key.as_ptr(),
            c("tts-1").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "openai_speech_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_openai_image_new(key.as_ptr(), c("gpt-image-1").as_ptr(), &mut err),
        "openai_image_new",
    ));
    handles.push(expect_handle(
        aimux_openai_image_new_with_base(
            key.as_ptr(),
            c("gpt-image-1").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "openai_image_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_google_image_new(
            key.as_ptr(),
            c("imagen-4.0-generate-001").as_ptr(),
            &mut err,
        ),
        "google_image_new",
    ));
    handles.push(expect_handle(
        aimux_google_image_new_with_base(
            key.as_ptr(),
            c("imagen-4.0-generate-001").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "google_image_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_openai_transcription_new(key.as_ptr(), c("whisper-1").as_ptr(), &mut err),
        "openai_transcription_new",
    ));
    handles.push(expect_handle(
        aimux_openai_transcription_new_with_base(
            key.as_ptr(),
            c("whisper-1").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "openai_transcription_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_openai_files_new(key.as_ptr(), &mut err),
        "openai_files_new",
    ));
    handles.push(expect_handle(
        aimux_openai_files_new_with_base(key.as_ptr(), base.as_ptr(), &mut err),
        "openai_files_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_cohere_reranking_new(key.as_ptr(), c("rerank-v3.5").as_ptr(), &mut err),
        "cohere_reranking_new",
    ));
    handles.push(expect_handle(
        aimux_cohere_reranking_new_with_base(
            key.as_ptr(),
            c("rerank-v3.5").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "cohere_reranking_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_google_video_new(key.as_ptr(), c("veo-3.0-generate-001").as_ptr(), &mut err),
        "google_video_new",
    ));
    handles.push(expect_handle(
        aimux_google_video_new_with_base(
            key.as_ptr(),
            c("veo-3.0-generate-001").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "google_video_new_with_base",
    ));
    handles.push(expect_handle(
        aimux_tavily_search_new(key.as_ptr(), c("tavily").as_ptr(), &mut err),
        "tavily_search_new",
    ));
    handles.push(expect_handle(
        aimux_tavily_search_new_with_base(
            key.as_ptr(),
            c("tavily").as_ptr(),
            base.as_ptr(),
            &mut err,
        ),
        "tavily_search_new_with_base",
    ));

    // Registry-backed constructors (RFC-0017 / RFC-0027).
    let provider_opts = c(r#"{"base_url":"http://127.0.0.1:1","max_retries":0}"#);
    handles.push(expect_handle(
        aimux_provider_new(
            c("groq").as_ptr(),
            key.as_ptr(),
            c("llama-3.3-70b-versatile").as_ptr(),
            provider_opts.as_ptr(),
            &mut err,
        ),
        "provider_new",
    ));
    handles.push(expect_handle(
        aimux_provider_handle_new(
            c("groq").as_ptr(),
            key.as_ptr(),
            provider_opts.as_ptr(),
            &mut err,
        ),
        "provider_handle_new",
    ));

    // Composite wrappers over an unreachable model handle.
    let model = unreachable_model(&mut err);
    handles.push(expect_handle(
        model,
        "openai_new_with_base (composite base)",
    ));
    let traced = aimux_trace_new(model, &mut err);
    handles.push(expect_handle(traced, "trace_new"));
    let audited = aimux_trace_new_audited(model, 0, &mut err);
    handles.push(expect_handle(audited, "trace_new_audited"));
    let children = [traced, audited];
    let router = aimux_router_new(children.as_ptr(), children.len(), ptr::null(), &mut err);
    handles.push(expect_handle(router, "router_new"));
    let moa = aimux_moa_new(
        children.as_ptr(),
        children.len(),
        model,
        ptr::null(),
        &mut err,
    );
    handles.push(expect_handle(moa, "moa_new"));
    handles.push(expect_handle(aimux_abort_signal_new(), "abort_signal_new"));

    // Release every handle created above (aimux_drop_handle is safe with 0).
    for h in [traced, audited, router, moa, model] {
        aimux_drop_handle(h);
    }

    // mock_replay_new: empty input must fail cleanly (no recordings) — the
    // success path needs a real Recording, covered by replay tests elsewhere.
    let mut replay_err = fresh_err();
    let h = aimux_mock_replay_new(c("").as_ptr(), &mut replay_err);
    expect_u64_failure(h, &replay_err, "mock_replay_new (empty jsonl)");
    release_err(&mut replay_err);

    // None of the successful constructors above filled the shared error slot.
    assert_eq!(
        err.code, AIMUX_OK,
        "successful constructors must not fill err"
    );
    for h in handles {
        aimux_drop_handle(h);
    }
}

/// `provider_from_env` reads the provider's env var, which the harness cannot
/// control: a live handle or a clean missing-key error are both correct.
#[test]
fn provider_from_env_handle_or_clean_failure() {
    let mut err = fresh_err();
    let h = aimux_provider_from_env(
        c("groq").as_ptr(),
        c("llama-3.3-70b-versatile").as_ptr(),
        &mut err,
    );
    expect_handle_or_clean_failure(h, &err, "provider_from_env");
    if h != 0 {
        aimux_drop_handle(h);
    }
    release_err(&mut err);
}

// ── session class: text generation (4 exports) ──────────────────────────────

#[test]
fn text_generation_exports_fail_cleanly_on_unreachable_host() {
    // Collect every created handle so the release path is exercised too.
    let mut handles: Vec<u64> = Vec::new();
    let mut err = fresh_err();
    let h = unreachable_model(&mut err);
    handles.push(expect_handle(h, "model handle"));

    let prompt = c("\"ffi smoke\"");
    let opts = c(r#"{"max_retries":0}"#);

    let out = aimux_generate_text(h, prompt.as_ptr(), opts.as_ptr(), &mut err);
    expect_ptr_failure(out, &err, "generate_text");
    release_err(&mut err);

    let out = aimux_generate_object(h, prompt.as_ptr(), opts.as_ptr(), &mut err);
    expect_ptr_failure(out, &err, "generate_object");
    release_err(&mut err);

    let out = aimux_consume_stream_text(h, prompt.as_ptr(), opts.as_ptr(), &mut err);
    expect_ptr_failure(out, &err, "consume_stream_text");
    release_err(&mut err);

    let out = aimux_generate_text_as_openai(h, prompt.as_ptr(), opts.as_ptr(), &mut err);
    expect_ptr_failure(out, &err, "generate_text_as_openai");
    release_err(&mut err);

    aimux_drop_handle(h);
    for h in handles {
        aimux_drop_handle(h);
    }
}

// ── session class: streaming with callbacks (4 exports) ─────────────────────

#[test]
fn streaming_exports_fail_cleanly_on_unreachable_host() {
    // Collect every created handle so the release path is exercised too.
    let mut handles: Vec<u64> = Vec::new();
    let mut err = fresh_err();
    let h = unreachable_model(&mut err);
    handles.push(expect_handle(h, "model handle"));

    let prompt = c("\"ffi smoke\"");
    let opts = c(r#"{"max_retries":0}"#);

    let rc = aimux_stream_text(
        h,
        prompt.as_ptr(),
        opts.as_ptr(),
        smoke_on_part,
        smoke_on_done,
        ptr::null_mut(),
        &mut err,
    );
    expect_i32_failure(rc, &err, "stream_text");
    release_err(&mut err);

    let abort = aimux_abort_signal_new();
    let rc = aimux_stream_text_with_abort(
        h,
        abort,
        prompt.as_ptr(),
        opts.as_ptr(),
        smoke_on_part,
        smoke_on_done,
        ptr::null_mut(),
        &mut err,
    );
    expect_i32_failure(rc, &err, "stream_text_with_abort");
    release_err(&mut err);

    let rc = aimux_stream_text_as_openai(
        h,
        prompt.as_ptr(),
        opts.as_ptr(),
        smoke_on_part,
        smoke_on_done,
        ptr::null_mut(),
        &mut err,
    );
    expect_i32_failure(rc, &err, "stream_text_as_openai");
    release_err(&mut err);

    let rc = aimux_stream_text_as_openai_with_abort(
        h,
        abort,
        prompt.as_ptr(),
        opts.as_ptr(),
        smoke_on_part,
        smoke_on_done,
        ptr::null_mut(),
        &mut err,
    );
    expect_i32_failure(rc, &err, "stream_text_as_openai_with_abort");
    release_err(&mut err);

    aimux_abort_signal_drop(abort);
    aimux_drop_handle(h);
    for h in handles {
        aimux_drop_handle(h);
    }
}

// ── session class: multimodal generation (8 exports, one per test to
// parallelise the default retry backoff — their option structs expose no
// max_retries override) ───────────────────────────────────────────────────────

#[test]
fn embed_fails_cleanly_on_unreachable_host() {
    // Collect every created handle so the release path is exercised too.
    let mut handles: Vec<u64> = Vec::new();
    let mut err = fresh_err();
    let h = aimux_openai_embedding_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("text-embedding-3-small").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut err,
    );
    handles.push(expect_handle(h, "embedding handle"));
    let out = aimux_embed(
        h,
        c(r#"["hello"]"#).as_ptr(),
        c(r#"{"values":[]}"#).as_ptr(),
        &mut err,
    );
    expect_ptr_failure(out, &err, "embed");
    release_err(&mut err);
    aimux_drop_handle(h);
    for h in handles {
        aimux_drop_handle(h);
    }
}

#[test]
fn speech_generate_fails_cleanly_on_unreachable_host() {
    // Collect every created handle so the release path is exercised too.
    let mut handles: Vec<u64> = Vec::new();
    let mut err = fresh_err();
    let h = aimux_openai_speech_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("tts-1").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut err,
    );
    handles.push(expect_handle(h, "speech handle"));
    let out = aimux_speech_generate(h, c(r#"{"text":"ffi smoke"}"#).as_ptr(), &mut err);
    expect_ptr_failure(out, &err, "speech_generate");
    release_err(&mut err);
    aimux_drop_handle(h);
    for h in handles {
        aimux_drop_handle(h);
    }
}

#[test]
fn image_generate_fails_cleanly_on_unreachable_host() {
    // Collect every created handle so the release path is exercised too.
    let mut handles: Vec<u64> = Vec::new();
    let mut err = fresh_err();
    let h = aimux_openai_image_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("gpt-image-1").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut err,
    );
    handles.push(expect_handle(h, "image handle"));
    let opts = c(r#"{"prompt":"a rust crab","n":1,"provider_options":{}}"#);
    let out = aimux_image_generate(h, opts.as_ptr(), &mut err);
    expect_ptr_failure(out, &err, "image_generate");
    release_err(&mut err);
    aimux_drop_handle(h);
    for h in handles {
        aimux_drop_handle(h);
    }
}

#[test]
fn transcription_generate_fails_cleanly_on_unreachable_host() {
    // Collect every created handle so the release path is exercised too.
    let mut handles: Vec<u64> = Vec::new();
    let mut err = fresh_err();
    let h = aimux_openai_transcription_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("whisper-1").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut err,
    );
    handles.push(expect_handle(h, "transcription handle"));
    let out = aimux_transcription_generate(
        h,
        c("ZmFrZS1hdWRpbw==").as_ptr(),
        c("audio/wav").as_ptr(),
        ptr::null(),
        &mut err,
    );
    expect_ptr_failure(out, &err, "transcription_generate");
    release_err(&mut err);
    aimux_drop_handle(h);
    for h in handles {
        aimux_drop_handle(h);
    }
}

#[test]
fn file_upload_fails_cleanly_on_unreachable_host() {
    // Collect every created handle so the release path is exercised too.
    let mut handles: Vec<u64> = Vec::new();
    let mut err = fresh_err();
    let h =
        aimux_openai_files_new_with_base(c(FAKE_KEY).as_ptr(), c(UNREACHABLE).as_ptr(), &mut err);
    handles.push(expect_handle(h, "files handle"));
    let out = aimux_file_upload(
        h,
        c("ZmFrZS1maWxl").as_ptr(),
        c("text/plain").as_ptr(),
        ptr::null(),
        &mut err,
    );
    expect_ptr_failure(out, &err, "file_upload");
    release_err(&mut err);
    aimux_drop_handle(h);
    for h in handles {
        aimux_drop_handle(h);
    }
}

#[test]
fn rerank_fails_cleanly_on_unreachable_host() {
    // Collect every created handle so the release path is exercised too.
    let mut handles: Vec<u64> = Vec::new();
    let mut err = fresh_err();
    let h = aimux_cohere_reranking_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("rerank-v3.5").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut err,
    );
    handles.push(expect_handle(h, "reranking handle"));
    let opts = c(r#"{"query":"rust","documents":{"Text":{"values":["a","b"]}}}"#);
    let out = aimux_rerank(h, opts.as_ptr(), &mut err);
    expect_ptr_failure(out, &err, "rerank");
    release_err(&mut err);
    aimux_drop_handle(h);
    for h in handles {
        aimux_drop_handle(h);
    }
}

#[test]
fn video_generate_fails_cleanly_on_unreachable_host() {
    // Collect every created handle so the release path is exercised too.
    let mut handles: Vec<u64> = Vec::new();
    let mut err = fresh_err();
    let h = aimux_google_video_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("veo-3.0-generate-001").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut err,
    );
    handles.push(expect_handle(h, "video handle"));
    let opts = c(r#"{"prompt":"waves","n":1,"provider_options":{}}"#);
    let out = aimux_video_generate(h, opts.as_ptr(), &mut err);
    expect_ptr_failure(out, &err, "video_generate");
    release_err(&mut err);
    aimux_drop_handle(h);
    for h in handles {
        aimux_drop_handle(h);
    }
}

#[test]
fn search_fails_cleanly_on_unreachable_host() {
    // Collect every created handle so the release path is exercised too.
    let mut handles: Vec<u64> = Vec::new();
    let mut err = fresh_err();
    let h = aimux_tavily_search_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("tavily").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut err,
    );
    handles.push(expect_handle(h, "search handle"));
    let out = aimux_search(h, c(r#"{"query":"ffi smoke"}"#).as_ptr(), &mut err);
    expect_ptr_failure(out, &err, "search");
    release_err(&mut err);
    aimux_drop_handle(h);
    for h in handles {
        aimux_drop_handle(h);
    }
}

// ── session class: transcription streaming session (5 exports) ──────────────

#[test]
fn transcription_session_reaches_clean_terminal_error() {
    // Collect every created handle so the release path is exercised too.
    let mut handles: Vec<u64> = Vec::new();
    let mut err = fresh_err();
    let model = aimux_openai_transcription_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("whisper-1").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut err,
    );
    handles.push(expect_handle(model, "transcription handle"));

    let session = aimux_transcription_session_new(model, 0, ptr::null(), &mut err);
    handles.push(expect_handle(session, "transcription_session_new"));

    // The driver fails on the unreachable host; pull parts until the stream
    // reaches its terminal NULL (error or end) within a bounded number of
    // timed pulls. This is exactly the smoke point: a clean error path.
    let mut reached_terminal = false;
    for _ in 0..3 {
        let mut err = fresh_err();
        let part = aimux_transcription_next_part(session, 3_000, &mut err);
        if part.is_null() {
            release_err(&mut err);
            reached_terminal = true;
            break;
        }
        read_and_free_json(part, "transcription part");
    }
    assert!(
        reached_terminal,
        "session must reach a terminal NULL within a few pulls"
    );

    // After the terminal state a push is either rejected (0 + err) or still
    // buffered (1) — both clean, non-panicking outcomes.
    let mut err = fresh_err();
    let chunk = [0u8, 1, 2, 3];
    let _ = aimux_transcription_push_audio(session, chunk.as_ptr(), chunk.len(), &mut err);
    release_err(&mut err);

    assert_eq!(
        aimux_transcription_input_done(session, ptr::null_mut()),
        1,
        "input_done on a live session handle must succeed"
    );

    // Drop, then verify the handle is gone (clean failure, no panic).
    aimux_transcription_session_drop(session);
    let mut err = fresh_err();
    assert_eq!(
        aimux_transcription_push_audio(session, chunk.as_ptr(), chunk.len(), &mut err),
        0,
        "push after drop must fail"
    );
    release_err(&mut err);
    aimux_drop_handle(model);
    for h in handles {
        aimux_drop_handle(h);
    }
}

// ── session class: provider discovery + catalogue + codex (4 exports) ───────

#[test]
fn provider_discovery_and_catalogue_exports_fail_cleanly() {
    // Collect every created handle so the release path is exercised too.
    let mut handles: Vec<u64> = Vec::new();
    let mut err = fresh_err();
    // Provider handle bound to the unreachable host with retries disabled.
    let provider = aimux_provider_handle_new(
        c("groq").as_ptr(),
        c(FAKE_KEY).as_ptr(),
        c(r#"{"base_url":"http://127.0.0.1:1","max_retries":0}"#).as_ptr(),
        &mut err,
    );
    handles.push(expect_handle(provider, "provider_handle_new"));

    let models = aimux_provider_list_models(provider, &mut err);
    expect_ptr_failure(models, &err, "provider_list_models");
    release_err(&mut err);

    let model = aimux_provider_model(provider, c("llama-3.3-70b-versatile").as_ptr(), &mut err);
    handles.push(expect_handle(model, "provider_model"));
    aimux_drop_handle(model);
    aimux_drop_handle(provider);

    // Catalogue fetch pointed at the unreachable host.
    let specs = aimux_get_model_specs(c(UNREACHABLE).as_ptr(), &mut err);
    expect_ptr_failure(specs, &err, "get_model_specs");
    release_err(&mut err);

    // Codex OAuth refresh: NULL args fail cleanly at the boundary without
    // touching the network.
    let out = aimux_codex_refresh(ptr::null(), ptr::null(), &mut err);
    expect_ptr_failure(out, &err, "codex_refresh (null args)");
    release_err(&mut err);
    for h in handles {
        aimux_drop_handle(h);
    }
}

// ── utility class: lifecycle, sessions, config (10 exports) ─────────────────

#[test]
fn utility_exports_return_clean_values() {
    assert_eq!(aimux_init_logging(c("warn").as_ptr()), 0, "init_logging");
    assert_eq!(aimux_session_store_init(), 0, "session_store_init");
    assert_eq!(aimux_session_infer_init(0), 0, "session_infer_init");

    let mut err = fresh_err();
    let calls = aimux_session_calls(c("ffi-smoke-session").as_ptr(), &mut err);
    let calls = read_and_free_json(calls, "session_calls");
    assert!(calls.starts_with('['), "session_calls returns a JSON array");

    let sessions = aimux_list_sessions(&mut err);
    let sessions = read_and_free_json(sessions, "list_sessions");
    assert!(
        sessions.starts_with('['),
        "list_sessions returns a JSON array"
    );

    // Abort-signal lifecycle.
    let abort = aimux_abort_signal_new();
    aimux_abort_signal_abort(abort); // idempotent no-op on valid handle
    aimux_abort_signal_abort(abort);
    aimux_abort_signal_drop(abort);
    aimux_abort_signal_drop(0); // safe with 0

    // aimux_drop_handle(0) is documented as a safe no-op.
    aimux_drop_handle(0);

    // register_providers: valid overlay then a malformed config.
    let valid = c(
        r#"{"providers":[{"name":"ffi-smoke-provider","base_url":"http://127.0.0.1:1/v1","protocol":"openai_compat"}]}"#,
    );
    assert_eq!(
        aimux_register_providers(valid.as_ptr(), ptr::null_mut()),
        1,
        "register_providers (valid)"
    );
    let mut err = fresh_err();
    assert_eq!(
        aimux_register_providers(c("{not json").as_ptr(), &mut err),
        0,
        "register_providers (malformed)"
    );
    assert_eq!(err.code, AIMUX_E_JSON_PARSE);
    release_err(&mut err);

    // Empty proxy config is the documented no-op-style success.
    assert_eq!(
        aimux_init_proxy(c("{}").as_ptr(), ptr::null_mut()),
        1,
        "init_proxy"
    );
}

// ── utility class: trace queries (5 exports) ────────────────────────────────

#[test]
fn trace_query_exports_return_clean_values() {
    // Collect every created handle so the release path is exercised too.
    let mut handles: Vec<u64> = Vec::new();
    let mut err = fresh_err();
    let model = unreachable_model(&mut err);
    handles.push(expect_handle(model, "model handle"));
    let traced = aimux_trace_new(model, &mut err);
    handles.push(expect_handle(traced, "trace handle"));

    // Empty filter `{}` = all (TraceFilter is all-Option).
    let stats = aimux_trace_aggregate(traced, c("{}").as_ptr(), &mut err);
    let stats = read_and_free_json(stats, "trace_aggregate");
    assert!(
        stats.starts_with('['),
        "trace_aggregate returns a JSON array"
    );

    // Unknown session: documented clean failure.
    let chain = aimux_trace_session_chain(traced, c("ffi-smoke-unknown").as_ptr(), &mut err);
    expect_ptr_failure(chain, &err, "trace_session_chain (unknown session)");
    release_err(&mut err);

    let trajectory =
        aimux_trace_session_trajectory(traced, c("ffi-smoke-unknown").as_ptr(), &mut err);
    let trajectory = read_and_free_json(trajectory, "trace_session_trajectory");
    assert!(
        trajectory.starts_with('['),
        "trace_session_trajectory returns a JSON array"
    );

    let jsonl_ptr = aimux_trace_export_jsonl(traced, &mut err);
    assert!(
        !jsonl_ptr.is_null(),
        "trace_export_jsonl must return a string"
    );
    // SAFETY: `jsonl_ptr` is an owned NUL-terminated C string returned by the
    // FFI layer; read it, then hand the original pointer back for freeing.
    let jsonl = unsafe { CStr::from_ptr(jsonl_ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { aimux_free_string(jsonl_ptr) };
    assert!(
        jsonl.is_empty() || jsonl.lines().all(|l| !l.trim().is_empty()),
        "trace_export_jsonl is empty-or-JSONL"
    );

    assert_eq!(aimux_trace_clear(traced), 0, "trace_clear on live handle");
    assert_eq!(
        aimux_trace_clear(999_999),
        -1,
        "trace_clear on unknown handle"
    );

    aimux_drop_handle(traced);
    aimux_drop_handle(model);
    for h in handles {
        aimux_drop_handle(h);
    }
}

// ── utility class: recording (6 exports) ────────────────────────────────────

#[test]
fn recording_exports_lifecycle_cleanly() {
    // cap == 0 is the documented failure.
    assert_eq!(aimux_init_recording_ring(0), -1, "init_recording_ring(0)");

    assert_eq!(aimux_init_recording_ring(8), 0, "init_recording_ring");
    assert_eq!(aimux_recording_flush(), 0, "recording_flush");
    assert_eq!(aimux_recording_try_flush(), AIMUX_OK, "recording_try_flush");

    // File-backed recorder in a throwaway temp dir.
    let dir = std::env::temp_dir().join(format!("aimux-ffi-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    assert_eq!(
        aimux_init_recording(c(dir.to_str().expect("utf8 temp dir")).as_ptr()),
        0,
        "init_recording"
    );

    assert_eq!(
        aimux_init_recording_ring_default(),
        0,
        "init_recording_ring_default"
    );
    assert_eq!(aimux_recording_stop(), 0, "recording_stop");
    assert_eq!(aimux_recording_flush(), 0, "recording_flush after stop");
    assert_eq!(
        aimux_recording_try_flush(),
        AIMUX_OK,
        "recording_try_flush after stop"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
