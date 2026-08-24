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
//!   (`http://127.0.0.1:1`) → must return NULL and write a non-zero handle,
//!   which is then released with [`aimux_ffi::aimux_drop_handle`].
//!   Constructors that cannot take a base URL (e.g. `aimux_provider_from_env`)
//!   are only checked for a clean handle-or-error outcome.
//! - **Utility class**: called directly with minimal/empty arguments; asserts
//!   a sane outcome, never a panic.
//! - **Session class** (generate / stream / embed / speech / image /
//!   transcription / rerank / video / search / files): handle + minimal
//!   options against the unreachable base URL → must return a returned error
//!   with the AiMuxError view and leave the out-param NULL (a clean error path).
//!   Text calls pass `{"max_retries":0}` to skip the retry backoff; the
//!   multimodal option structs expose no retry override, so those tests run
//!   as separate `#[test]`s to parallelise the default backoff.
//! - Every returned error and every returned JSON string is released
//!   (`aimux_error_free` / `aimux_free_string`).
//!
//! Note: multimodal session smoke calls use the default RetryConfig
//! (2 retries) against a refused port; under `--test-threads=1` this adds
//! roughly a minute of backoff sleep — run the default parallel profile.
//!
//! ## Coverage
//!
//! All 114 `#[unsafe(no_mangle)]` exports in `src/lib.rs` are exercised (the
//! constructor and utility classes in full; the session class one
//! representative call per export). [`header_and_exports_agree`] pins the
//! count against the two headers.

mod common;

use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimux_ffi::{
    AIMUX_E_FFI_INVALID_WIRE_JSON, AIMUX_E_INVALID_ARGUMENT, AIMUX_OK, aimux_abort_signal_abort,
    aimux_abort_signal_drop, aimux_abort_signal_new, aimux_anthropic_aws_new,
    aimux_anthropic_aws_new_with_base, aimux_anthropic_new, aimux_anthropic_new_with_base,
    aimux_azure_new, aimux_azure_new_with_base, aimux_bedrock_new, aimux_bedrock_new_with_base,
    aimux_codex_refresh, aimux_cohere_embedding_new, aimux_cohere_embedding_new_with_base,
    aimux_cohere_new, aimux_cohere_new_with_base, aimux_cohere_reranking_new,
    aimux_cohere_reranking_new_with_base, aimux_consume_stream_text, aimux_drop_handle,
    aimux_embed, aimux_error_available_tools, aimux_error_code, aimux_error_free,
    aimux_error_message, aimux_error_model_id, aimux_error_model_type, aimux_error_original_error,
    aimux_error_provider_code, aimux_error_provider_id, aimux_error_provider_message,
    aimux_error_request_id, aimux_error_response_body, aimux_error_retry_ms, aimux_error_retryable,
    aimux_error_status, aimux_error_t, aimux_error_tool_input, aimux_error_tool_name,
    aimux_file_upload, aimux_free_string, aimux_generate_object, aimux_generate_text,
    aimux_generate_text_as_openai, aimux_get_model_specs, aimux_google_embedding_new,
    aimux_google_embedding_new_with_base, aimux_google_image_new, aimux_google_image_new_with_base,
    aimux_google_video_new, aimux_google_video_new_with_base, aimux_image_generate,
    aimux_init_logging, aimux_init_proxy, aimux_init_recording, aimux_init_recording_ring,
    aimux_init_recording_ring_default, aimux_list_sessions, aimux_mistral_new,
    aimux_mistral_new_with_base, aimux_moa_new, aimux_mock_replay_new, aimux_openai_embedding_new,
    aimux_openai_embedding_new_with_base, aimux_openai_files_new, aimux_openai_files_new_with_base,
    aimux_openai_image_new, aimux_openai_image_new_with_base, aimux_openai_new,
    aimux_openai_new_with_base, aimux_openai_speech_new, aimux_openai_speech_new_with_base,
    aimux_openai_transcription_new, aimux_openai_transcription_new_with_base,
    aimux_provider_from_env, aimux_provider_handle_new, aimux_provider_list_models,
    aimux_provider_model, aimux_provider_new, aimux_recording_flush, aimux_recording_stop,
    aimux_recording_try_flush, aimux_register_providers, aimux_rerank, aimux_router_new,
    aimux_search, aimux_session_calls, aimux_session_infer_init, aimux_session_store_init,
    aimux_speech_generate, aimux_stream_text, aimux_stream_text_as_openai,
    aimux_stream_text_as_openai_with_abort, aimux_stream_text_with_abort, aimux_tavily_search_new,
    aimux_tavily_search_new_with_base, aimux_trace_aggregate, aimux_trace_clear,
    aimux_trace_export_jsonl, aimux_trace_new, aimux_trace_new_audited, aimux_trace_session_chain,
    aimux_trace_session_trajectory, aimux_transcription_generate, aimux_transcription_input_done,
    aimux_transcription_next_part, aimux_transcription_push_audio,
    aimux_transcription_session_drop, aimux_transcription_session_new, aimux_vertex_new,
    aimux_vertex_new_with_base, aimux_video_generate, aimux_xai_new, aimux_xai_new_with_base,
};
use common::{c, expect_aimux_error, expect_failure, expect_ffi_error, msg, ok, take};

/// Loopback port that nothing listens on: connections are refused (fast,
/// deterministic) on every platform. Through an HTTP proxy it surfaces as a
/// proxy error — either way a clean transport error, never a hang.
const UNREACHABLE: &str = "http://127.0.0.1:1";

const FAKE_KEY: &str = "sk-ffi-smoke-fake-key";

// ── harness helpers ─────────────────────────────────────────────────────────

/// Success + non-zero handle, or panic with the message.
fn expect_handle(e: *mut aimux_error_t, h: u64, name: &str) -> u64 {
    ok(e, name);
    assert_ne!(h, 0, "{name}: expected a non-zero handle");
    h
}

/// A string-result call failed cleanly: NULL out-param and AiMuxError code.
fn expect_ptr_aimux_failure(e: *mut aimux_error_t, out: *mut c_char, name: &str) {
    assert!(out.is_null(), "{name}: expected NULL out-param on failure");
    expect_aimux_error(e, name);
}

/// Read a returned JSON string, assert it is non-empty, and free it.
fn read_and_free_json(out: *mut c_char, name: &str) -> String {
    assert!(!out.is_null(), "{name}: expected a non-NULL JSON string");
    let s = take(out);
    assert!(!s.is_empty(), "{name}: expected a non-empty JSON payload");
    s
}

/// An OpenAI model handle bound to the unreachable base URL (session-class
/// smoke input; `max_retries:0` is passed per-call via opts).
fn unreachable_model() -> u64 {
    let mut h = 0;
    let e = aimux_openai_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("gpt-4o-mini").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut h,
    );
    expect_handle(e, h, "openai_new_with_base (unreachable)")
}

// ── no-op stream callbacks (must never re-enter the FFI layer) ──────────────

static PARTS_SEEN: AtomicUsize = AtomicUsize::new(0);
static DONE_SEEN: AtomicUsize = AtomicUsize::new(0);

extern "C-unwind" fn smoke_on_part(_json: *const c_char, _ctx: *mut c_void) {
    PARTS_SEEN.fetch_add(1, Ordering::Relaxed);
}

extern "C-unwind" fn smoke_on_done(_ctx: *mut c_void) {
    DONE_SEEN.fetch_add(1, Ordering::Relaxed);
}

// ── header / export parity ──────────────────────────────────────────────────

/// Every `#[unsafe(no_mangle)]` export in `src/lib.rs` has exactly one
/// prototype across the two headers, and vice versa.
#[test]
fn header_and_exports_agree() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
    let mut exports: Vec<String> = lib
        .split("#[unsafe(no_mangle)]")
        .skip(1)
        .map(|after| {
            let after = &after[after.find("fn aimux_").expect("export fn") + 3..];
            after[..after.find('(').unwrap()].to_string()
        })
        .collect();
    exports.sort();
    assert_eq!(
        exports.len(),
        113,
        "export count changed; update the headers"
    );

    let headers = std::fs::read_to_string(root.join("aimux-ffi.h")).unwrap()
        + &std::fs::read_to_string(root.join("aimux-error.h")).unwrap();
    // A prototype: a line that starts with a return type and names an
    // `aimux_*(` symbol (typedefs and comments start with other tokens).
    let mut protos: Vec<String> = headers
        .lines()
        .filter(|l| {
            l.starts_with("aimux_error_t *aimux_")
                || l.starts_with("uint64_t aimux_")
                || l.starts_with("void aimux_")
                || l.starts_with("int32_t aimux_")
                || l.starts_with("int64_t aimux_")
                || l.starts_with("char *aimux_")
                || (l.starts_with("const aimux_") && l.contains("*aimux_"))
        })
        .map(|l| {
            let sig = &l[..l.find('(').unwrap()];
            sig.rsplit(' ')
                .next()
                .unwrap()
                .trim_start_matches('*')
                .to_string()
        })
        .collect();
    protos.sort();
    assert_eq!(protos, exports, "header prototypes vs. no_mangle exports");
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
    let mut h: u64;

    macro_rules! ctor {
        ($name:literal, $call:expr) => {{
            h = 0;
            let e = $call;
            handles.push(expect_handle(e, h, $name));
        }};
    }

    // Simple key constructors + with_base variants.
    ctor!(
        "openai_new",
        aimux_openai_new(key.as_ptr(), c("gpt-4o-mini").as_ptr(), &mut h)
    );
    ctor!(
        "openai_new_with_base",
        aimux_openai_new_with_base(
            key.as_ptr(),
            c("gpt-4o-mini").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "anthropic_new",
        aimux_anthropic_new(key.as_ptr(), c("claude-3-5-sonnet-latest").as_ptr(), &mut h)
    );
    ctor!(
        "anthropic_new_with_base",
        aimux_anthropic_new_with_base(
            key.as_ptr(),
            c("claude-3-5-sonnet-latest").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "cohere_new",
        aimux_cohere_new(key.as_ptr(), c("command-r-plus").as_ptr(), &mut h)
    );
    ctor!(
        "cohere_new_with_base",
        aimux_cohere_new_with_base(
            key.as_ptr(),
            c("command-r-plus").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "mistral_new",
        aimux_mistral_new(key.as_ptr(), c("mistral-small-latest").as_ptr(), &mut h)
    );
    ctor!(
        "mistral_new_with_base",
        aimux_mistral_new_with_base(
            key.as_ptr(),
            c("mistral-small-latest").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "xai_new",
        aimux_xai_new(key.as_ptr(), c("grok-3").as_ptr(), &mut h)
    );
    ctor!(
        "xai_new_with_base",
        aimux_xai_new_with_base(key.as_ptr(), c("grok-3").as_ptr(), base.as_ptr(), &mut h)
    );

    // Credential constructors.
    ctor!(
        "anthropic_aws_new",
        aimux_anthropic_aws_new(
            key.as_ptr(),
            region.as_ptr(),
            c("anthropic.claude-3-5-sonnet-20240620-v1:0").as_ptr(),
            &mut h
        )
    );
    ctor!(
        "anthropic_aws_new_with_base",
        aimux_anthropic_aws_new_with_base(
            key.as_ptr(),
            region.as_ptr(),
            c("anthropic.claude-3-5-sonnet-20240620-v1:0").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "azure_new",
        aimux_azure_new(
            key.as_ptr(),
            resource.as_ptr(),
            c("gpt-4o").as_ptr(),
            ptr::null(),
            &mut h
        )
    );
    ctor!(
        "azure_new_with_base",
        aimux_azure_new_with_base(
            key.as_ptr(),
            base.as_ptr(),
            c("gpt-4o").as_ptr(),
            ptr::null(),
            &mut h
        )
    );
    ctor!(
        "bedrock_new",
        aimux_bedrock_new(
            key.as_ptr(),
            secret.as_ptr(),
            region.as_ptr(),
            c("anthropic.claude-3-5-sonnet-20240620-v1:0").as_ptr(),
            &mut h
        )
    );
    ctor!(
        "bedrock_new_with_base",
        aimux_bedrock_new_with_base(
            key.as_ptr(),
            secret.as_ptr(),
            region.as_ptr(),
            c("anthropic.claude-3-5-sonnet-20240620-v1:0").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "vertex_new",
        aimux_vertex_new(
            token.as_ptr(),
            project.as_ptr(),
            location.as_ptr(),
            c("gemini-2.0-flash").as_ptr(),
            &mut h
        )
    );
    ctor!(
        "vertex_new_with_base",
        aimux_vertex_new_with_base(
            token.as_ptr(),
            project.as_ptr(),
            location.as_ptr(),
            c("gemini-2.0-flash").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );

    // Multimodal constructors.
    ctor!(
        "openai_embedding_new",
        aimux_openai_embedding_new(key.as_ptr(), c("text-embedding-3-small").as_ptr(), &mut h)
    );
    ctor!(
        "openai_embedding_new_with_base",
        aimux_openai_embedding_new_with_base(
            key.as_ptr(),
            c("text-embedding-3-small").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "cohere_embedding_new",
        aimux_cohere_embedding_new(key.as_ptr(), c("embed-english-v3.0").as_ptr(), &mut h)
    );
    ctor!(
        "cohere_embedding_new_with_base",
        aimux_cohere_embedding_new_with_base(
            key.as_ptr(),
            c("embed-english-v3.0").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "google_embedding_new",
        aimux_google_embedding_new(key.as_ptr(), c("text-embedding-004").as_ptr(), &mut h)
    );
    ctor!(
        "google_embedding_new_with_base",
        aimux_google_embedding_new_with_base(
            key.as_ptr(),
            c("text-embedding-004").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "openai_speech_new",
        aimux_openai_speech_new(key.as_ptr(), c("tts-1").as_ptr(), &mut h)
    );
    ctor!(
        "openai_speech_new_with_base",
        aimux_openai_speech_new_with_base(key.as_ptr(), c("tts-1").as_ptr(), base.as_ptr(), &mut h)
    );
    ctor!(
        "openai_image_new",
        aimux_openai_image_new(key.as_ptr(), c("gpt-image-1").as_ptr(), &mut h)
    );
    ctor!(
        "openai_image_new_with_base",
        aimux_openai_image_new_with_base(
            key.as_ptr(),
            c("gpt-image-1").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "google_image_new",
        aimux_google_image_new(key.as_ptr(), c("imagen-4.0-generate-001").as_ptr(), &mut h)
    );
    ctor!(
        "google_image_new_with_base",
        aimux_google_image_new_with_base(
            key.as_ptr(),
            c("imagen-4.0-generate-001").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "openai_transcription_new",
        aimux_openai_transcription_new(key.as_ptr(), c("whisper-1").as_ptr(), &mut h)
    );
    ctor!(
        "openai_transcription_new_with_base",
        aimux_openai_transcription_new_with_base(
            key.as_ptr(),
            c("whisper-1").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "openai_files_new",
        aimux_openai_files_new(key.as_ptr(), &mut h)
    );
    ctor!(
        "openai_files_new_with_base",
        aimux_openai_files_new_with_base(key.as_ptr(), base.as_ptr(), &mut h)
    );
    ctor!(
        "cohere_reranking_new",
        aimux_cohere_reranking_new(key.as_ptr(), c("rerank-v3.5").as_ptr(), &mut h)
    );
    ctor!(
        "cohere_reranking_new_with_base",
        aimux_cohere_reranking_new_with_base(
            key.as_ptr(),
            c("rerank-v3.5").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "google_video_new",
        aimux_google_video_new(key.as_ptr(), c("veo-3.0-generate-001").as_ptr(), &mut h)
    );
    ctor!(
        "google_video_new_with_base",
        aimux_google_video_new_with_base(
            key.as_ptr(),
            c("veo-3.0-generate-001").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "tavily_search_new",
        aimux_tavily_search_new(key.as_ptr(), c("tavily").as_ptr(), &mut h)
    );
    ctor!(
        "tavily_search_new_with_base",
        aimux_tavily_search_new_with_base(
            key.as_ptr(),
            c("tavily").as_ptr(),
            base.as_ptr(),
            &mut h
        )
    );

    // Registry-backed constructors (RFC-0017 / RFC-0027).
    let provider_opts = c(r#"{"base_url":"http://127.0.0.1:1","max_retries":0}"#);
    ctor!(
        "provider_new",
        aimux_provider_new(
            c("groq").as_ptr(),
            key.as_ptr(),
            c("llama-3.3-70b-versatile").as_ptr(),
            provider_opts.as_ptr(),
            &mut h
        )
    );
    ctor!(
        "provider_handle_new",
        aimux_provider_handle_new(
            c("groq").as_ptr(),
            key.as_ptr(),
            provider_opts.as_ptr(),
            &mut h
        )
    );

    // Composite wrappers over an unreachable model handle.
    let model = unreachable_model();
    handles.push(model);
    let mut traced = 0;
    expect_handle(aimux_trace_new(model, &mut traced), traced, "trace_new");
    let mut audited = 0;
    expect_handle(
        aimux_trace_new_audited(model, 0, &mut audited),
        audited,
        "trace_new_audited",
    );
    let children = [traced, audited];
    let mut router = 0;
    expect_handle(
        aimux_router_new(children.as_ptr(), children.len(), ptr::null(), &mut router),
        router,
        "router_new",
    );
    let mut moa = 0;
    expect_handle(
        aimux_moa_new(
            children.as_ptr(),
            children.len(),
            model,
            ptr::null(),
            &mut moa,
        ),
        moa,
        "moa_new",
    );
    handles.push(aimux_abort_signal_new());
    assert_ne!(*handles.last().unwrap(), 0, "abort_signal_new");

    // Release every handle created above (aimux_drop_handle is safe with 0).
    for h in [traced, audited, router, moa] {
        aimux_drop_handle(h);
    }

    // mock_replay_new: empty input must fail cleanly (no recordings) — the
    // success path needs a real Recording, covered by replay tests elsewhere.
    let mut replay = 7;
    let e = aimux_mock_replay_new(c("").as_ptr(), &mut replay);
    assert_eq!(replay, 0, "failure writes the sentinel");
    assert_eq!(
        expect_aimux_error(e, "mock_replay_new (empty jsonl)").0,
        AIMUX_E_INVALID_ARGUMENT
    );

    for h in handles {
        aimux_drop_handle(h);
    }
}

/// `provider_from_env` reads the provider's env var, which the harness cannot
/// control: a live handle or a clean missing-key error are both correct.
#[test]
fn provider_from_env_handle_or_clean_failure() {
    let mut h = 0;
    let e = aimux_provider_from_env(
        c("groq").as_ptr(),
        c("llama-3.3-70b-versatile").as_ptr(),
        &mut h,
    );
    if e.is_null() {
        assert_ne!(h, 0);
        aimux_drop_handle(h);
    } else {
        assert_eq!(h, 0);
        expect_aimux_error(e, "provider_from_env");
    }
}

// ── session class: text generation (4 exports) ──────────────────────────────

#[test]
fn text_generation_exports_fail_cleanly_on_unreachable_host() {
    let h = unreachable_model();
    let prompt = c("\"ffi smoke\"");
    let opts = c(r#"{"max_retries":0}"#);
    let mut out: *mut c_char = ptr::null_mut();

    let e = aimux_generate_text(h, prompt.as_ptr(), opts.as_ptr(), &mut out);
    expect_ptr_aimux_failure(e, out, "generate_text");

    let e = aimux_generate_object(h, prompt.as_ptr(), opts.as_ptr(), &mut out);
    expect_ptr_aimux_failure(e, out, "generate_object");

    let e = aimux_consume_stream_text(h, prompt.as_ptr(), opts.as_ptr(), &mut out);
    expect_ptr_aimux_failure(e, out, "consume_stream_text");

    let e = aimux_generate_text_as_openai(h, prompt.as_ptr(), opts.as_ptr(), &mut out);
    expect_ptr_aimux_failure(e, out, "generate_text_as_openai");

    aimux_drop_handle(h);
}

// ── session class: streaming with callbacks (4 exports) ─────────────────────

#[test]
fn streaming_exports_fail_cleanly_on_unreachable_host() {
    let h = unreachable_model();
    let prompt = c("\"ffi smoke\"");
    let opts = c(r#"{"max_retries":0}"#);
    let done_before = DONE_SEEN.load(Ordering::Relaxed);

    let e = aimux_stream_text(
        h,
        prompt.as_ptr(),
        opts.as_ptr(),
        Some(smoke_on_part),
        Some(smoke_on_done),
        ptr::null_mut(),
    );
    expect_aimux_error(e, "stream_text");

    let abort = aimux_abort_signal_new();
    let e = aimux_stream_text_with_abort(
        h,
        abort,
        prompt.as_ptr(),
        opts.as_ptr(),
        Some(smoke_on_part),
        Some(smoke_on_done),
        ptr::null_mut(),
    );
    expect_aimux_error(e, "stream_text_with_abort");

    let e = aimux_stream_text_as_openai(
        h,
        prompt.as_ptr(),
        opts.as_ptr(),
        Some(smoke_on_part),
        Some(smoke_on_done),
        ptr::null_mut(),
    );
    expect_aimux_error(e, "stream_text_as_openai");

    let e = aimux_stream_text_as_openai_with_abort(
        h,
        abort,
        prompt.as_ptr(),
        opts.as_ptr(),
        Some(smoke_on_part),
        Some(smoke_on_done),
        ptr::null_mut(),
    );
    expect_aimux_error(e, "stream_text_as_openai_with_abort");

    // Failure never calls on_done.
    assert_eq!(DONE_SEEN.load(Ordering::Relaxed), done_before);
    aimux_abort_signal_drop(abort);
    aimux_drop_handle(h);
}

// ── session class: multimodal generation (8 exports, one per test to
// parallelise the default retry backoff — their option structs expose no
// max_retries override) ───────────────────────────────────────────────────────

#[test]
fn embed_fails_cleanly_on_unreachable_host() {
    let mut h = 0;
    let e = aimux_openai_embedding_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("text-embedding-3-small").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut h,
    );
    expect_handle(e, h, "embedding handle");
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_embed(
        h,
        c(r#"["hello"]"#).as_ptr(),
        c(r#"{"values":[]}"#).as_ptr(),
        &mut out,
    );
    expect_ptr_aimux_failure(e, out, "embed");
    aimux_drop_handle(h);
}

#[test]
fn speech_generate_fails_cleanly_on_unreachable_host() {
    let mut h = 0;
    let e = aimux_openai_speech_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("tts-1").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut h,
    );
    expect_handle(e, h, "speech handle");
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_speech_generate(h, c(r#"{"text":"ffi smoke"}"#).as_ptr(), &mut out);
    expect_ptr_aimux_failure(e, out, "speech_generate");
    aimux_drop_handle(h);
}

#[test]
fn image_generate_fails_cleanly_on_unreachable_host() {
    let mut h = 0;
    let e = aimux_openai_image_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("gpt-image-1").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut h,
    );
    expect_handle(e, h, "image handle");
    let opts = c(r#"{"prompt":"a rust crab","n":1,"provider_options":{}}"#);
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_image_generate(h, opts.as_ptr(), &mut out);
    expect_ptr_aimux_failure(e, out, "image_generate");
    aimux_drop_handle(h);
}

#[test]
fn transcription_generate_fails_cleanly_on_unreachable_host() {
    let mut h = 0;
    let e = aimux_openai_transcription_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("whisper-1").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut h,
    );
    expect_handle(e, h, "transcription handle");
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_transcription_generate(
        h,
        c("ZmFrZS1hdWRpbw==").as_ptr(),
        c("audio/wav").as_ptr(),
        ptr::null(),
        &mut out,
    );
    expect_ptr_aimux_failure(e, out, "transcription_generate");
    aimux_drop_handle(h);
}

#[test]
fn file_upload_fails_cleanly_on_unreachable_host() {
    let mut h = 0;
    let e = aimux_openai_files_new_with_base(c(FAKE_KEY).as_ptr(), c(UNREACHABLE).as_ptr(), &mut h);
    expect_handle(e, h, "files handle");
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_file_upload(
        h,
        c("ZmFrZS1maWxl").as_ptr(),
        c("text/plain").as_ptr(),
        ptr::null(),
        &mut out,
    );
    expect_ptr_aimux_failure(e, out, "file_upload");
    aimux_drop_handle(h);
}

#[test]
fn rerank_fails_cleanly_on_unreachable_host() {
    let mut h = 0;
    let e = aimux_cohere_reranking_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("rerank-v3.5").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut h,
    );
    expect_handle(e, h, "reranking handle");
    let opts = c(r#"{"query":"rust","documents":{"Text":{"values":["a","b"]}}}"#);
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_rerank(h, opts.as_ptr(), &mut out);
    expect_ptr_aimux_failure(e, out, "rerank");
    aimux_drop_handle(h);
}

#[test]
fn video_generate_fails_cleanly_on_unreachable_host() {
    let mut h = 0;
    let e = aimux_google_video_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("veo-3.0-generate-001").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut h,
    );
    expect_handle(e, h, "video handle");
    let opts = c(r#"{"prompt":"waves","n":1,"provider_options":{}}"#);
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_video_generate(h, opts.as_ptr(), &mut out);
    expect_ptr_aimux_failure(e, out, "video_generate");
    aimux_drop_handle(h);
}

#[test]
fn search_fails_cleanly_on_unreachable_host() {
    let mut h = 0;
    let e = aimux_tavily_search_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("tavily").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut h,
    );
    expect_handle(e, h, "search handle");
    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_search(h, c(r#"{"query":"ffi smoke"}"#).as_ptr(), &mut out);
    expect_ptr_aimux_failure(e, out, "search");
    aimux_drop_handle(h);
}

// ── session class: transcription streaming session (5 exports) ──────────────

#[test]
fn transcription_session_reaches_clean_terminal_error() {
    let mut model = 0;
    let e = aimux_openai_transcription_new_with_base(
        c(FAKE_KEY).as_ptr(),
        c("whisper-1").as_ptr(),
        c(UNREACHABLE).as_ptr(),
        &mut model,
    );
    expect_handle(e, model, "transcription handle");

    let mut session = 0;
    let e = aimux_transcription_session_new(model, 0, ptr::null(), &mut session);
    expect_handle(e, session, "transcription_session_new");

    // The driver fails on the unreachable host; pull parts until the stream
    // reaches its terminal state (error or ENDED) within a bounded number of
    // timed pulls. This is exactly the smoke point: a clean error path.
    let mut reached_terminal = false;
    for _ in 0..3 {
        let mut part: *mut c_char = ptr::null_mut();
        let mut state: i32 = 0;
        let e = aimux_transcription_next_part(session, 3_000, &mut part, &mut state);
        if !e.is_null() {
            assert!(part.is_null(), "failure leaves *out_part NULL");
            expect_failure(e, "next_part (terminal)");
            reached_terminal = true;
            break;
        }
        match state {
            aimux_ffi::AIMUX_TRANSCRIPTION_NEXT_PART_PART => {
                read_and_free_json(part, "transcription part");
            }
            aimux_ffi::AIMUX_TRANSCRIPTION_NEXT_PART_ENDED => {
                assert!(part.is_null());
                reached_terminal = true;
                break;
            }
            aimux_ffi::AIMUX_TRANSCRIPTION_NEXT_PART_TIMEOUT => assert!(part.is_null()),
            other => panic!("unknown next_part state {other}"),
        }
    }
    assert!(
        reached_terminal,
        "session must reach a terminal state within a few pulls"
    );

    // After the terminal state a push is either rejected (error) or still
    // buffered (NULL) — both clean, non-panicking outcomes.
    let chunk = [0u8, 1, 2, 3];
    let e = aimux_transcription_push_audio(session, chunk.as_ptr(), chunk.len());
    aimux_error_free(e);

    ok(
        aimux_transcription_input_done(session),
        "input_done on a live session handle",
    );

    // Drop, then verify the handle is gone (clean failure, no panic).
    aimux_transcription_session_drop(session);
    let e = aimux_transcription_push_audio(session, chunk.as_ptr(), chunk.len());
    assert_eq!(
        expect_ffi_error(e, "push after drop"),
        "invalid or expired transcription session handle"
    );
    aimux_drop_handle(model);
}

// ── session class: provider discovery + catalogue + codex (4 exports) ───────

#[test]
fn provider_discovery_and_catalogue_exports_fail_cleanly() {
    // Provider handle bound to the unreachable host with retries disabled.
    let mut provider = 0;
    let e = aimux_provider_handle_new(
        c("groq").as_ptr(),
        c(FAKE_KEY).as_ptr(),
        c(r#"{"base_url":"http://127.0.0.1:1","max_retries":0}"#).as_ptr(),
        &mut provider,
    );
    expect_handle(e, provider, "provider_handle_new");

    let mut out: *mut c_char = ptr::null_mut();
    let e = aimux_provider_list_models(provider, &mut out);
    expect_ptr_aimux_failure(e, out, "provider_list_models");

    let mut model = 0;
    let e = aimux_provider_model(provider, c("llama-3.3-70b-versatile").as_ptr(), &mut model);
    expect_handle(e, model, "provider_model");
    aimux_drop_handle(model);
    aimux_drop_handle(provider);

    // Catalogue fetch pointed at the unreachable host.
    let e = aimux_get_model_specs(c(UNREACHABLE).as_ptr(), &mut out);
    expect_ptr_aimux_failure(e, out, "get_model_specs");

    // Codex OAuth refresh: NULL args fail cleanly in the C ABI without
    // touching the network.
    let e = aimux_codex_refresh(ptr::null(), ptr::null(), &mut out);
    assert!(out.is_null());
    assert_eq!(
        expect_ffi_error(e, "codex_refresh (null args)"),
        "refresh_token: must not be NULL"
    );
}

// ── utility class: lifecycle, sessions, config ──────────────────────────────

#[test]
fn utility_exports_return_clean_values() {
    ok(aimux_init_logging(c("warn").as_ptr()), "init_logging");
    ok(
        aimux_init_logging(ptr::null()),
        "init_logging (NULL = default)",
    );
    aimux_session_store_init();
    aimux_session_infer_init(0);

    let mut out: *mut c_char = ptr::null_mut();
    ok(
        aimux_session_calls(c("ffi-smoke-session").as_ptr(), &mut out),
        "session_calls",
    );
    let calls = read_and_free_json(out, "session_calls");
    assert!(calls.starts_with('['), "session_calls returns a JSON array");

    ok(aimux_list_sessions(&mut out), "list_sessions");
    let sessions = read_and_free_json(out, "list_sessions");
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
    ok(
        aimux_register_providers(valid.as_ptr()),
        "register_providers (valid)",
    );
    let e = aimux_register_providers(c("{not json").as_ptr());
    // Malformed JSON text is this C layer's finding. AiMuxError-only payload
    // getters answer their sentinels for its unified FFI code.
    assert!(!e.is_null());
    assert_eq!(aimux_error_code(e), AIMUX_E_FFI_INVALID_WIRE_JSON);
    assert_eq!(aimux_error_retryable(e), 0);
    assert_eq!(aimux_error_status(e), -1);
    assert_eq!(aimux_error_retry_ms(e), -1);
    for get in [
        aimux_error_provider_code,
        aimux_error_provider_message,
        aimux_error_request_id,
        aimux_error_response_body,
        aimux_error_model_id,
        aimux_error_model_type,
        aimux_error_provider_id,
        aimux_error_tool_name,
        aimux_error_available_tools,
        aimux_error_tool_input,
        aimux_error_original_error,
    ] {
        assert!(get(e).is_null(), "AiMuxError payload getter must be NULL");
    }
    let m = aimux_error_message(e);
    assert!(!m.is_null());
    assert!(
        unsafe { CStr::from_ptr(m) }
            .to_str()
            .unwrap()
            .starts_with("config_json: invalid JSON:")
    );
    unsafe { aimux_free_string(m) };
    aimux_error_free(e);
    // NULL means success.
    assert_eq!(aimux_error_code(ptr::null()), AIMUX_OK);
    assert!(aimux_error_message(ptr::null()).is_null());
    // NULL owner: accessors are NULL-safe, free is a no-op.
    assert!(aimux_error_message(ptr::null()).is_null());
    aimux_error_free(ptr::null_mut());

    // Empty proxy config is the documented no-op-style success.
    ok(aimux_init_proxy(c("{}").as_ptr()), "init_proxy");
}

// ── utility class: trace queries ────────────────────────────────────────────

#[test]
fn trace_query_exports_return_clean_values() {
    let model = unreachable_model();
    let mut traced = 0;
    expect_handle(aimux_trace_new(model, &mut traced), traced, "trace handle");

    // Empty filter `{}` = all (TraceFilter is all-Option).
    let mut out: *mut c_char = ptr::null_mut();
    ok(
        aimux_trace_aggregate(traced, c("{}").as_ptr(), &mut out),
        "trace_aggregate",
    );
    let stats = read_and_free_json(out, "trace_aggregate");
    assert!(
        stats.starts_with('['),
        "trace_aggregate returns a JSON array"
    );

    // Unknown session: documented clean AiMuxError.
    let e = aimux_trace_session_chain(traced, c("ffi-smoke-unknown").as_ptr(), &mut out);
    assert!(out.is_null());
    let (code, m) = expect_aimux_error(e, "trace_session_chain (unknown session)");
    assert_eq!(code, AIMUX_E_INVALID_ARGUMENT);
    assert!(m.contains("unknown session"), "{m}");

    ok(
        aimux_trace_session_trajectory(traced, c("ffi-smoke-unknown").as_ptr(), &mut out),
        "trace_session_trajectory",
    );
    let trajectory = read_and_free_json(out, "trace_session_trajectory");
    assert!(
        trajectory.starts_with('['),
        "trace_session_trajectory returns a JSON array"
    );

    ok(
        aimux_trace_export_jsonl(traced, &mut out),
        "trace_export_jsonl",
    );
    assert!(!out.is_null(), "trace_export_jsonl must return a string");
    let jsonl = take(out);
    assert!(
        jsonl.is_empty() || jsonl.lines().all(|l| !l.trim().is_empty()),
        "trace_export_jsonl is empty-or-JSONL"
    );

    ok(aimux_trace_clear(traced), "trace_clear on live handle");
    assert_eq!(
        expect_ffi_error(aimux_trace_clear(999_999), "trace_clear on unknown handle"),
        "invalid or expired trace handle"
    );

    aimux_drop_handle(traced);
    aimux_drop_handle(model);
    // A released trace handle is gone from the trace registry too.
    let e = aimux_trace_export_jsonl(traced, &mut out);
    assert!(out.is_null());
    assert_eq!(msg(e), "invalid or expired trace handle");
}

// ── utility class: recording ────────────────────────────────────────────────

#[test]
fn recording_exports_lifecycle_cleanly() {
    // cap == 0 is the documented failure.
    assert_eq!(
        expect_aimux_error(aimux_init_recording_ring(0), "init_recording_ring(0)").0,
        AIMUX_E_INVALID_ARGUMENT
    );

    ok(aimux_init_recording_ring(8), "init_recording_ring");
    aimux_recording_flush();
    ok(aimux_recording_try_flush(), "recording_try_flush");

    // File-backed recorder in a throwaway temp dir.
    let dir = std::env::temp_dir().join(format!("aimux-ffi-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    ok(
        aimux_init_recording(c(dir.to_str().expect("utf8 temp dir")).as_ptr()),
        "init_recording",
    );

    aimux_init_recording_ring_default();
    aimux_recording_stop();
    aimux_recording_flush();
    ok(
        aimux_recording_try_flush(),
        "recording_try_flush after stop",
    );
    let _ = std::fs::remove_dir_all(&dir);
}
