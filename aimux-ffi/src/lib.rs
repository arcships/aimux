//! aimux-ffi: C ABI for multi-language bindings.
//!
//! Provides an opaque handle registry + JSON wire format + push callback
//! stream. The C ABI is consumed by C/C++, Go, Kotlin, Java, Swift, and
//! Flutter. Native bindings (Python / Node) bypass this layer and use
//! `aimux-providers` directly.
//!
//! ## Errors
//!
//! Every fallible C function returns `*mut aimux_error_t`: NULL on
//! success (the result is written to the out-parameter), a heap-owned error
//! value on failure (the out-parameter is left at its sentinel: handle 0,
//! pointer NULL). Every non-NULL error has one code from [`aimux_error_code`]
//! and one message from [`aimux_error_message`], and is released exactly once
//! with [`aimux_error_free`]. Codes 1..13 come from `AiMuxError`, 100..105
//! from `RecordingError`, and 200..206 identify failures detected while
//! crossing the C ABI.
//!
//! ## Memory ownership
//!
//! - Every function returning `*mut c_char` — error getters included —
//!   transfers ownership to the caller, who MUST free it with
//!   [`aimux_free_string`]. Handles (`u64`) are released with
//!   [`aimux_drop_handle`].
//! - [`aimux_stream_text`] callbacks receive `*const c_char` pointers that are
//!   valid **only for the duration of the callback**. The callback must copy
//!   the data synchronously; the backing buffer is freed when the callback
//!   returns.
//!
//! ## Concurrency
//!
//! All async provider work runs on a shared multi-threaded tokio runtime. The
//! C ABI functions are synchronous: they `block_on` the runtime until the
//! operation completes. Callbacks execute on the same thread/call-stack that
//! invoked the FFI function, so they must **not** re-enter the FFI layer:
//! a nested `block_on` on the same thread makes tokio **panic** ("Cannot start
//! a runtime from within a runtime"). Rust's non-unwind `extern "C"` ABI does
//! not allow a panic to propagate — the process terminates (and under this
//! workspace's release profile, `panic = "abort"`, it terminates at the panic
//! site). Either way the re-entrant call must be rejected before it reaches
//! the runtime; the thread-local guard in `ffi_block_on` does that
//! (issue M7).
#![allow(clippy::not_unsafe_ptr_arg_deref)]
// `extern "C"` entry points dereference raw pointers (`*const c_char`) by
// design: the C ABI contract requires callers to pass valid pointers (see
// memory-ownership docs above), so the functions are safe only on the C side.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::de::DeserializeOwned;

use aimux_core::AiMuxError;
use aimux_core::generate::{
    GenerateTextOptions, generate_object, generate_text, generate_text_as_openai, stream_text,
    stream_text_as_openai,
};
use aimux_core::language_model::LanguageModel;
use aimux_core::message::ModelPrompt;
use aimux_core::openai_output::OpenAiStreamOptions;
use aimux_core::provider::Provider;
use aimux_core::recording::RecordingError;
use aimux_core::shared::AbortSignal;
use aimux_core::trace::{RingTraceStore, TraceFilter, TraceLayer};
use aimux_providers::anthropic::{AnthropicConfig, AnthropicProvider};
use aimux_providers::anthropic_aws::{AnthropicAwsProvider, AnthropicAwsProviderConfig};
use aimux_providers::azure::{AzureConfig, AzureProvider};
use aimux_providers::bedrock::{BedrockProvider, BedrockProviderConfig};
use aimux_providers::cohere::{CohereConfig, CohereProvider};
use aimux_providers::google::{GoogleConfig, GoogleProvider};
use aimux_providers::mistral::{MistralConfig, MistralProvider};
use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};
use aimux_providers::tavily::{TavilyConfig, TavilyProvider};
use aimux_providers::vertex::{VertexProvider, VertexProviderConfig};
use aimux_providers::xai::{XAIConfig, XAIProvider};
use aimux_providers::{ProviderOptions, provider, provider_handle};

use futures::StreamExt;
use tokio::runtime::Runtime;

// ─────────────────────────────────────────────────────────────────────────────
// Global state: handle registry + tokio runtime
// ─────────────────────────────────────────────────────────────────────────────

/// A type-erased FFI handle. One registry holds models, providers, sessions
/// and abort signals.
#[derive(Clone)]
enum HandleEntry {
    Language(Arc<dyn LanguageModel>),
    Provider(Arc<dyn aimux_core::provider::Provider>),
    Embedding(Arc<dyn aimux_core::embedding_model::EmbeddingModel>),
    Speech(Arc<dyn aimux_core::speech_model::SpeechModel>),
    Image(Arc<dyn aimux_core::image_model::ImageModel>),
    Transcription(Arc<dyn aimux_core::transcription_model::TranscriptionModel>),
    Reranking(Arc<dyn aimux_core::reranking_model::RerankingModel>),
    Video(Arc<dyn aimux_core::video_model::VideoModel>),
    Search(Arc<dyn aimux_core::search_model::SearchModel>),
    Files(Arc<dyn aimux_core::files_model::Files>),
    /// Live transcription streaming session (RFC-0028 Phase 2).
    TranscriptionSession(Arc<transcription_session::TranscriptionFfiSession>),
    Abort(AbortSignal),
}

type Registry = HashMap<u64, HandleEntry>;

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register a model instance, returning its opaque `u64` handle.
///
/// Handles start at 1; 0 is reserved for "failure / invalid".
fn intern_model(model: Arc<dyn LanguageModel>) -> u64 {
    intern_handle(HandleEntry::Language(model))
}

fn intern_handle(h: HandleEntry) -> u64 {
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    registry()
        .lock()
        .expect("aimux-ffi: registry mutex poisoned")
        .insert(handle, h);
    handle
}

/// Look up any handle.
fn get_handle(handle: u64) -> Option<HandleEntry> {
    registry()
        .lock()
        .expect("aimux-ffi: registry mutex poisoned")
        .get(&handle)
        .cloned()
}

/// Look up a model by handle, cloning the `Arc` out of the registry.
fn get_model(handle: u64) -> Option<Arc<dyn LanguageModel>> {
    match get_handle(handle)? {
        HandleEntry::Language(m) => Some(m),
        _ => None,
    }
}

/// A model handle, or [`FfiError::InvalidHandle`] (`"model"`).
fn model_of(handle: u64) -> Result<Arc<dyn LanguageModel>, FfiError> {
    get_model(handle).ok_or(FfiError::InvalidHandle { expected: "model" })
}

/// A live handle of any registered type, or [`FfiError::InvalidHandle`] naming what was
/// expected. Callers match the arm they need and fall back to the same error.
fn entry_of(handle: u64, expected: &'static str) -> Result<HandleEntry, FfiError> {
    get_handle(handle).ok_or(FfiError::InvalidHandle { expected })
}

fn get_abort_signal(handle: u64) -> Option<AbortSignal> {
    match get_handle(handle)? {
        HandleEntry::Abort(signal) => Some(signal),
        _ => None,
    }
}

fn abort_of(handle: u64) -> Result<AbortSignal, FfiError> {
    get_abort_signal(handle).ok_or(FfiError::InvalidHandle { expected: "abort" })
}

/// Remove a handle from the registry (the model drops when the last ref goes).
/// Trace stores bound to the handle are released with it.
fn drop_handle(handle: u64) {
    let removed = registry()
        .lock()
        .expect("aimux-ffi: registry mutex poisoned")
        .remove(&handle);
    // A transcription session owns a driver task: abort and join it here too,
    // so the generic drop is never a silent leak (the registry mutex is
    // already released — the join must not hold it).
    if let Some(HandleEntry::TranscriptionSession(s)) = removed {
        s.terminate();
    }
    if let Some(stores) = TRACE_STORES.get() {
        stores
            .lock()
            .expect("aimux-ffi: trace registry mutex poisoned")
            .remove(&handle);
    }
}

fn drop_abort_signal(handle: u64) {
    let mut registry = registry()
        .lock()
        .expect("aimux-ffi: registry mutex poisoned");
    if matches!(registry.get(&handle), Some(HandleEntry::Abort(_))) {
        registry.remove(&handle);
    }
}

/// Transcription streaming sessions (RFC-0028 Phase 2).
mod transcription_session;

/// The shared tokio runtime driving all async provider calls.
fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Runtime::new().expect("aimux-ffi: failed to build tokio runtime")
    })
}

thread_local! {
    /// Re-entrancy guard: set while an FFI entry point is `block_on`-ing the
    /// shared runtime on the current thread.
    ///
    /// Stream callbacks (`on_part`/`on_done`) run synchronously on
    /// the same thread/call-stack that entered the FFI function, so a callback
    /// that calls back into the FFI layer would enter a second `block_on` on
    /// this thread. tokio rejects nested `block_on` with a **panic**; Rust's
    /// non-unwind `extern "C"` ABI terminates the process rather than letting
    /// the panic propagate (and `panic = "abort"` terminates at the panic site).
    /// `ffi_block_on` checks this guard and turns that re-entrant call into
    /// a [`FfiError::ReentrantCall`] instead (issue M7).
    static IN_FFI_BLOCK_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run a future on the shared runtime from an FFI entry point (issue M7).
///
/// Rejects re-entrant calls made from inside a stream callback, returning
/// [`FfiError::ReentrantCall`] instead of letting tokio's nested `block_on` panic —
/// a non-unwind `extern "C"` call never lets the panic propagate, so the
/// process would terminate. The guard is released when the future completes,
/// including when it panics.
fn ffi_block_on<F, T>(f: F) -> Result<T, FfiError>
where
    F: std::future::Future<Output = T>,
{
    IN_FFI_BLOCK_ON.with(|flag| {
        if flag.replace(true) {
            return Err(FfiError::ReentrantCall);
        }
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                IN_FFI_BLOCK_ON.with(|flag| flag.set(false));
            }
        }
        let _reset = Reset;
        Ok(runtime().block_on(f))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Error model (aimux-error.h)
// ─────────────────────────────────────────────────────────────────────────────

/// A failure detected while lifting arguments, using handles, invoking a
/// callback, or lowering a result across the C ABI.
#[derive(Debug, Clone)]
enum FfiError {
    /// A required pointer argument was NULL.
    NullPointer { argument: &'static str },
    /// A string argument was not valid UTF-8.
    InvalidUtf8 { argument: &'static str },
    /// A JSON-text argument of this wire format (`prompt_json`, `opts_json`,
    /// `config_json`, …) did not parse. Not `AiMuxError::JsonParse` — that
    /// one is about provider responses.
    InvalidWireJson {
        argument: &'static str,
        message: String,
    },
    /// A handle argument is 0, released, or has the wrong handle type.
    InvalidHandle { expected: &'static str },
    /// The FFI was re-entered from inside one of its own callbacks.
    ReentrantCall,
    /// A result could not be serialized to the wire format.
    ResultSerialization { message: String },
    /// A host callback panicked (caught inside the C ABI).
    CallbackFailure { message: String },
}

impl std::fmt::Display for FfiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FfiError::NullPointer { argument } => write!(f, "{argument}: must not be NULL"),
            FfiError::InvalidUtf8 { argument } => write!(f, "{argument}: must be valid UTF-8"),
            FfiError::InvalidWireJson { argument, message } => {
                write!(f, "{argument}: invalid JSON: {message}")
            }
            FfiError::InvalidHandle { expected } => {
                write!(f, "invalid or expired {expected} handle")
            }
            FfiError::ReentrantCall => {
                f.write_str("re-entrant FFI call from within a callback is not allowed")
            }
            FfiError::ResultSerialization { message } => {
                write!(f, "could not serialize result: {message}")
            }
            FfiError::CallbackFailure { message } => write!(f, "host callback failed: {message}"),
        }
    }
}

/// The errors that can cross the Aimux C ABI.
enum AiMuxFfiError {
    AiMux(AiMuxError),
    Recording(RecordingError),
    Ffi(FfiError),
}

impl From<AiMuxError> for AiMuxFfiError {
    fn from(inner: AiMuxError) -> Self {
        AiMuxFfiError::AiMux(inner)
    }
}
impl From<RecordingError> for AiMuxFfiError {
    fn from(inner: RecordingError) -> Self {
        AiMuxFfiError::Recording(inner)
    }
}
impl From<FfiError> for AiMuxFfiError {
    fn from(e: FfiError) -> Self {
        AiMuxFfiError::Ffi(e)
    }
}

impl std::fmt::Display for AiMuxFfiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiMuxFfiError::AiMux(e) => e.fmt(f),
            AiMuxFfiError::Recording(e) => e.fmt(f),
            AiMuxFfiError::Ffi(e) => e.fmt(f),
        }
    }
}

/// The owned error returned by a failed C ABI invocation. Opaque to C: read it
/// with the `aimux_error_*` getters, then release it once with
/// `aimux_error_free`.
#[allow(non_camel_case_types)]
pub struct aimux_error_t {
    error: AiMuxFfiError,
}

type FfiResult<T> = Result<T, AiMuxFfiError>;

/// Ok → run `write` with the value and return NULL; Err → hand the owned
/// error to the caller.
fn finish<T>(r: FfiResult<T>, write: impl FnOnce(T)) -> *mut aimux_error_t {
    match r {
        Ok(v) => {
            write(v);
            std::ptr::null_mut()
        }
        Err(error) => Box::into_raw(Box::new(aimux_error_t { error })),
    }
}

/// Entry-point shape: `uint64_t *out_handle`. NULL out-param is a
/// `NullPointer("out_handle")`; the sentinel 0 is written before `f` runs.
fn with_out_handle(out_handle: *mut u64, f: impl FnOnce() -> FfiResult<u64>) -> *mut aimux_error_t {
    if out_handle.is_null() {
        return finish(
            Err(FfiError::NullPointer {
                argument: "out_handle",
            }
            .into()),
            |_: u64| {},
        );
    }
    unsafe { *out_handle = 0 };
    finish(f(), |h| unsafe { *out_handle = h })
}

/// Entry-point shape: `char **out_json` (name given by `argument`). NULL
/// out-param is a `NullPointer`; the sentinel NULL is written before `f` runs.
fn with_out_string(
    out: *mut *mut c_char,
    argument: &'static str,
    f: impl FnOnce() -> FfiResult<String>,
) -> *mut aimux_error_t {
    if out.is_null() {
        return finish(
            Err(FfiError::NullPointer { argument }.into()),
            |_: String| {},
        );
    }
    unsafe { *out = std::ptr::null_mut() };
    finish(f(), |s| unsafe { *out = into_cstring_raw(s) })
}

/// Entry-point shape: no result.
fn no_result(f: impl FnOnce() -> FfiResult<()>) -> *mut aimux_error_t {
    finish(f(), |()| {})
}

// ── aimux_error_* ────────────────────────────────────────────────────────────

/// Release a returned error. NULL-safe; call exactly once.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_free(err: *mut aimux_error_t) {
    if !err.is_null() {
        // SAFETY: `err` came from `Box::into_raw` in `finish` and the caller
        // passes it exactly once.
        drop(unsafe { Box::from_raw(err) });
    }
}

// ── Unified error codes (aimux-error.h `aimux_error_code_t`) ─────────────────

pub const AIMUX_OK: i32 = 0;
// 1 is the catch-all `Other` (it was `AIMUX_E_UNKNOWN`, whose slot it inherits:
// a code a binding does not know is a header mismatch, not a user-facing code).
pub const AIMUX_E_OTHER: i32 = 1;
pub const AIMUX_E_JSON_PARSE: i32 = 2;
pub const AIMUX_E_INVALID_RESPONSE_DATA: i32 = 3;
pub const AIMUX_E_TOOL: i32 = 4;
pub const AIMUX_E_INVALID_ARGUMENT: i32 = 5;
pub const AIMUX_E_INVALID_PROMPT: i32 = 6;
pub const AIMUX_E_TOKEN_EXPIRED: i32 = 7;
pub const AIMUX_E_UNSUPPORTED_FUNCTIONALITY: i32 = 8;
pub const AIMUX_E_NO_SUCH_MODEL: i32 = 9;
pub const AIMUX_E_NO_SUCH_PROVIDER: i32 = 10;
pub const AIMUX_E_API_CALL: i32 = 11;
pub const AIMUX_E_TIMEOUT: i32 = 12;
pub const AIMUX_E_ABORTED: i32 = 13;

// 100..105 preserve `RecordingError` as a separate high-level type while C
// uses one code space for every returned error.
pub const AIMUX_E_RECORDING_INIT: i32 = 100;
pub const AIMUX_E_RECORDING_OPEN_FILE: i32 = 101;
pub const AIMUX_E_RECORDING_SPAWN: i32 = 102;
pub const AIMUX_E_RECORDING_WRITER_GONE: i32 = 103;
pub const AIMUX_E_RECORDING_FLUSH_TIMEOUT: i32 = 104;
pub const AIMUX_E_RECORDING_WRITE: i32 = 105;

// 200..206 are failures detected while lifting arguments, looking up handles,
// invoking callbacks, or lowering a result across the C ABI.
pub const AIMUX_E_FFI_NULL_POINTER: i32 = 200;
pub const AIMUX_E_FFI_INVALID_UTF8: i32 = 201;
pub const AIMUX_E_FFI_INVALID_WIRE_JSON: i32 = 202;
pub const AIMUX_E_FFI_INVALID_HANDLE: i32 = 203;
pub const AIMUX_E_FFI_REENTRANT_CALL: i32 = 204;
pub const AIMUX_E_FFI_RESULT_SERIALIZATION: i32 = 205;
pub const AIMUX_E_FFI_CALLBACK_FAILURE: i32 = 206;

// ── `aimux_transcription_next_part_state_t` ──────────────────────────────────
pub const AIMUX_TRANSCRIPTION_NEXT_PART_PART: i32 = 1;
pub const AIMUX_TRANSCRIPTION_NEXT_PART_ENDED: i32 = 2;
pub const AIMUX_TRANSCRIPTION_NEXT_PART_TIMEOUT: i32 = 3;

fn aimux_error_code_of(err: &AiMuxError) -> i32 {
    match err {
        AiMuxError::ApiCall { .. } => AIMUX_E_API_CALL,
        AiMuxError::JsonParse(_) => AIMUX_E_JSON_PARSE,
        AiMuxError::InvalidResponseData(_) => AIMUX_E_INVALID_RESPONSE_DATA,
        AiMuxError::Tool(_) => AIMUX_E_TOOL,
        AiMuxError::InvalidArgument(_) => AIMUX_E_INVALID_ARGUMENT,
        AiMuxError::InvalidPrompt(_) => AIMUX_E_INVALID_PROMPT,
        AiMuxError::TokenExpired(_) => AIMUX_E_TOKEN_EXPIRED,
        AiMuxError::UnsupportedFunctionality(_) => AIMUX_E_UNSUPPORTED_FUNCTIONALITY,
        AiMuxError::NoSuchModel { .. } => AIMUX_E_NO_SUCH_MODEL,
        AiMuxError::NoSuchProvider { .. } => AIMUX_E_NO_SUCH_PROVIDER,
        AiMuxError::Timeout(_) => AIMUX_E_TIMEOUT,
        AiMuxError::Aborted => AIMUX_E_ABORTED,
        AiMuxError::Other(_) => AIMUX_E_OTHER,
    }
}

fn recording_error_code_of(e: &RecordingError) -> i32 {
    use RecordingError as R;
    match e {
        R::Init { .. } => AIMUX_E_RECORDING_INIT,
        R::OpenFile { .. } => AIMUX_E_RECORDING_OPEN_FILE,
        R::Spawn { .. } => AIMUX_E_RECORDING_SPAWN,
        R::WriterGone => AIMUX_E_RECORDING_WRITER_GONE,
        R::FlushTimeout => AIMUX_E_RECORDING_FLUSH_TIMEOUT,
        R::Write(_) => AIMUX_E_RECORDING_WRITE,
    }
}

fn ffi_error_code_of(e: &FfiError) -> i32 {
    match e {
        FfiError::NullPointer { .. } => AIMUX_E_FFI_NULL_POINTER,
        FfiError::InvalidUtf8 { .. } => AIMUX_E_FFI_INVALID_UTF8,
        FfiError::InvalidWireJson { .. } => AIMUX_E_FFI_INVALID_WIRE_JSON,
        FfiError::InvalidHandle { .. } => AIMUX_E_FFI_INVALID_HANDLE,
        FfiError::ReentrantCall => AIMUX_E_FFI_REENTRANT_CALL,
        FfiError::ResultSerialization { .. } => AIMUX_E_FFI_RESULT_SERIALIZATION,
        FfiError::CallbackFailure { .. } => AIMUX_E_FFI_CALLBACK_FAILURE,
    }
}

fn error_code_of(e: &AiMuxFfiError) -> i32 {
    match e {
        AiMuxFfiError::AiMux(e) => aimux_error_code_of(e),
        AiMuxFfiError::Recording(e) => recording_error_code_of(e),
        AiMuxFfiError::Ffi(e) => ffi_error_code_of(e),
    }
}

fn api_call(e: &AiMuxError) -> Option<&aimux_core::ApiCallError> {
    match e {
        AiMuxError::ApiCall(d) => Some(d),
        _ => None,
    }
}

/// Apply `f` to the `AiMuxError` stored in a returned error.
///
/// The closure keeps the borrowed error scoped to this call instead of
/// manufacturing an unconstrained lifetime from the raw pointer.
fn map_aimux_error<T>(err: *const aimux_error_t, f: impl FnOnce(&AiMuxError) -> T) -> Option<T> {
    // SAFETY: the C API requires NULL or a live error returned by `finish`.
    match unsafe { err.as_ref() } {
        Some(aimux_error_t {
            error: AiMuxFfiError::AiMux(e),
        }) => Some(f(e)),
        _ => None,
    }
}

fn opt_cstring(v: Option<String>) -> *mut c_char {
    v.map_or(std::ptr::null_mut(), into_cstring_raw)
}

// ── aimux_error_* getters ────────────────────────────────────────────
//
// One getter per fact. `code` and `message` answer for every returned error;
// the rest belong to one AiMuxError code and return NULL / -1 / 0 under every
// other code or for NULL. Strings are owned by the caller (aimux_free_string).

/// Machine-readable code (`aimux_error_code_t`); `AIMUX_OK` for NULL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_code(err: *const aimux_error_t) -> i32 {
    // SAFETY: NULL or a live error returned by `finish`.
    unsafe { err.as_ref() }.map_or(AIMUX_OK, |e| error_code_of(&e.error))
}

/// Human-readable description for every code; NULL only for NULL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_message(err: *const aimux_error_t) -> *mut c_char {
    // SAFETY: NULL or a live error returned by `finish`.
    unsafe { err.as_ref() }.map_or(std::ptr::null_mut(), |e| {
        into_cstring_raw(e.error.to_string())
    })
}

/// HTTP status: the observed status under `AIMUX_E_API_CALL`, 401 by
/// definition under `AIMUX_E_TOKEN_EXPIRED`, -1 otherwise or when no response
/// was observed (`AiMuxError::status_code`).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_status(err: *const aimux_error_t) -> i32 {
    map_aimux_error(err, AiMuxError::status_code)
        .flatten()
        .map_or(-1, i32::from)
}

/// `AIMUX_E_API_CALL`: retry hint in ms (0 = retry now), or -1 when none.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_retry_ms(err: *const aimux_error_t) -> i64 {
    map_aimux_error(err, AiMuxError::retry_after_hint)
        .flatten()
        .unwrap_or(-1)
}

/// 1 when retrying may help, 0 when it will not — the core's verdict. Do not
/// infer it from `status`: a statusless `AIMUX_E_API_CALL` may be either.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_retryable(err: *const aimux_error_t) -> i32 {
    map_aimux_error(err, AiMuxError::is_retryable).unwrap_or(false) as i32
}

/// `AIMUX_E_API_CALL`: the provider's own error code, e.g. "insufficient_quota".
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_provider_code(err: *const aimux_error_t) -> *mut c_char {
    opt_cstring(map_aimux_error(err, |e| api_call(e)?.provider_code.clone()).flatten())
}

/// `AIMUX_E_API_CALL`: the failure's own text ("slow down"), without the
/// composed prefix `message` carries ("API call error: HTTP 429: slow down").
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_provider_message(err: *const aimux_error_t) -> *mut c_char {
    opt_cstring(
        map_aimux_error(err, |e| Some(api_call(e)?.message.clone()))
            .flatten()
            .filter(|m| !m.is_empty()),
    )
}

/// `AIMUX_E_API_CALL`: provider request id.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_request_id(err: *const aimux_error_t) -> *mut c_char {
    opt_cstring(map_aimux_error(err, |e| api_call(e)?.request_id.clone()).flatten())
}

/// `AIMUX_E_API_CALL`: raw response body.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_response_body(err: *const aimux_error_t) -> *mut c_char {
    opt_cstring(map_aimux_error(err, |e| api_call(e)?.response_body.clone()).flatten())
}

/// `AIMUX_E_NO_SUCH_MODEL`: the model id that was asked for.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_model_id(err: *const aimux_error_t) -> *mut c_char {
    opt_cstring(
        map_aimux_error(err, |e| match e {
            AiMuxError::NoSuchModel { model_id, .. } => Some(model_id.clone()),
            _ => None,
        })
        .flatten(),
    )
}

/// `AIMUX_E_NO_SUCH_MODEL`: the model type it was asked for as.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_model_type(err: *const aimux_error_t) -> *mut c_char {
    opt_cstring(
        map_aimux_error(err, |e| match e {
            AiMuxError::NoSuchModel { model_type, .. } => Some(model_type.clone()),
            _ => None,
        })
        .flatten(),
    )
}

/// `AIMUX_E_NO_SUCH_PROVIDER`: the provider id that was asked for.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_provider_id(err: *const aimux_error_t) -> *mut c_char {
    opt_cstring(
        map_aimux_error(err, |e| match e {
            AiMuxError::NoSuchProvider { provider_id } => Some(provider_id.clone()),
            _ => None,
        })
        .flatten(),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Argument helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Read a required string argument, naming it in the failure
/// (`NullPointer` / `InvalidUtf8`).
///
/// # Safety: `ptr` is null or a valid NUL-terminated C string.
fn str_arg(ptr: *const c_char, argument: &'static str) -> Result<String, FfiError> {
    if ptr.is_null() {
        return Err(FfiError::NullPointer { argument });
    }
    // SAFETY: caller guarantees `ptr` is a valid NUL-terminated C string.
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| FfiError::InvalidUtf8 { argument })
}

/// Read an optional string argument: NULL means "absent"; a non-NULL pointer
/// must be valid UTF-8 or it is [`FfiError::InvalidUtf8`] — absent and
/// malformed are different things.
fn opt_str_arg(ptr: *const c_char, argument: &'static str) -> Result<Option<String>, FfiError> {
    if ptr.is_null() {
        return Ok(None);
    }
    str_arg(ptr, argument).map(Some)
}

/// Build (a, b) from two required C strings.
fn parse_two_args(
    a: *const c_char,
    an: &'static str,
    b: *const c_char,
    bn: &'static str,
) -> Result<(String, String), FfiError> {
    Ok((str_arg(a, an)?, str_arg(b, bn)?))
}

/// Parse four required C string arguments; any null fails the whole call.
#[allow(clippy::too_many_arguments)]
fn parse_four_args(
    a: *const c_char,
    an: &'static str,
    b: *const c_char,
    bn: &'static str,
    c: *const c_char,
    cn: &'static str,
    d: *const c_char,
    dn: &'static str,
) -> Result<(String, String, String, String), FfiError> {
    Ok((
        str_arg(a, an)?,
        str_arg(b, bn)?,
        str_arg(c, cn)?,
        str_arg(d, dn)?,
    ))
}

/// Parse the base_url argument; NULL or an empty string means unset.
fn parse_base_url(base_url: *const c_char) -> Result<Option<String>, FfiError> {
    Ok(opt_str_arg(base_url, "base_url")?.filter(|url| !url.is_empty()))
}

/// Where a serde failure on a wire-JSON argument belongs. Malformed text
/// (syntax / EOF) is this layer's finding: [`FfiError::InvalidWireJson`].
/// Well-formed JSON of the wrong shape (missing field, wrong type) is what the
/// core would reject: [`AiMuxError::InvalidArgument`]. `detail` is the
/// message to carry (usually `e.to_string()`, sometimes with a line prefix).
fn wire_failure(argument: &'static str, e: &serde_json::Error, detail: String) -> AiMuxFfiError {
    match e.classify() {
        serde_json::error::Category::Data => {
            AiMuxError::InvalidArgument(format!("{argument}: {detail}")).into()
        }
        _ => FfiError::InvalidWireJson {
            argument,
            message: detail,
        }
        .into(),
    }
}

/// [`wire_failure`] with the serde message as the detail.
fn wire_err(argument: &'static str, e: serde_json::Error) -> AiMuxFfiError {
    let detail = e.to_string();
    wire_failure(argument, &e, detail)
}

/// Parse a JSON C-string argument into `T`. NULL / non-UTF-8 → FFI
/// `NullPointer` / `InvalidUtf8`; text that does not parse → FFI
/// `InvalidWireJson`; text that parses but violates `T`'s schema →
/// `AiMuxError::InvalidArgument` (see [`wire_failure`]).
fn parse_json_arg<T: DeserializeOwned>(json: *const c_char, name: &'static str) -> FfiResult<T> {
    let s = str_arg(json, name)?;
    serde_json::from_str::<T>(&s).map_err(|e| wire_err(name, e))
}

/// Parse the prompt JSON accepted by the FFI (`prompt_json`, required).
///
/// Accepts either a bare prompt value (`"text"` or `[{...}]`) or a wrapper
/// object `{"prompt": <value>}`.
fn parse_prompt_arg(prompt_json: *const c_char) -> FfiResult<ModelPrompt> {
    let s = str_arg(prompt_json, "prompt_json")?;
    let parse = |json: &str| -> Result<ModelPrompt, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let inner = match &value {
            serde_json::Value::Object(obj) if obj.len() == 1 && obj.contains_key("prompt") => {
                obj.get("prompt").expect("checked by guard")
            }
            _ => &value,
        };
        serde_json::from_value(inner.clone())
    };
    parse(&s).map_err(|e| wire_err("prompt_json", e))
}

/// Parse the options JSON (`opts_json`, optional). NULL / empty / `null`
/// yields the default options.
fn parse_opts_arg(opts_json: *const c_char) -> FfiResult<GenerateTextOptions> {
    match opt_str_arg(opts_json, "opts_json")? {
        Some(s) if !s.trim().is_empty() && s.trim() != "null" => {
            serde_json::from_str(&s).map_err(|e| wire_err("opts_json", e))
        }
        _ => Ok(GenerateTextOptions::default()),
    }
}

/// Parse the optional `config_json` argument (`ProviderOptions`); NULL,
/// empty or "null" means unset.
fn parse_provider_options(config_json: *const c_char) -> FfiResult<Option<ProviderOptions>> {
    match opt_str_arg(config_json, "config_json")? {
        Some(s) if !s.trim().is_empty() && s.trim() != "null" => {
            serde_json::from_str::<ProviderOptions>(&s)
                .map(Some)
                .map_err(|e| wire_err("config_json", e))
        }
        _ => Ok(None),
    }
}

/// Read a config-style JSON C string and normalize it for lenient
/// deserialization: NULL, empty, whitespace-only and `"null"` all become
/// `"{}"` (defaults). Invalid UTF-8 is [`FfiError::InvalidUtf8`].
fn normalize_config_json(json: *const c_char, argument: &'static str) -> Result<String, FfiError> {
    Ok(match opt_str_arg(json, argument)? {
        Some(s) if !s.trim().is_empty() && s.trim() != "null" => s,
        _ => String::from("{}"),
    })
}

/// Build an owned C string (`*mut c_char`) from a `String`, transferring
/// ownership to the caller (who must free it with [`aimux_free_string`]).
///
/// Never returns null (issue M1): interior NUL bytes — which would make
/// `CString::new` fail — are replaced with U+FFFD so the C-side contract
/// ("a non-null NUL-terminated buffer to free, or null") always holds. The
/// replacement is reported through `tracing` (RFC-0014 logging, which C hosts
/// route via `aimux_init_logging`) so accidental NULs stay visible without
/// the library writing to the host's stderr directly.
fn into_cstring_raw(s: String) -> *mut c_char {
    if s.contains('\0') {
        tracing::warn!(
            target: "aimux_ffi",
            nul_bytes = s.bytes().filter(|&b| b == 0).count(),
            "string contains NUL byte(s); replaced with U+FFFD before FFI return"
        );
    }
    let sanitized = s.replace('\0', "\u{FFFD}");
    // No interior NUL remains: this cannot fail.
    CString::new(sanitized)
        .expect("aimux-ffi: impossible: NUL-free string rejected by CString::new")
        .into_raw()
}

/// Serialize a result to its wire JSON, or [`FfiError::ResultSerialization`].
fn to_json<T: serde::Serialize>(v: &T) -> FfiResult<String> {
    serde_json::to_string(v).map_err(|e| {
        FfiError::ResultSerialization {
            message: format!("serialize: {e}"),
        }
        .into()
    })
}

/// Run an async model operation on the runtime and serialize its result.
fn run_json<F, T>(f: F) -> FfiResult<String>
where
    F: std::future::Future<Output = Result<T, AiMuxError>>,
    T: serde::Serialize,
{
    let v = ffi_block_on(f)??;
    to_json(&v)
}

/// Invoke a stream callback (`on_part`/`on_done`) while catching any panic.
///
/// The callbacks are declared `extern "C-unwind"`, so a *Rust* panic raised
/// inside a Rust-implemented callback (panic=unwind builds, same runtime)
/// propagates back here instead of aborting at the ABI edge (issue #64); this
/// wrapper catches it and converts it to a structured
/// [`FfiError::CallbackFailure`] that ends the stream. Foreign exceptions
/// (C++, JVM, Swift, Dart, Go) are NOT covered — `catch_unwind` may abort on
/// them — so callbacks must not unwind across the C ABI; binding trampolines
/// catch their own language's exceptions. (Release builds use `panic =
/// "abort"`, in which case the
/// process aborts before this point and the code is never produced).
///
/// `AssertUnwindSafe` is required because the callback receives raw pointers
/// (`*const c_char`/`*mut c_void`) that are not `UnwindSafe`; this is sound
/// because we abort the stream on any panic rather than continuing.
fn invoke_stream_callback(callback_name: &str, f: impl FnOnce()) -> Result<(), FfiError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => Ok(()),
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&'static str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("<non-string panic>");
            Err(FfiError::CallbackFailure {
                message: format!("stream callback '{callback_name}' panicked: {msg}"),
            })
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: provider constructors
// ─────────────────────────────────────────────────────────────────────────────

/// Create an OpenAI model instance. AiMuxError: invalid model id.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_new(
    api_key: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let m = OpenAIProvider::new(OpenAIConfig::new(api_key)).language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create an OpenAI model instance with a custom base URL.
///
/// `base_url` may be null (defaults to the provider's standard URL).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let mut config = OpenAIConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let m = OpenAIProvider::new(config).language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create an Anthropic model instance. AiMuxError: invalid model id.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_new(
    api_key: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let m = AnthropicProvider::new(AnthropicConfig::new(api_key)).language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create an Anthropic model instance with a custom base URL.
///
/// `base_url` may be null (defaults to the provider's standard URL).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let mut config = AnthropicConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let m = AnthropicProvider::new(config).language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create an Anthropic-on-AWS model instance (API key + region).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_aws_new(
    api_key: *const c_char,
    region: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let api_key = str_arg(api_key, "api_key")?;
        let region = str_arg(region, "region")?;
        let model_id = str_arg(model_id, "model_id")?;
        let m =
            AnthropicAwsProvider::new(AnthropicAwsProviderConfig::with_api_key(api_key, region))
                .language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create an Anthropic-on-AWS model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_aws_new_with_base(
    api_key: *const c_char,
    region: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let api_key = str_arg(api_key, "api_key")?;
        let region = str_arg(region, "region")?;
        let model_id = str_arg(model_id, "model_id")?;
        let mut config = AnthropicAwsProviderConfig::with_api_key(api_key, region);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let m = AnthropicAwsProvider::new(config).language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create an Azure OpenAI model instance (API key + resource name).
///
/// `api_version` may be null (uses the provider default). The deployment
/// name is passed as `model_id`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_azure_new(
    api_key: *const c_char,
    resource_name: *const c_char,
    deployment: *const c_char,
    api_version: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let api_key = str_arg(api_key, "api_key")?;
        let resource_name = str_arg(resource_name, "resource_name")?;
        let deployment = str_arg(deployment, "deployment")?;
        let mut config = AzureConfig::new()
            .with_api_key(api_key)
            .with_resource_name(resource_name);
        if let Some(v) = opt_str_arg(api_version, "api_version")?.filter(|v| !v.is_empty()) {
            config = config.with_api_version(v);
        }
        let m = AzureProvider::new(config)?.language_model(&deployment)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create an Azure OpenAI model instance with a custom base URL (required,
/// in place of `resource_name`).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_azure_new_with_base(
    api_key: *const c_char,
    base_url: *const c_char,
    deployment: *const c_char,
    api_version: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let api_key = str_arg(api_key, "api_key")?;
        let base_url = str_arg(base_url, "base_url")?;
        let deployment = str_arg(deployment, "deployment")?;
        let mut config = AzureConfig::new()
            .with_api_key(api_key)
            .with_base_url(base_url);
        if let Some(v) = opt_str_arg(api_version, "api_version")?.filter(|v| !v.is_empty()) {
            config = config.with_api_version(v);
        }
        let m = AzureProvider::new(config)?.language_model(&deployment)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create a Bedrock model instance (AWS SigV4 credentials).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_bedrock_new(
    access_key_id: *const c_char,
    secret_access_key: *const c_char,
    region: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (access_key_id, secret_access_key, region, model_id) = parse_four_args(
            access_key_id,
            "access_key_id",
            secret_access_key,
            "secret_access_key",
            region,
            "region",
            model_id,
            "model_id",
        )?;
        let m = BedrockProvider::new(BedrockProviderConfig::new(
            access_key_id,
            secret_access_key,
            region,
        ))
        .language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create a Bedrock model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_bedrock_new_with_base(
    access_key_id: *const c_char,
    secret_access_key: *const c_char,
    region: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (access_key_id, secret_access_key, region, model_id) = parse_four_args(
            access_key_id,
            "access_key_id",
            secret_access_key,
            "secret_access_key",
            region,
            "region",
            model_id,
            "model_id",
        )?;
        let mut config = BedrockProviderConfig::new(access_key_id, secret_access_key, region);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let m = BedrockProvider::new(config).language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create a Vertex AI model instance (GCP bearer token).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_vertex_new(
    access_token: *const c_char,
    project: *const c_char,
    location: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (access_token, project, location, model_id) = parse_four_args(
            access_token,
            "access_token",
            project,
            "project",
            location,
            "location",
            model_id,
            "model_id",
        )?;
        let m = VertexProvider::new(VertexProviderConfig::new(access_token, project, location))
            .language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create a Vertex AI model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_vertex_new_with_base(
    access_token: *const c_char,
    project: *const c_char,
    location: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (access_token, project, location, model_id) = parse_four_args(
            access_token,
            "access_token",
            project,
            "project",
            location,
            "location",
            model_id,
            "model_id",
        )?;
        let mut config = VertexProviderConfig::new(access_token, project, location);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let m = VertexProvider::new(config).language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create a Cohere model instance. AiMuxError: invalid model id.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_new(
    api_key: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let m = CohereProvider::new(CohereConfig::new(api_key)).language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create a Cohere model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let mut config = CohereConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let m = CohereProvider::new(config).language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create a Mistral model instance. AiMuxError: invalid model id.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_mistral_new(
    api_key: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let m = MistralProvider::new(MistralConfig::new(api_key)).language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create a Mistral model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_mistral_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let mut config = MistralConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let m = MistralProvider::new(config).language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create an xAI model instance. AiMuxError: invalid model id.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_xai_new(
    api_key: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let m = XAIProvider::new(XAIConfig::new(api_key)).language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create an xAI model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_xai_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let mut config = XAIConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let m = XAIProvider::new(config).language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Create a language model from the registry by provider name (RFC-0017 phase 4).
///
/// - `name` — registry provider name, e.g. `"groq"` / `"deepseek"`.
/// - `api_key` — may be NULL to read the provider's env var from the registry
///   entry (replaces the retired `aimux_deepseek_new` etc.).
/// - `model_id` — model id string.
/// - `config_json` — optional JSON object of `ProviderOptions`
///   (`{"base_url": "...", "headers": {...}, "max_retries": 0, "body_overrides": {...}}`);
///   NULL / empty / "null" for defaults.
///
/// AiMuxError: unknown provider, bad config shape, missing env key, or
/// invalid model id.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_new(
    name: *const c_char,
    api_key: *const c_char,
    model_id: *const c_char,
    config_json: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let name = str_arg(name, "name")?;
        let model_id = str_arg(model_id, "model_id")?;
        // NULL => the registry entry's env var; a non-NULL key must be UTF-8.
        let key = opt_str_arg(api_key, "api_key")?;
        let opts = parse_provider_options(config_json)?;
        let m = provider(&name, key, &model_id, opts)?;
        Ok(intern_model(Arc::from(m)))
    })
}

/// Convenience: create a language model by provider name, reading the API key
/// from the provider's env var.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_from_env(
    name: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let name = str_arg(name, "name")?;
        let model_id = str_arg(model_id, "model_id")?;
        let m = provider(&name, None, &model_id, None)?;
        Ok(intern_model(Arc::from(m)))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: provider handles (RFC-0027) — createProvider / listModels / model
// ─────────────────────────────────────────────────────────────────────────────

/// Create a **provider handle** (RFC-0027) for a registry-backed provider.
///
/// Unlike `aimux_provider_new` (which binds to a single model_id), this returns
/// a provider handle that supports `aimux_provider_list_models` (runtime
/// discovery) and `aimux_provider_model` (build a model from a discovered id).
///
/// `api_key = null` reads the provider's env var from the registry entry.
/// `config_json` is an optional `ProviderOptions` JSON string (same as
/// `aimux_provider_new`).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_handle_new(
    name: *const c_char,
    api_key: *const c_char,
    config_json: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let name = str_arg(name, "name")?;
        let key = opt_str_arg(api_key, "api_key")?;
        let opts = parse_provider_options(config_json)?;
        let p = provider_handle(&name, key, opts)?;
        Ok(intern_handle(HandleEntry::Provider(Arc::from(p))))
    })
}

/// List models on a provider handle (RFC-0027 runtime discovery).
///
/// `handle` is from `aimux_provider_handle_new`. Writes a JSON array of
/// sparse `RuntimeModel` (id / owned_by / created) — **no community
/// enrichment**. To supplement with model specs, call `aimux_get_model_specs`
/// separately and merge in the host.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_list_models(
    handle: u64,
    out_models_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_models_json, "out_models_json", || {
        let HandleEntry::Provider(p) = entry_of(handle, "provider")? else {
            return Err(FfiError::InvalidHandle {
                expected: "provider",
            }
            .into());
        };
        run_json(p.list_models())
    })
}

/// Build a language model from a provider handle + model_id (RFC-0027).
///
/// `handle` is from `aimux_provider_handle_new`. The new handle is a model
/// handle (same as `aimux_provider_new`), usable with `aimux_generate_text`
/// etc.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_model(
    handle: u64,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let HandleEntry::Provider(p) = entry_of(handle, "provider")? else {
            return Err(FfiError::InvalidHandle {
                expected: "provider",
            }
            .into());
        };
        let model_id = str_arg(model_id, "model_id")?;
        let m = p.language_model(&model_id)?;
        Ok(intern_model(Arc::from(m)))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: model specs (RFC-0027) — get_model_specs
// ─────────────────────────────────────────────────────────────────────────────

/// Fetch the community model catalogue (anya2a). Writes a JSON-serialized
/// `Catalogue` (provider → model_id → ModelSpec).
///
/// `source_url` is an optional URL override (null = default anya2a endpoint).
/// This is a **thin fetch** — no caching, no FS writes. The host decides how
/// to cache/persist the result.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_get_model_specs(
    source_url: *const c_char,
    out_specs_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_specs_json, "out_specs_json", || {
        let url = opt_str_arg(source_url, "source_url")?;
        run_json(aimux_providers::get_model_specs(url.as_deref()))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: non-streaming generation
// ─────────────────────────────────────────────────────────────────────────────

/// Non-streaming generation.
///
/// `prompt_json` is either a bare prompt value (`"text"` or a messages array)
/// or `{"prompt": <value>}`. `opts_json` is a serialized `GenerateTextOptions`
/// (empty / null for defaults). Writes the serialized `GenerateTextResult`
/// to `*out_json` (caller frees with [`aimux_free_string`]).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_generate_text(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let model = model_of(handle)?;
        let prompt = parse_prompt_arg(prompt_json)?;
        let opts = parse_opts_arg(opts_json)?;
        run_json(async move { generate_text(&*model, prompt, opts).await })
    })
}

/// Generate a structured JSON object from the model (M12, RFC-0016).
///
/// Same signature as [`aimux_generate_text`]; writes the serialized
/// `GenerateObjectResult`. The caller passes `response_format: { "Json": {
/// ... } }` via `opts_json` to control the schema; the function applies JSON
/// repair before parsing.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_generate_object(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let model = model_of(handle)?;
        let prompt = parse_prompt_arg(prompt_json)?;
        let opts = parse_opts_arg(opts_json)?;
        run_json(async move { generate_object(&*model, prompt, opts).await })
    })
}

/// Consume a stream to completion and write the aggregated result (M11,
/// RFC-0016). Synchronous — blocks until the stream finishes.
///
/// Same signature as [`aimux_generate_text`]; writes the serialized
/// `StreamTextResultAggregated`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_consume_stream_text(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let model = model_of(handle)?;
        let prompt = parse_prompt_arg(prompt_json)?;
        let opts = parse_opts_arg(opts_json)?;
        run_json(async move {
            let stream_result = stream_text(&*model, prompt, opts).await?;
            stream_result.consume().await
        })
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: streaming generation (push callbacks)
// ─────────────────────────────────────────────────────────────────────────────

/// Streaming generation with push callbacks.
///
/// Blocks the calling thread until the stream completes (synchronous +
/// callback mode). Callbacks are invoked in the same call stack:
/// - `on_part(json, stream_ctx)`: each `StreamPart` as JSON (valid only during
///   the call). `StreamPart::Error` is data on this path, not a terminal fail.
/// - `on_done(stream_ctx)`: once on normal completion (not on failure).
///
/// NULL on success (after `on_done`); a returned error otherwise (no
/// `on_done`): a NULL callback is `NullPointer`, a provider failure is the
/// model's, a part that cannot be serialized is `ResultSerialization`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_stream_text(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: Option<extern "C-unwind" fn(*const c_char, *mut c_void)>,
    on_done: Option<extern "C-unwind" fn(*mut c_void)>,
    stream_ctx: *mut c_void,
) -> *mut aimux_error_t {
    no_result(|| {
        stream_text_with_signal(
            handle,
            prompt_json,
            opts_json,
            on_part,
            on_done,
            stream_ctx,
            None,
        )
    })
}

/// Create a per-call abort signal for a cancelable FFI operation.
///
/// The caller must release the returned handle with
/// [`aimux_abort_signal_drop`].
#[unsafe(no_mangle)]
pub extern "C" fn aimux_abort_signal_new() -> u64 {
    intern_handle(HandleEntry::Abort(AbortSignal::new()))
}

/// Request cancellation for an abort-signal handle.
///
/// Invalid handles and repeated calls are no-ops.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_abort_signal_abort(handle: u64) {
    if let Some(signal) = get_abort_signal(handle) {
        signal.abort();
    }
}

/// Release an abort-signal handle.
///
/// This function does not remove model handles if the caller passes the wrong
/// handle type. Active calls keep their cloned signal alive.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_abort_signal_drop(handle: u64) {
    if handle != 0 {
        drop_abort_signal(handle);
    }
}

/// Stream text with a per-call abort signal.
///
/// This function blocks like [`aimux_stream_text`]. Another thread can call
/// [`aimux_abort_signal_abort`] with `abort_handle` to stop this call
/// (`AiMuxError::Aborted`, no `on_done`).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_stream_text_with_abort(
    handle: u64,
    abort_handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: Option<extern "C-unwind" fn(*const c_char, *mut c_void)>,
    on_done: Option<extern "C-unwind" fn(*mut c_void)>,
    stream_ctx: *mut c_void,
) -> *mut aimux_error_t {
    no_result(|| {
        let abort_signal = abort_of(abort_handle)?;
        stream_text_with_signal(
            handle,
            prompt_json,
            opts_json,
            on_part,
            on_done,
            stream_ctx,
            Some(abort_signal),
        )
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: OpenAI-compatible output (RFC-0026)
// ─────────────────────────────────────────────────────────────────────────────

/// Non-streaming text generation with OpenAI Chat Completions output.
///
/// Identical to [`aimux_generate_text`] except the written JSON string is a
/// serialized `ChatCompletion` (OpenAI `chat.completion` object) rather than a
/// `GenerateTextResult`. Works with any provider — the result is always
/// standard OpenAI format.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_generate_text_as_openai(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let model = model_of(handle)?;
        let prompt = parse_prompt_arg(prompt_json)?;
        let opts = parse_opts_arg(opts_json)?;
        run_json(async move { generate_text_as_openai(&*model, prompt, opts).await })
    })
}

/// Streaming text generation with OpenAI Chat Completions output.
///
/// Identical to [`aimux_stream_text`] except each `on_part` callback receives a
/// serialized `ChatCompletionChunk` (OpenAI `chat.completion.chunk` object)
/// rather than a `StreamPart`. Works with any provider.
///
/// `opts_json` may carry an extra `openai_stream_options` object with
/// `include_usage` (bool, default true) and `include_reasoning` (bool, default
/// true) fields to control the chunk output.
///
/// `StreamPart::Error` is mapped to a content delta + finish chunk; terminal
/// failures return a returned error (same polarity as [`aimux_stream_text`]).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_stream_text_as_openai(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: Option<extern "C-unwind" fn(*const c_char, *mut c_void)>,
    on_done: Option<extern "C-unwind" fn(*mut c_void)>,
    stream_ctx: *mut c_void,
) -> *mut aimux_error_t {
    no_result(|| {
        stream_text_as_openai_with_signal(
            handle,
            prompt_json,
            opts_json,
            on_part,
            on_done,
            stream_ctx,
            None,
        )
    })
}

/// Cancelable streaming OpenAI-compatible output (see
/// [`aimux_stream_text_with_abort`]).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_stream_text_as_openai_with_abort(
    handle: u64,
    abort_handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: Option<extern "C-unwind" fn(*const c_char, *mut c_void)>,
    on_done: Option<extern "C-unwind" fn(*mut c_void)>,
    stream_ctx: *mut c_void,
) -> *mut aimux_error_t {
    no_result(|| {
        let abort_signal = abort_of(abort_handle)?;
        stream_text_as_openai_with_signal(
            handle,
            prompt_json,
            opts_json,
            on_part,
            on_done,
            stream_ctx,
            Some(abort_signal),
        )
    })
}

/// Serialize a stream item for `on_part`, or [`FfiError::ResultSerialization`].
fn stream_part_cstring<T: serde::Serialize>(part: &T) -> Result<CString, FfiError> {
    let json = serde_json::to_string(part).map_err(|e| FfiError::ResultSerialization {
        message: format!("stream part: {e}"),
    })?;
    CString::new(json).map_err(|e| FfiError::ResultSerialization {
        message: format!("stream part contains NUL: {e}"),
    })
}

fn stream_text_as_openai_with_signal(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: Option<extern "C-unwind" fn(*const c_char, *mut c_void)>,
    on_done: Option<extern "C-unwind" fn(*mut c_void)>,
    stream_ctx: *mut c_void,
    abort_signal: Option<AbortSignal>,
) -> FfiResult<()> {
    // A NULL callback is this layer's finding, before anything else runs.
    let on_part = on_part.ok_or(FfiError::NullPointer {
        argument: "on_part",
    })?;
    let on_done = on_done.ok_or(FfiError::NullPointer {
        argument: "on_done",
    })?;
    let model = model_of(handle)?;
    let prompt = parse_prompt_arg(prompt_json)?;
    let mut opts = parse_opts_arg(opts_json)?;

    // Extract OpenAI stream options from the opts JSON, then remove them so
    // they don't confuse the provider (which doesn't know this field).
    let stream_options = opts
        .provider_options
        .as_ref()
        .and_then(|po| po.get("openai"))
        .and_then(|o| o.get("stream_options"))
        .cloned()
        .map(|v| OpenAiStreamOptions {
            include_usage: v
                .get("include_usage")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            include_reasoning: v
                .get("include_reasoning")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
        })
        .unwrap_or_default();

    opts.abort_signal = abort_signal.clone();
    // stream_ctx is only for C callbacks; not Send into the async task.
    let stream_ctx = stream_ctx as usize;

    ffi_block_on(async move {
        let sr = match abort_signal.as_ref() {
            Some(signal) => {
                tokio::select! {
                    biased;
                    _ = signal.cancelled() => Err(AiMuxError::Aborted),
                    result = stream_text_as_openai(&*model, prompt, opts, stream_options) => result,
                }
            }
            None => stream_text_as_openai(&*model, prompt, opts, stream_options).await,
        }?;
        let mut stream = sr.stream;
        loop {
            let next = match abort_signal.as_ref() {
                Some(signal) => {
                    tokio::select! {
                        biased;
                        _ = signal.cancelled() => return Err(AiMuxError::Aborted.into()),
                        item = stream.next() => item,
                    }
                }
                None => stream.next().await,
            };
            let Some(item) = next else {
                break;
            };
            // A chunk that cannot be serialized ends the stream with this
            // layer's ResultSerialization — never a silent "{}" placeholder.
            let cstr = stream_part_cstring(&item?)?;
            invoke_stream_callback("on_part", || {
                on_part(cstr.as_ptr(), stream_ctx as *mut c_void);
            })?;
        }
        invoke_stream_callback("on_done", || {
            on_done(stream_ctx as *mut c_void);
        })?;
        Ok::<(), AiMuxFfiError>(())
    })?
}

fn stream_text_with_signal(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: Option<extern "C-unwind" fn(*const c_char, *mut c_void)>,
    on_done: Option<extern "C-unwind" fn(*mut c_void)>,
    stream_ctx: *mut c_void,
    abort_signal: Option<AbortSignal>,
) -> FfiResult<()> {
    // A NULL callback is this layer's finding, before anything else runs.
    let on_part = on_part.ok_or(FfiError::NullPointer {
        argument: "on_part",
    })?;
    let on_done = on_done.ok_or(FfiError::NullPointer {
        argument: "on_done",
    })?;
    let model = model_of(handle)?;
    let prompt = parse_prompt_arg(prompt_json)?;
    let mut opts = parse_opts_arg(opts_json)?;
    opts.abort_signal = abort_signal.clone();
    let stream_ctx = stream_ctx as usize;

    ffi_block_on(async move {
        let sr = match abort_signal.as_ref() {
            Some(signal) => {
                tokio::select! {
                    biased;
                    _ = signal.cancelled() => Err(AiMuxError::Aborted),
                    result = stream_text(&*model, prompt, opts) => result,
                }
            }
            None => stream_text(&*model, prompt, opts).await,
        }?;
        let mut stream = sr.stream;
        loop {
            let next = match abort_signal.as_ref() {
                Some(signal) => {
                    tokio::select! {
                        biased;
                        _ = signal.cancelled() => return Err(AiMuxError::Aborted.into()),
                        item = stream.next() => item,
                    }
                }
                None => stream.next().await,
            };
            let Some(item) = next else {
                break;
            };
            // A part that cannot be serialized ends the stream with this
            // layer's ResultSerialization — never a silent "{}" placeholder.
            let cstr = stream_part_cstring(&item?)?;
            invoke_stream_callback("on_part", || {
                on_part(cstr.as_ptr(), stream_ctx as *mut c_void);
            })?;
        }
        invoke_stream_callback("on_done", || {
            on_done(stream_ctx as *mut c_void);
        })?;
        Ok::<(), AiMuxFfiError>(())
    })?
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: resource management
// ─────────────────────────────────────────────────────────────────────────────

/// Release a handle previously returned by `aimux_*_new` (any handle type — a
/// transcription session's driver task is aborted and joined as well, so
/// `aimux_transcription_session_drop` is only a clearer name for the same
/// thing).
///
/// Safe to call with `0` or an unknown handle (no-op).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_drop_handle(handle: u64) {
    if handle != 0 {
        drop_handle(handle);
    }
}

/// Free a C string previously returned by any aimux function that hands out
/// `*mut c_char` — result JSON, error getters.
///
/// # Safety
///
/// `ptr` must be null or a pointer previously produced by an aimux `char*`
/// return (i.e. via `CString::into_raw`). Passing any other pointer is
/// undefined behavior.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aimux_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: caller guarantees `ptr` came from `CString::into_raw`.
    drop(unsafe { CString::from_raw(ptr) });
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Embedding
// ─────────────────────────────────────────────────────────────────────────────

/// Create an OpenAI embedding model instance.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_embedding_new(
    api_key: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).embedding_model(&model_id);
        Ok(intern_handle(HandleEntry::Embedding(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_embedding_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let mut config = OpenAIConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let model = OpenAIProvider::new(config).embedding_model(&model_id);
        Ok(intern_handle(HandleEntry::Embedding(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_embedding_new(
    api_key: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let model = CohereProvider::new(CohereConfig::new(api_key)).embedding_model(&model_id);
        Ok(intern_handle(HandleEntry::Embedding(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_embedding_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let mut config = CohereConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let model = CohereProvider::new(config).embedding_model(&model_id);
        Ok(intern_handle(HandleEntry::Embedding(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_embedding_new(
    api_key: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let model = GoogleProvider::new(GoogleConfig::new(api_key)).embedding_model(&model_id);
        Ok(intern_handle(HandleEntry::Embedding(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_embedding_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let mut config = GoogleConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let model = GoogleProvider::new(config).embedding_model(&model_id);
        Ok(intern_handle(HandleEntry::Embedding(Arc::new(model))))
    })
}

/// Generate embeddings. `values_json` is a JSON array of strings.
/// Writes `EmbeddingResult` JSON (caller frees).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_embed(
    handle: u64,
    values_json: *const c_char,
    opts_json: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let HandleEntry::Embedding(model) = entry_of(handle, "embedding")? else {
            return Err(FfiError::InvalidHandle {
                expected: "embedding",
            }
            .into());
        };
        let values_json = str_arg(values_json, "values_json")?;
        let mut opts = aimux_core::embedding_model::EmbeddingCallOptions::new("");
        if let Some(s) = opt_str_arg(opts_json, "opts_json")?
            && !s.trim().is_empty()
            && s.trim() != "null"
        {
            opts = serde_json::from_str(&s).map_err(|e| wire_err("opts_json", e))?;
        }
        opts.values = serde_json::from_str(&values_json).map_err(|e| wire_err("values_json", e))?;
        run_json(async move { model.do_embed(&opts).await })
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Speech (TTS)
// ─────────────────────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_speech_new(
    api_key: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).speech(&model_id);
        Ok(intern_handle(HandleEntry::Speech(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_speech_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let mut config = OpenAIConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let model = OpenAIProvider::new(config).speech(&model_id);
        Ok(intern_handle(HandleEntry::Speech(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_speech_generate(
    handle: u64,
    opts_json: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let HandleEntry::Speech(model) = entry_of(handle, "speech")? else {
            return Err(FfiError::InvalidHandle { expected: "speech" }.into());
        };
        let opts: aimux_core::speech_model::SpeechCallOptions =
            parse_json_arg(opts_json, "opts_json")?;
        run_json(async move { model.do_generate(&opts).await })
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Image
// ─────────────────────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_image_new(
    api_key: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).image(&model_id);
        Ok(intern_handle(HandleEntry::Image(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_image_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let mut config = OpenAIConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let model = OpenAIProvider::new(config).image(&model_id);
        Ok(intern_handle(HandleEntry::Image(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_image_new(
    api_key: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let model = GoogleProvider::new(GoogleConfig::new(api_key)).image(&model_id);
        Ok(intern_handle(HandleEntry::Image(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_image_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let mut config = GoogleConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let model = GoogleProvider::new(config).image(&model_id);
        Ok(intern_handle(HandleEntry::Image(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_image_generate(
    handle: u64,
    opts_json: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let HandleEntry::Image(model) = entry_of(handle, "image")? else {
            return Err(FfiError::InvalidHandle { expected: "image" }.into());
        };
        let opts: aimux_core::image_model::ImageCallOptions =
            parse_json_arg(opts_json, "opts_json")?;
        run_json(async move { model.do_generate(&opts).await })
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Transcription (non-streaming)
// ─────────────────────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_transcription_new(
    api_key: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).transcription(&model_id);
        Ok(intern_handle(HandleEntry::Transcription(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_transcription_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let mut config = OpenAIConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let model = OpenAIProvider::new(config).transcription(&model_id);
        Ok(intern_handle(HandleEntry::Transcription(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_transcription_generate(
    handle: u64,
    audio_base64: *const c_char,
    media_type: *const c_char,
    _opts_json: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let HandleEntry::Transcription(model) = entry_of(handle, "transcription")? else {
            return Err(FfiError::InvalidHandle {
                expected: "transcription",
            }
            .into());
        };
        let audio_base64 = str_arg(audio_base64, "audio_base64")?;
        let media_type = str_arg(media_type, "media_type")?;
        let opts = aimux_core::transcription_model::TranscriptionCallOptions::new(
            aimux_core::transcription_model::AudioInput::Base64(audio_base64),
            media_type,
        );
        run_json(async move { model.do_generate(&opts).await })
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: transcription streaming sessions (RFC-0028 Phase 2)
// ─────────────────────────────────────────────────────────────────────────────

/// JSON options for `aimux_transcription_session_new` (all fields optional):
/// `{ "input_audio_format": { "format_type": "audio/pcm", "rate": 24000 },
///    "provider_options": { … }, "headers": { … }, "include_raw_chunks": false }`.
#[derive(serde::Deserialize, Default)]
struct TranscriptionSessionFfiOptions {
    input_audio_format: Option<aimux_core::transcription_model::InputAudioFormat>,
    provider_options: Option<HashMap<String, serde_json::Value>>,
    headers: Option<HashMap<String, String>>,
    include_raw_chunks: Option<bool>,
    timeout: Option<aimux_core::options::TimeoutConfiguration>,
}

/// Look up a transcription streaming session handle.
fn session_of(
    handle: u64,
) -> Result<Arc<transcription_session::TranscriptionFfiSession>, FfiError> {
    match entry_of(handle, "transcription session")? {
        HandleEntry::TranscriptionSession(s) => Ok(s),
        _ => Err(FfiError::InvalidHandle {
            expected: "transcription session",
        }),
    }
}

/// Start a transcription streaming session (RFC-0028 Phase 2). Internally
/// spawns the driver task immediately; audio is then pushed with
/// `aimux_transcription_push_audio` and parts pulled with
/// `aimux_transcription_next_part`.
///
/// `model_handle` must be a transcription model handle that supports
/// streaming (e.g. OpenAI `gpt-realtime-whisper`); models without `do_stream`
/// fail on the first `next_part` (the connect/establishment error surfaces
/// there). `abort_handle` may be 0 (no cancellation) or an
/// `aimux_abort_signal_new` handle — firing it aborts the session.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_transcription_session_new(
    model_handle: u64,
    abort_handle: u64,
    opts_json: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let HandleEntry::Transcription(model) = entry_of(model_handle, "transcription")? else {
            return Err(FfiError::InvalidHandle {
                expected: "transcription",
            }
            .into());
        };
        let abort = if abort_handle == 0 {
            None
        } else {
            Some(abort_of(abort_handle)?)
        };
        // NULL / empty / "null" = defaults (shared convention).
        let opts_json = normalize_config_json(opts_json, "opts_json")?;
        let ffi_opts: TranscriptionSessionFfiOptions =
            serde_json::from_str(&opts_json).map_err(|e| wire_err("opts_json", e))?;
        let session = transcription_session::TranscriptionFfiSession::spawn(
            model,
            transcription_session::SessionOptions {
                input_audio_format: ffi_opts.input_audio_format,
                provider_options: ffi_opts.provider_options,
                headers: ffi_opts.headers,
                include_raw_chunks: ffi_opts.include_raw_chunks.unwrap_or(false),
                timeout: ffi_opts.timeout,
            },
            abort,
        );
        Ok(intern_handle(HandleEntry::TranscriptionSession(session)))
    })
}

/// Push one binary audio chunk into the session. **Blocking**: waits while
/// the internal channel is full (backpressure propagation; the caller's
/// capture loop throttles). Must not be called from within an aimux callback
/// (re-entrancy guard rejects it).
///
/// `data` may be NULL only when `len == 0` (no-op). Failures: session ended /
/// input already finished → AiMuxError; NULL data with `len > 0` →
/// `NullPointer`; dead session → `InvalidHandle`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_transcription_push_audio(
    session: u64,
    data: *const u8,
    len: usize,
) -> *mut aimux_error_t {
    no_result(|| {
        let session = session_of(session)?;
        if data.is_null() {
            if len == 0 {
                return Ok(()); // no-op
            }
            return Err(FfiError::NullPointer { argument: "data" }.into());
        }
        // SAFETY: caller guarantees `data` points to `len` valid bytes; the copy
        // happens synchronously before any await.
        let bytes = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
        ffi_block_on(session.push_audio(bytes))??;
        Ok(())
    })
}

/// Signal end-of-audio (the audio stream ends; the provider flushes). Safe to
/// call multiple times. Fails only for a dead session handle.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_transcription_input_done(session: u64) -> *mut aimux_error_t {
    no_result(|| {
        session_of(session)?.input_done();
        Ok(())
    })
}

/// Pull the next transcription part (JSON-serialized `TranscriptionStreamPart`;
/// free with `aimux_free_string`).
///
/// `timeout_ms`: >0 = wait at most that long; 0 = immediate poll; <0 = wait
/// indefinitely. Both out-params are required. On NULL return `*out_state`
/// says what happened:
/// - `AIMUX_TRANSCRIPTION_NEXT_PART_PART` — `*out_part` holds a part;
/// - `AIMUX_TRANSCRIPTION_NEXT_PART_ENDED` — the stream ended normally (a
///   `Finish` part was delivered earlier); `*out_part` is NULL;
/// - `AIMUX_TRANSCRIPTION_NEXT_PART_TIMEOUT` — no part in time; the session
///   is still live, call again; `*out_part` is NULL.
///
/// A non-NULL return is a failure (abort / API error / dead handle);
/// `*out_part` is NULL and `*out_state` is unspecified.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_transcription_next_part(
    session: u64,
    timeout_ms: i64,
    out_part: *mut *mut c_char,
    out_state: *mut i32,
) -> *mut aimux_error_t {
    if out_part.is_null() {
        return no_result(|| {
            Err(FfiError::NullPointer {
                argument: "out_part",
            }
            .into())
        });
    }
    // Sentinel first, so a failure below (including a NULL out_state) never
    // leaves a stale pointer in *out_part.
    unsafe { *out_part = std::ptr::null_mut() };
    if out_state.is_null() {
        return no_result(|| {
            Err(FfiError::NullPointer {
                argument: "out_state",
            }
            .into())
        });
    }
    let r: FfiResult<(i32, Option<String>)> = (|| {
        let session = session_of(session)?;
        let timeout = if timeout_ms < 0 {
            None
        } else {
            Some(std::time::Duration::from_millis(timeout_ms as u64))
        };
        Ok(match ffi_block_on(session.next_part(timeout))? {
            // A part arrived.
            Ok(Some(Ok(part))) => (AIMUX_TRANSCRIPTION_NEXT_PART_PART, Some(to_json(&part)?)),
            // Channel closed: normal end.
            Ok(None) => (AIMUX_TRANSCRIPTION_NEXT_PART_ENDED, None),
            // No part within `timeout`; the session is still live.
            Err(AiMuxError::Timeout(_)) => (AIMUX_TRANSCRIPTION_NEXT_PART_TIMEOUT, None),
            // The part stream itself errored (abort / API error) — final error.
            Ok(Some(Err(e))) | Err(e) => return Err(e.into()),
        })
    })();
    finish(r, |(state, part)| unsafe {
        *out_state = state;
        if let Some(p) = part {
            *out_part = into_cstring_raw(p);
        }
    })
}

/// Terminate a transcription session and release it: aborts the driver,
/// joins (bounded), and drops the handle. Safe with 0 or an unknown handle.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_transcription_session_drop(session: u64) {
    // Remove from the registry FIRST (the join below must not hold the
    // registry mutex).
    let removed = registry()
        .lock()
        .expect("aimux-ffi: registry mutex poisoned")
        .remove(&session);
    if let Some(HandleEntry::TranscriptionSession(s)) = removed {
        s.terminate();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Files
// ─────────────────────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_files_new(
    api_key: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let api_key = str_arg(api_key, "api_key")?;
        let files = OpenAIProvider::new(OpenAIConfig::new(api_key)).files();
        Ok(intern_handle(HandleEntry::Files(Arc::new(files))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_files_new_with_base(
    api_key: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let api_key = str_arg(api_key, "api_key")?;
        let mut config = OpenAIConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let files = OpenAIProvider::new(config).files();
        Ok(intern_handle(HandleEntry::Files(Arc::new(files))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_file_upload(
    handle: u64,
    data_base64: *const c_char,
    media_type: *const c_char,
    _opts_json: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let HandleEntry::Files(model) = entry_of(handle, "files")? else {
            return Err(FfiError::InvalidHandle { expected: "files" }.into());
        };
        let data_base64 = str_arg(data_base64, "data_base64")?;
        let media_type = str_arg(media_type, "media_type")?;
        let opts = aimux_core::files_model::UploadFileCallOptions::new(
            aimux_core::files_model::UploadFileData::Data {
                data: aimux_core::shared::FileBytes::Base64(data_base64),
            },
            media_type,
        );
        run_json(async move { model.upload_file(&opts).await })
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Reranking
// ─────────────────────────────────────────────────────────────────────────────

/// Create a Cohere reranking model instance.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_reranking_new(
    api_key: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let model = CohereProvider::new(CohereConfig::new(api_key)).reranking_model(&model_id);
        Ok(intern_handle(HandleEntry::Reranking(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_reranking_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let mut config = CohereConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let model = CohereProvider::new(config).reranking_model(&model_id);
        Ok(intern_handle(HandleEntry::Reranking(Arc::new(model))))
    })
}

/// Rerank documents. `opts_json` is JSON-serialized `RerankingCallOptions`
/// (must contain `query` and `documents`). Writes `RerankingResult` JSON.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_rerank(
    handle: u64,
    opts_json: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let HandleEntry::Reranking(model) = entry_of(handle, "reranking")? else {
            return Err(FfiError::InvalidHandle {
                expected: "reranking",
            }
            .into());
        };
        let opts: aimux_core::reranking_model::RerankingCallOptions =
            parse_json_arg(opts_json, "opts_json")?;
        run_json(async move { model.do_rerank(&opts).await })
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Video
// ─────────────────────────────────────────────────────────────────────────────

/// Create a Google video model instance.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_video_new(
    api_key: *const c_char,
    model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let model = GoogleProvider::new(GoogleConfig::new(api_key)).video(&model_id);
        Ok(intern_handle(HandleEntry::Video(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_video_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let (api_key, model_id) = parse_two_args(api_key, "api_key", model_id, "model_id")?;
        let mut config = GoogleConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let model = GoogleProvider::new(config).video(&model_id);
        Ok(intern_handle(HandleEntry::Video(Arc::new(model))))
    })
}

/// Generate video. `opts_json` is JSON-serialized `VideoCallOptions`
/// (must contain `prompt`). Writes `VideoResult` JSON.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_video_generate(
    handle: u64,
    opts_json: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let HandleEntry::Video(model) = entry_of(handle, "video")? else {
            return Err(FfiError::InvalidHandle { expected: "video" }.into());
        };
        let opts: aimux_core::video_model::VideoCallOptions =
            parse_json_arg(opts_json, "opts_json")?;
        run_json(async move { model.do_generate(&opts).await })
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Search
// ─────────────────────────────────────────────────────────────────────────────

/// Create a Tavily search model instance. `model_id` is accepted for API
/// symmetry but ignored (Tavily uses a fixed endpoint).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_tavily_search_new(
    api_key: *const c_char,
    _model_id: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let api_key = str_arg(api_key, "api_key")?;
        let model = TavilyProvider::new(TavilyConfig::new(api_key)).search_model();
        Ok(intern_handle(HandleEntry::Search(Arc::new(model))))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_tavily_search_new_with_base(
    api_key: *const c_char,
    _model_id: *const c_char,
    base_url: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let api_key = str_arg(api_key, "api_key")?;
        let mut config = TavilyConfig::new(api_key);
        if let Some(url) = parse_base_url(base_url)? {
            config = config.with_base_url(url);
        }
        let model = TavilyProvider::new(config).search_model();
        Ok(intern_handle(HandleEntry::Search(Arc::new(model))))
    })
}

/// Execute a search. `opts_json` is JSON-serialized `SearchCallOptions`
/// (must contain `query`). Writes `SearchResult` JSON.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_search(
    handle: u64,
    opts_json: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let HandleEntry::Search(model) = entry_of(handle, "search")? else {
            return Err(FfiError::InvalidHandle { expected: "search" }.into());
        };
        let opts: aimux_core::search_model::SearchCallOptions =
            parse_json_arg(opts_json, "opts_json")?;
        run_json(async move { model.do_search(&opts).await })
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Codex subscription helper (RFC-0018 §3.2)
// ─────────────────────────────────────────────────────────────────────────────

/// Refresh a Codex subscription access token (RFC-0018 §3.2).
///
/// Stateless: performs one OAuth `refresh_token` grant against
/// `auth.openai.com/oauth/token`. Writes
/// `{"access_token","refresh_token","expires_in_secs"}` JSON on success
/// (caller frees with `aimux_free_string`). The caller owns token persistence
/// and the 401 → refresh → retry orchestration — the library never stores
/// credentials.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_codex_refresh(
    refresh_token: *const c_char,
    client_id: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let (refresh_token, client_id) =
            parse_two_args(refresh_token, "refresh_token", client_id, "client_id")?;
        run_json(async move { aimux_providers::codex_refresh(&refresh_token, &client_id).await })
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Logging (RFC-0014)
// ─────────────────────────────────────────────────────────────────────────────

/// Initialize the global logger. Idempotent — safe to call any number of
/// times from any thread; only the first call has an effect.
///
/// If the host already registered its own `tracing` subscriber, this is a
/// no-op (aimux never overrides a consumer's logger).
///
/// @param level NUL-terminated level string: "off" / "error" / "warn" /
///              "info" / "debug" / "trace". NULL falls back to the default
///              ("warn"); a non-UTF-8 pointer is `InvalidUtf8`. `AIMUX_LOG`
///              (RUST_LOG-style) and `AIMUX_LOG_LEVEL` env vars take
///              precedence when set. Logs go to stderr.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_init_logging(level: *const c_char) -> *mut aimux_error_t {
    no_result(|| {
        let level = opt_str_arg(level, "level")?;
        aimux_providers::init_logging(level.as_deref().unwrap_or("warn"));
        Ok(())
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: session grouping (RFC-0024)
// ─────────────────────────────────────────────────────────────────────────────

/// Register the global session store (RFC-0024). Replaces any previous one.
/// Until called, calls are not grouped and the session query functions return
/// empty results.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_session_store_init() {
    aimux_core::session::init_session_store(std::sync::Arc::new(
        aimux_core::session::SessionStore::new(),
    ));
}

/// Enable/disable the global session inferer (RFC-0024, opt-in, off by
/// default). Explicit `session_id` values always win regardless.
/// `enabled` nonzero = on.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_session_infer_init(enabled: i32) {
    aimux_core::session::init_session_infer(enabled != 0);
}

/// Query: all calls of a session, ordered by step (RFC-0024).
///
/// Writes a serialized `SessionCall[]` (empty array if the session is unknown
/// or no store is registered); the caller frees it with [`aimux_free_string`].
#[unsafe(no_mangle)]
pub extern "C" fn aimux_session_calls(
    session_id: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let id = str_arg(session_id, "session_id")?;
        to_json(&aimux_core::session::session_calls(&id))
    })
}

/// Query: all known sessions (RFC-0024). Writes a serialized `SessionView[]`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_list_sessions(out_json: *mut *mut c_char) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        to_json(&aimux_core::session::list_sessions())
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: cache probing (RFC-0015)
// ─────────────────────────────────────────────────────────────────────────────

static TRACE_STORES: OnceLock<Mutex<HashMap<u64, Arc<RingTraceStore>>>> = OnceLock::new();

fn trace_stores() -> &'static Mutex<HashMap<u64, Arc<RingTraceStore>>> {
    TRACE_STORES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn trace_store_of(handle: u64) -> Result<Arc<RingTraceStore>, FfiError> {
    trace_stores()
        .lock()
        .expect("aimux-ffi: trace registry mutex poisoned")
        .get(&handle)
        .cloned()
        .ok_or(FfiError::InvalidHandle { expected: "trace" })
}

fn trace_wrap(handle: u64, audited: Option<bool>) -> FfiResult<u64> {
    let model = model_of(handle)?;
    let store = Arc::new(RingTraceStore::new());
    let mut layer = TraceLayer::new(model, store.clone());
    if let Some(strict) = audited {
        layer = layer.with_rules_auditor(strict);
    }
    let new_handle = intern_model(Arc::new(layer));
    trace_stores()
        .lock()
        .expect("aimux-ffi: trace registry mutex poisoned")
        .insert(new_handle, store);
    Ok(new_handle)
}

/// Wrap a model handle in a probe layer (RFC-0015). The new handle can be
/// used with `aimux_generate_text` / `aimux_stream_text` (probed) and with
/// the `aimux_trace_*` query functions; release with `aimux_drop_handle`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_new(handle: u64, out_handle: *mut u64) -> *mut aimux_error_t {
    with_out_handle(out_handle, || trace_wrap(handle, None))
}

/// Wrap a model handle in a probe layer WITH the built-in rules auditor
/// (RFC-0015 §4). `strict` nonzero = strict mode (self-hosted single
/// instance); zero = shared mode (safe default). Release with
/// `aimux_drop_handle`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_new_audited(
    handle: u64,
    strict: i32,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || trace_wrap(handle, Some(strict != 0)))
}

/// Query: aggregated probe statistics, filtered by `filter_json` (a serialized
/// `TraceFilter`; pass `"{}"` for all rows — NULL is `NullPointer`). Writes
/// JSON `TraceStats[]`; caller frees with `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_aggregate(
    handle: u64,
    filter_json: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let store = trace_store_of(handle)?;
        let filter = parse_json_arg::<TraceFilter>(filter_json, "filter_json")?;
        to_json(&store.aggregate(&filter))
    })
}

/// Query: one session's chain view. Writes JSON `SessionChainView`; caller
/// frees with `aimux_free_string`. An unknown `session_id` is a lookup miss
/// on a caller-supplied value and is reported as
/// `AiMuxError::InvalidArgument` ("unknown session"), not as an invalid C
/// handle — the id is a string key, not a registry handle.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_session_chain(
    handle: u64,
    session_id: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let store = trace_store_of(handle)?;
        let id = str_arg(session_id, "session_id")?;
        let view = store
            .session_chain(&id)
            .ok_or_else(|| AiMuxError::InvalidArgument("unknown session".into()))?;
        to_json(&view)
    })
}

/// Query: one session's per-step cache-hit trajectory (RFC-0024 §4.3).
/// Writes a JSON array of `SessionStepStat` (empty for unknown sessions);
/// caller frees with `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_session_trajectory(
    handle: u64,
    session_id: *const c_char,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_json, "out_json", || {
        let store = trace_store_of(handle)?;
        let id = str_arg(session_id, "session_id")?;
        to_json(&store.session_cache_trajectory(&id))
    })
}

/// Export all probe records as JSONL (one `TraceRecord` per line). Writes a
/// string with embedded newlines; caller frees with `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_export_jsonl(
    handle: u64,
    out_jsonl: *mut *mut c_char,
) -> *mut aimux_error_t {
    with_out_string(out_jsonl, "out_jsonl", || {
        let store = trace_store_of(handle)?;
        let mut buf = Vec::new();
        store
            .export_jsonl(&mut buf)
            .map_err(|e| FfiError::ResultSerialization {
                message: format!("export: {e}"),
            })?;
        String::from_utf8(buf).map_err(|e| {
            FfiError::ResultSerialization {
                message: format!("utf8: {e}"),
            }
            .into()
        })
    })
}

/// Clear all probe records of a trace handle. Fails only for a dead handle.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_clear(handle: u64) -> *mut aimux_error_t {
    no_result(|| {
        trace_store_of(handle)?.clear();
        Ok(())
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: recording + mock replay (RFC-0023)
// ─────────────────────────────────────────────────────────────────────────────

/// Start recording (RFC-0023 P1/P2): writes complete `Recording` JSONL to
/// `{dir}/recordings.jsonl` (dir auto-created). Recording is **opt-in**.
/// Calling again with a different dir replaces the recorder.
///
/// A null / non-UTF-8 `dir` is a C ABI failure; recorder construction
/// failures carry the recording view (`AIMUX_E_RECORDING_INIT`,
/// `AIMUX_E_RECORDING_OPEN_FILE`, or `AIMUX_E_RECORDING_SPAWN`). On failure
/// the previous recorder, if any, is left in place.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_init_recording(dir: *const c_char) -> *mut aimux_error_t {
    no_result(|| {
        let dir = str_arg(dir, "dir")?;
        let rec = aimux_core::recording::JsonlRecorder::try_new(dir)?;
        aimux_core::recording::init_recording(Some(std::sync::Arc::new(rec)));
        Ok(())
    })
}

/// Start in-memory bounded recording (RFC-0023 P6): `RingRecorder` with `cap`
/// entries, FIFO eviction, dropped-count queryable. `cap == 0` is
/// `AiMuxError::InvalidArgument`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_init_recording_ring(cap: u64) -> *mut aimux_error_t {
    no_result(|| {
        if cap == 0 {
            return Err(AiMuxError::InvalidArgument("cap: must be > 0".into()).into());
        }
        aimux_core::recording::init_recording(Some(std::sync::Arc::new(
            aimux_core::recording::RingRecorder::with_capacity(cap as usize),
        )));
        Ok(())
    })
}

/// No-argument variant of [`aimux_init_recording_ring`]: initialize the global
/// recorder with a `RingRecorder` at the library default capacity (2048
/// entries, [`aimux_core::recording::RingRecorder::default`]). Ordinary callers
/// should prefer this entry point and leave the ring size to the library; pass
/// an explicit `cap` via [`aimux_init_recording_ring`] only when a different
/// size is required.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_init_recording_ring_default() {
    aimux_core::recording::init_recording(Some(std::sync::Arc::new(
        aimux_core::recording::RingRecorder::default(),
    )));
}

/// Stop recording: global recorder = None (new calls are unrecorded).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_recording_stop() {
    aimux_core::recording::init_recording(None);
}

/// Flush the global recorder (blocks until JSONL is on disk; no-op for the
/// ring recorder). Write failures are not reported here — see
/// [`aimux_recording_try_flush`].
#[unsafe(no_mangle)]
pub extern "C" fn aimux_recording_flush() {
    if let Some(rec) = aimux_core::recording::recorder() {
        rec.flush();
    }
}

/// Flush the global recorder and **report write failures** (see #136).
///
/// Unlike [`aimux_recording_flush`], this surface makes the recorder's
/// durability observable across the C ABI: NULL when the data is confirmed
/// on disk (also when recording was never initialized: nothing to flush), an
/// returned error with the recording view otherwise. Codes reachable from a
/// flush:
/// - `AIMUX_E_RECORDING_WRITE` — a prior write failed (e.g. ENOSPC); the
///   first error is sticky and every later flush keeps reporting it.
/// - `AIMUX_E_RECORDING_WRITER_GONE` — the writer thread is unavailable.
/// - `AIMUX_E_RECORDING_FLUSH_TIMEOUT` — no writer ack within 30s.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_recording_try_flush() -> *mut aimux_error_t {
    no_result(|| {
        if let Some(rec) = aimux_core::recording::recorder() {
            rec.try_flush()?;
        }
        Ok(())
    })
}

/// Register external OpenAI-compatible providers from a JSON config string
/// (RFC-0020). `config_json` is `{ "providers": [ { "name": ..., "base_url":
/// ..., ... }, ... ] }`. Entries override same-named built-ins or add new
/// ones. Malformed JSON text is `InvalidWireJson`; a well-formed document the
/// registry rejects is `AiMuxError::InvalidArgument`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_register_providers(config_json: *const c_char) -> *mut aimux_error_t {
    no_result(|| {
        let json = str_arg(config_json, "config_json")?;
        // Malformed JSON text is this layer's finding; a well-formed document
        // that the registry rejects (bad schema, unknown protocol) is an AiMuxError.
        serde_json::from_str::<serde_json::Value>(&json).map_err(|e| wire_err("config_json", e))?;
        aimux_providers::load_providers_from_json(&json).map_err(|e| match e {
            // The registry reports a schema mismatch as `JsonParse`; the text
            // already parsed above, so what it rejected is the shape —
            // `AiMuxError::InvalidArgument`, not a provider-response parse failure.
            AiMuxError::JsonParse(m) => AiMuxError::InvalidArgument(format!("config_json: {m}")),
            e => e,
        })?;
        Ok(())
    })
}

/// Set the global proxy configuration (M6, RFC-0016). Must be called before
/// the first `aimux_generate_text` / `aimux_stream_text` call; a no-op if the
/// shared HTTP client is already initialised.
///
/// `config_json` is a serialized `ProxyConfig`:
/// `{ "http_url": "...", "https_url": "...", "all_url": "...", "no_proxy":
/// "..." }` (all fields optional; omitting all is equivalent to relying on the
/// `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY` env vars).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_init_proxy(config_json: *const c_char) -> *mut aimux_error_t {
    no_result(|| {
        let config: aimux_provider_utils::ProxyConfig = parse_json_arg(config_json, "config_json")?;
        // `init_proxy` returns false when the shared client is already up; treat
        // that as success (idempotent) so callers don't need to reason about
        // ordering races.
        let _ = aimux_provider_utils::init_proxy(config);
        Ok(())
    })
}

/// Create a mock replay model from recorded JSONL (RFC-0023 P3). `recordings`
/// is one `Recording` JSON per line. The handle works with
/// `aimux_generate_text` / `aimux_stream_text` (no real API is sent).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_mock_replay_new(
    recordings_jsonl: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let recordings_jsonl = str_arg(recordings_jsonl, "recordings_jsonl")?;
        let mut recordings: Vec<aimux_core::recording::Recording> = Vec::new();
        for (idx, line) in recordings_jsonl.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            recordings.push(serde_json::from_str(line).map_err(|e| {
                wire_failure("recordings_jsonl", &e, format!("line {}: {e}", idx + 1))
            })?);
        }
        if recordings.is_empty() {
            return Err(
                AiMuxError::InvalidArgument("recordings_jsonl: no recordings".into()).into(),
            );
        }
        let model = aimux_core::replay::MockReplayModel::new(
            recordings[0].provider.provider.clone(),
            recordings[0].provider.model_id.clone(),
            recordings,
        );
        Ok(intern_model(Arc::new(model)))
    })
}

/// Resolve an array of model handles for a composite (router / moa).
///
/// `NULL` with `len == 0` is an empty list; `NULL` with `len > 0` is
/// `NullPointer`; any element that is not a live model handle is
/// `InvalidHandle { expected }` — a dead child is a caller bug, never
/// silently dropped (the composite would otherwise run with fewer members
/// than the caller believes).
fn model_handles(
    handles: *const u64,
    len: usize,
    argument: &'static str,
    expected: &'static str,
) -> FfiResult<Vec<Arc<dyn aimux_core::LanguageModel>>> {
    let slice: &[u64] = if handles.is_null() {
        if len != 0 {
            return Err(FfiError::NullPointer { argument }.into());
        }
        &[]
    } else {
        // SAFETY: caller guarantees `handles` points to `len` u64s.
        unsafe { std::slice::from_raw_parts(handles, len) }
    };
    slice
        .iter()
        .map(|&h| get_model(h).ok_or_else(|| FfiError::InvalidHandle { expected }.into()))
        .collect()
}

/// Create a `RouterModel` (RFC-0021) over the given child-model handles. The
/// new handle is itself a model handle (works with `aimux_generate_text` /
/// `aimux_stream_text`); free it with `aimux_drop_handle`.
///
/// `handles` is an array of `len` existing model handles (e.g. from
/// `aimux_openai_new`). `config_json` selects the router + fallback policy:
/// `{ "router": "rule" | "weighted", "weights": [..], "fallback": "on_error" |
/// "none", "provider_name": "router", "model_id": "router" }`. All keys are
/// optional; defaults are `rule` / `on_error` / `"router"` / `"router"`.
///
/// Fails on: NULL `handles` with `len > 0`, bad JSON, zero-length `handles`,
/// or any dead child handle (nothing is silently dropped).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_router_new(
    handles: *const u64,
    len: usize,
    config_json: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let models = model_handles(handles, len, "handles", "router child")?;
        if models.is_empty() {
            // No children at all: nothing to route to.
            return Err(FfiError::InvalidHandle {
                expected: "router child",
            }
            .into());
        }
        // NULL / empty / "null" all mean "defaults" (matching parse_provider_options
        // convention). Invalid UTF-8 → FFI `InvalidUtf8`.
        let config_json = normalize_config_json(config_json, "config_json")?;
        let cfg: RouterFfiConfig =
            serde_json::from_str(&config_json).map_err(|e| wire_err("config_json", e))?;
        let router: Box<dyn aimux_core::router::Router> = match cfg.router.as_deref() {
            Some("weighted") => {
                let weights = cfg.weights.unwrap_or_else(|| vec![1.0; models.len()]);
                Box::new(aimux_core::router::WeightedRouter::new(weights))
            }
            // "rule", None, and any unknown value fall back to RuleRouter (safest
            // default: child 0 + ordered fallback).
            _ => Box::new(aimux_core::router::RuleRouter),
        };
        let fallback = if cfg.fallback.as_deref() == Some("none") {
            aimux_core::router::FallbackPolicy::None
        } else {
            aimux_core::router::FallbackPolicy::OnError
        };
        let router_cfg = aimux_core::router::RouterConfig {
            provider_name: cfg.provider_name.unwrap_or_else(|| "router".into()),
            model_id: cfg.model_id.unwrap_or_else(|| "router".into()),
        };
        let model = aimux_core::router::RouterModel::new(models, router, fallback, router_cfg);
        Ok(intern_model(Arc::new(model)))
    })
}

/// Create a `MoaModel` (RFC-0022) over the given reference handles + one
/// aggregator handle. The new handle is a model handle (works with
/// `aimux_generate_text` / `aimux_stream_text`); free it with
/// `aimux_drop_handle`.
///
/// `reference_handles` is an array of `ref_len` existing model handles (may be
/// 0 — MoaModel then degrades to running just the aggregator). `aggregator`
/// is a single existing model handle. `config_json` is a serialized `MoaConfig`
/// (all fields optional): `{ "provider_name": "moa", "model_id": "moa",
/// "aggregator_instructions": null, "strip_reference_tools": true,
/// "fail_mode": "best_effort" | "fail_fast" }`.
///
/// Fails on: bad JSON, an unknown aggregator handle (references may be empty;
/// aggregator may not).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_moa_new(
    reference_handles: *const u64,
    ref_len: usize,
    aggregator: u64,
    config_json: *const c_char,
    out_handle: *mut u64,
) -> *mut aimux_error_t {
    with_out_handle(out_handle, || {
        let aggregator_model = get_model(aggregator).ok_or(FfiError::InvalidHandle {
            expected: "moa aggregator",
        })?;
        let references = model_handles(
            reference_handles,
            ref_len,
            "reference_handles",
            "moa reference",
        )?;
        // NULL / empty / "null" all mean "defaults" (matching parse_provider_options
        // convention). Invalid UTF-8 → FFI `InvalidUtf8`.
        let config_json = normalize_config_json(config_json, "config_json")?;
        let cfg: aimux_core::moa::MoaConfig =
            serde_json::from_str(&config_json).map_err(|e| wire_err("config_json", e))?;
        let model = aimux_core::moa::MoaModel::new(references, aggregator_model, cfg);
        Ok(intern_model(Arc::new(model)))
    })
}

/// FFI-side router config (lenient: all fields optional).
#[derive(Default, serde::Deserialize)]
struct RouterFfiConfig {
    router: Option<String>,
    weights: Option<Vec<f64>>,
    fallback: Option<String>,
    provider_name: Option<String>,
    model_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use aimux_core::ApiCallError;
    use std::time::Duration;

    use super::*;

    /// Own the string a getter returns; None for NULL.
    fn take(p: *mut c_char) -> Option<String> {
        if p.is_null() {
            return None;
        }
        let s = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string();
        unsafe { aimux_free_string(p) };
        Some(s)
    }

    /// The owned returned error a failing call would return for `p`.
    fn boxed(p: impl Into<AiMuxFfiError>) -> *mut aimux_error_t {
        finish::<()>(Err(p.into()), |()| {})
    }

    /// Message of a returned error, which is then freed.
    fn msg(e: *mut aimux_error_t) -> String {
        assert!(!e.is_null(), "expected a returned error");
        let m = take(aimux_error_message(e)).unwrap();
        aimux_error_free(e);
        m
    }

    /// AiMuxError code and message; the returned error is freed.
    fn expect_aimux_error(e: *mut aimux_error_t) -> (i32, String) {
        assert!(!e.is_null(), "expected a returned error");
        let code = aimux_error_code(e);
        if !(AIMUX_E_OTHER..=AIMUX_E_ABORTED).contains(&code) {
            panic!("expected an AiMuxError code, got {code}: {}", msg(e));
        }
        let out = (code, take(aimux_error_message(e)).unwrap());
        aimux_error_free(e);
        out
    }

    /// C ABI failure code and non-empty message; the returned error is freed.
    fn expect_ffi_error(e: *mut aimux_error_t) -> String {
        assert!(!e.is_null(), "expected a returned error");
        let code = aimux_error_code(e);
        assert!(
            (AIMUX_E_FFI_NULL_POINTER..=AIMUX_E_FFI_CALLBACK_FAILURE).contains(&code),
            "expected a C ABI failure code, got {code}"
        );
        let m = msg(e);
        assert!(!m.is_empty());
        m
    }

    fn zero_err() -> u64 {
        0
    }

    /// Each payload getter answers under the one code that owns it and is
    /// NULL / -1 / 0 under every other; a NULL pointer answers as "no error".
    #[test]
    fn payload_getters_follow_the_code() {
        let owner = boxed(AiMuxError::ApiCall(ApiCallError {
            status_code: Some(429),
            provider_code: Some("insufficient_quota".into()),
            message: "quota".into(),
            response_body: Some("{}".into()),
            request_id: Some("req_1".into()),
            retry_after_ms: Some(1500),
            is_retryable: true,
        }));
        let h = owner;
        assert_eq!(aimux_error_code(h), AIMUX_E_API_CALL);
        assert_eq!(aimux_error_status(h), 429);
        assert_eq!(aimux_error_retry_ms(h), 1500);
        assert_eq!(aimux_error_retryable(h), 1);
        assert_eq!(
            take(aimux_error_provider_code(h)).as_deref(),
            Some("insufficient_quota")
        );
        // The variant's own text, not the composed Display form in `message`.
        assert_eq!(
            take(aimux_error_provider_message(h)).as_deref(),
            Some("quota")
        );
        let m = take(aimux_error_message(h)).unwrap();
        assert!(m.contains("quota") && m != "quota", "{m}");
        assert_eq!(take(aimux_error_request_id(h)).as_deref(), Some("req_1"));
        assert_eq!(take(aimux_error_response_body(h)).as_deref(), Some("{}"));
        assert!(aimux_error_model_id(h).is_null());
        assert!(aimux_error_model_type(h).is_null());
        assert!(aimux_error_provider_id(h).is_null());
        aimux_error_free(owner);

        // Absent Option fields are NULL, not empty strings.
        let owner = boxed(AiMuxError::ApiCall(ApiCallError {
            message: "x".into(),
            ..Default::default()
        }));
        let h = owner;
        assert!(aimux_error_provider_code(h).is_null());
        assert!(aimux_error_request_id(h).is_null());
        assert!(aimux_error_response_body(h).is_null());
        assert_eq!(take(aimux_error_provider_message(h)).as_deref(), Some("x"));
        aimux_error_free(owner);

        let owner = boxed(AiMuxError::NoSuchModel {
            model_id: "m".into(),
            model_type: "language".into(),
        });
        let h = owner;
        assert_eq!(aimux_error_code(h), AIMUX_E_NO_SUCH_MODEL);
        assert_eq!(take(aimux_error_model_id(h)).as_deref(), Some("m"));
        assert_eq!(take(aimux_error_model_type(h)).as_deref(), Some("language"));
        assert!(aimux_error_provider_code(h).is_null());
        assert!(aimux_error_provider_id(h).is_null());
        assert_eq!(aimux_error_status(h), -1);
        assert_eq!(aimux_error_retryable(h), 0);
        aimux_error_free(owner);

        let owner = boxed(AiMuxError::NoSuchProvider {
            provider_id: "p".into(),
        });
        let h = owner;
        assert_eq!(aimux_error_code(h), AIMUX_E_NO_SUCH_PROVIDER);
        assert_eq!(take(aimux_error_provider_id(h)).as_deref(), Some("p"));
        assert!(aimux_error_model_id(h).is_null());
        aimux_error_free(owner);

        // NULL pointer: never UB, answers "no error".
        let n = std::ptr::null();
        assert_eq!(aimux_error_code(n), AIMUX_OK);
        assert!(aimux_error_message(n).is_null());
        assert_eq!(aimux_error_status(n), -1);
        assert_eq!(aimux_error_retry_ms(n), -1);
        assert_eq!(aimux_error_retryable(n), 0);
        assert!(aimux_error_provider_code(n).is_null());
    }

    /// The retry verdict crosses the ABI as its own getter. It is not
    /// derivable from `status`: both cases below report -1, and they disagree.
    #[test]
    fn retryable_crosses_the_abi_and_status_cannot_stand_in() {
        let transport_owner = boxed(AiMuxError::ApiCall(ApiCallError {
            message: "connection reset".into(),
            is_retryable: true,
            ..Default::default()
        }));
        let transport = transport_owner;
        assert_eq!(aimux_error_status(transport), -1);
        assert_eq!(aimux_error_retryable(transport), 1);
        aimux_error_free(transport_owner);

        let no_key_owner = boxed(AiMuxError::ApiCall(ApiCallError {
            message: "no api key".into(),
            ..Default::default()
        }));
        let no_key = no_key_owner;
        assert_eq!(
            aimux_error_status(no_key),
            -1,
            "same sentinel as the transport failure"
        );
        assert_eq!(aimux_error_retryable(no_key), 0, "but the opposite verdict");
        aimux_error_free(no_key_owner);

        // Non-ApiCall variants are never retryable.
        let arg_owner = boxed(AiMuxError::InvalidArgument("bad".into()));
        let arg = arg_owner;
        assert_eq!(aimux_error_retryable(arg), 0);
        aimux_error_free(arg_owner);
    }

    /// The unified code space identifies every returned error source.
    #[test]
    fn unified_error_codes_cover_every_source() {
        // AiMuxError.
        let e = boxed(AiMuxError::Aborted);
        assert_eq!(aimux_error_code(e), AIMUX_E_ABORTED);
        assert_eq!(msg(e), AiMuxError::Aborted.to_string());

        // Recording.
        use RecordingError as R;
        let cases = [
            (R::WriterGone, AIMUX_E_RECORDING_WRITER_GONE),
            (R::FlushTimeout, AIMUX_E_RECORDING_FLUSH_TIMEOUT),
            (R::Write("ENOSPC".into()), AIMUX_E_RECORDING_WRITE),
            (
                R::Spawn {
                    source: std::io::Error::other("x"),
                },
                AIMUX_E_RECORDING_SPAWN,
            ),
        ];
        for (r, code) in cases {
            let e = boxed(r);
            assert_eq!(aimux_error_code(e), code);
            assert!(!msg(e).is_empty());
        }
        // NULL means success.
        assert_eq!(aimux_error_code(std::ptr::null()), AIMUX_OK);
        assert!(aimux_error_message(std::ptr::null()).is_null());

        // FfiError codes and Display texts are pinned.
        let cases: Vec<(FfiError, i32, &str)> = vec![
            (
                FfiError::NullPointer {
                    argument: "api_key",
                },
                AIMUX_E_FFI_NULL_POINTER,
                "api_key: must not be NULL",
            ),
            (
                FfiError::InvalidUtf8 {
                    argument: "model_id",
                },
                AIMUX_E_FFI_INVALID_UTF8,
                "model_id: must be valid UTF-8",
            ),
            (
                FfiError::InvalidWireJson {
                    argument: "opts_json",
                    message: "expected `,`".into(),
                },
                AIMUX_E_FFI_INVALID_WIRE_JSON,
                "opts_json: invalid JSON: expected `,`",
            ),
            (
                FfiError::InvalidHandle { expected: "model" },
                AIMUX_E_FFI_INVALID_HANDLE,
                "invalid or expired model handle",
            ),
            (
                FfiError::ReentrantCall,
                AIMUX_E_FFI_REENTRANT_CALL,
                "re-entrant FFI call from within a callback is not allowed",
            ),
            (
                FfiError::ResultSerialization {
                    message: "serialize".into(),
                },
                AIMUX_E_FFI_RESULT_SERIALIZATION,
                "could not serialize result: serialize",
            ),
            (
                FfiError::CallbackFailure {
                    message: "on_part panicked".into(),
                },
                AIMUX_E_FFI_CALLBACK_FAILURE,
                "host callback failed: on_part panicked",
            ),
        ];
        for (f, code, text) in cases {
            let e = boxed(f);
            assert_eq!(aimux_error_code(e), code);
            assert_eq!(expect_ffi_error(e), text);
        }

        // NULL owner: every accessor is NULL-safe.
        let n: *mut aimux_error_t = std::ptr::null_mut();
        assert_eq!(aimux_error_code(n), AIMUX_OK);
        assert!(aimux_error_message(n).is_null());
        aimux_error_free(n);
    }

    /// Even on a recording entry point, malformed C input is an `FfiError`;
    /// only failures produced by `JsonlRecorder` carry the recording
    /// code.
    #[test]
    fn recording_init_reports_ffi_codes_for_invalid_input() {
        assert_eq!(
            expect_ffi_error(aimux_init_recording(std::ptr::null())),
            "dir: must not be NULL"
        );
        let invalid_utf8 = [0xff_u8, 0];
        assert_eq!(
            expect_ffi_error(aimux_init_recording(invalid_utf8.as_ptr().cast())),
            "dir: must be valid UTF-8"
        );
    }

    /// Pin the full 13-variant → code mapping.
    #[test]
    fn error_code_mapping_covers_all_variants() {
        let s = |t: &str| t.to_string();
        let cases: Vec<(AiMuxError, i32)> = vec![
            (
                AiMuxError::ApiCall(ApiCallError {
                    message: s("x"),
                    ..Default::default()
                }),
                AIMUX_E_API_CALL,
            ),
            (AiMuxError::JsonParse(s("x")), AIMUX_E_JSON_PARSE),
            (
                AiMuxError::InvalidResponseData(s("x")),
                AIMUX_E_INVALID_RESPONSE_DATA,
            ),
            (AiMuxError::Tool(s("x")), AIMUX_E_TOOL),
            (
                AiMuxError::InvalidArgument(s("x")),
                AIMUX_E_INVALID_ARGUMENT,
            ),
            (AiMuxError::InvalidPrompt(s("x")), AIMUX_E_INVALID_PROMPT),
            (AiMuxError::TokenExpired(s("x")), AIMUX_E_TOKEN_EXPIRED),
            (
                AiMuxError::UnsupportedFunctionality(s("x")),
                AIMUX_E_UNSUPPORTED_FUNCTIONALITY,
            ),
            (
                AiMuxError::NoSuchModel {
                    model_id: s("x"),
                    model_type: String::new(),
                },
                AIMUX_E_NO_SUCH_MODEL,
            ),
            (
                AiMuxError::NoSuchProvider {
                    provider_id: s("x"),
                },
                AIMUX_E_NO_SUCH_PROVIDER,
            ),
            (AiMuxError::Timeout(s("x")), AIMUX_E_TIMEOUT),
            (AiMuxError::Aborted, AIMUX_E_ABORTED),
            (AiMuxError::Other(s("x")), AIMUX_E_OTHER),
        ];
        for (e, code) in cases {
            let expect = format!("variant {e:?}");
            let (got, _) = expect_aimux_error(boxed(e));
            assert_eq!(got, code, "{expect}");
        }
    }

    /// Interior NUL bytes must not corrupt or truncate the message.
    #[test]
    fn error_message_sanitizes_interior_nul() {
        let (_, m) = expect_aimux_error(boxed(AiMuxError::Other("a\0b".into())));
        assert_eq!(m, "a\u{FFFD}b");
    }

    /// A NULL out-parameter is reported, not dereferenced.
    #[test]
    fn null_out_param_is_an_ffi_error() {
        let key = std::ffi::CString::new("k").unwrap();
        assert_eq!(
            expect_ffi_error(aimux_openai_new(
                key.as_ptr(),
                key.as_ptr(),
                std::ptr::null_mut()
            )),
            "out_handle: must not be NULL"
        );
        assert_eq!(
            expect_ffi_error(aimux_list_sessions(std::ptr::null_mut())),
            "out_json: must not be NULL"
        );
    }

    // ── invoke_stream_callback (issue #64: FFI panic guard) ───────────────────

    #[test]
    fn stream_callback_ok_passthrough() {
        // A callback that returns normally must pass through as Ok.
        let called = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let called_clone = called.clone();
        let result = invoke_stream_callback("on_part", || {
            called_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        assert!(result.is_ok());
        assert_eq!(
            called.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "callback must have been invoked"
        );
    }

    #[test]
    fn stream_callback_catches_str_panic() {
        // A panic with a &'static str payload must be caught and converted.
        let result = invoke_stream_callback("on_part", || {
            panic!("callback explosion");
        });
        match result {
            Err(FfiError::CallbackFailure { message: msg }) => {
                assert!(
                    msg.contains("on_part"),
                    "message should name the callback: {msg}"
                );
                assert!(
                    msg.contains("callback explosion"),
                    "message should include the panic payload: {msg}"
                );
            }
            other => panic!("expected FfiError::CallbackFailure, got {other:?}"),
        }
    }

    #[test]
    fn stream_callback_catches_string_panic() {
        // A panic with a String payload must also be caught.
        let result = invoke_stream_callback("on_done", || {
            panic!("{}", "dynamic boom".to_string());
        });
        match result {
            Err(FfiError::CallbackFailure { message: msg }) => {
                assert!(
                    msg.contains("on_done"),
                    "message should name the callback: {msg}"
                );
                assert!(
                    msg.contains("dynamic boom"),
                    "message should include the panic payload: {msg}"
                );
            }
            other => panic!("expected FfiError::CallbackFailure, got {other:?}"),
        }
    }

    #[test]
    fn stream_callback_catches_non_string_panic() {
        // A panic with a non-string payload (e.g. a struct) must still be
        // caught — the message falls back to the placeholder.
        let result = invoke_stream_callback("on_part", || {
            std::panic::panic_any(42i32);
        });
        match result {
            Err(FfiError::CallbackFailure { message: msg }) => {
                assert!(
                    msg.contains("on_part"),
                    "message should name the callback: {msg}"
                );
                assert!(
                    msg.contains("<non-string panic>"),
                    "non-string payload should use placeholder: {msg}"
                );
            }
            other => panic!("expected FfiError::CallbackFailure, got {other:?}"),
        }
    }

    #[test]
    fn stream_callback_catches_on_done_panic() {
        // Symmetric coverage for the on_done callback path (the other panic
        // tests exercise on_part). The guard is identical, but this pins the
        // on_done name in the error message.
        let result = invoke_stream_callback("on_done", || {
            panic!("done callback failed");
        });
        match result {
            Err(FfiError::CallbackFailure { message: msg }) => {
                assert!(
                    msg.contains("on_done"),
                    "message should name the callback: {msg}"
                );
                assert!(
                    msg.contains("done callback failed"),
                    "message should include the panic payload: {msg}"
                );
            }
            other => panic!("expected FfiError::CallbackFailure, got {other:?}"),
        }
    }

    /// A tiny mock model for composite-FFI tests (returns fixed text).
    struct MockText {
        provider: &'static str,
        model_id: &'static str,
        text: &'static str,
    }
    #[async_trait::async_trait]
    impl aimux_core::LanguageModel for MockText {
        fn provider(&self) -> &str {
            self.provider
        }
        fn model_id(&self) -> &str {
            self.model_id
        }
        async fn do_generate(
            &self,
            _options: &aimux_core::options::CallOptions,
        ) -> Result<aimux_core::result::GenerateResult, AiMuxError> {
            Ok(aimux_core::result::GenerateResult {
                content: vec![aimux_core::result::GenerateContent::Text {
                    text: self.text.into(),
                    provider_metadata: None,
                }],
                finish_reason: aimux_core::types::FinishReason {
                    unified: aimux_core::types::FinishReasonUnified::Stop,
                    raw: None,
                },
                usage: aimux_core::types::Usage::default(),
                warnings: vec![],
                provider_metadata: None,
                response: aimux_core::types::ResponseMetadata {
                    model_id: Some(self.model_id.into()),
                    ..Default::default()
                },
                request_body: None,
                response_headers: None,
            })
        }
        async fn do_stream(
            &self,
            _options: &aimux_core::options::CallOptions,
        ) -> Result<aimux_core::result::StreamResult, AiMuxError> {
            unimplemented!()
        }
    }

    fn mock_handle(provider: &'static str, model_id: &'static str, text: &'static str) -> u64 {
        intern_model(Arc::new(MockText {
            provider,
            model_id,
            text,
        }))
    }

    #[test]
    fn router_new_builds_router_model() {
        let handles = [
            mock_handle("mock", "primary", "primary-out"),
            mock_handle("mock", "backup", "backup-out"),
        ];
        let mut h = zero_err();
        let e = aimux_router_new(handles.as_ptr(), handles.len(), std::ptr::null(), &mut h);
        assert!(e.is_null(), "{}", msg(e));
        assert!(h != 0, "router handle must be non-zero");
        // Confirm it resolves to a model.
        let model = get_model(h).expect("router handle resolves");
        assert_eq!(model.provider(), "router");
        assert_eq!(model.model_id(), "router");
    }

    #[test]
    fn router_new_rejects_empty_handles() {
        // len == 0 (with or without a NULL array) is "no usable child", not a
        // null pointer; a NULL array with len > 0 is.
        let mut h = zero_err();
        let e = aimux_router_new(std::ptr::null(), 0, std::ptr::null(), &mut h);
        assert_eq!(h, 0);
        assert_eq!(
            expect_ffi_error(e),
            "invalid or expired router child handle"
        );
        let e = aimux_router_new(std::ptr::null(), 2, std::ptr::null(), &mut h);
        assert_eq!(h, 0);
        assert_eq!(expect_ffi_error(e), "handles: must not be NULL");
    }

    #[test]
    fn router_new_rejects_bad_json() {
        let handles = [mock_handle("mock", "m", "x")];
        let bad = std::ffi::CString::new("{not json").unwrap();
        let mut h = zero_err();
        let e = aimux_router_new(handles.as_ptr(), handles.len(), bad.as_ptr(), &mut h);
        assert_eq!(h, 0);
        assert!(expect_ffi_error(e).starts_with("config_json: invalid JSON:"));
    }

    #[test]
    fn router_new_treats_empty_config_as_defaults() {
        // S1 guard: empty string / "null" config must NOT be a JSON_PARSE error
        // (consistent with other config-bearing FFI entry points).
        let handles = [mock_handle("mock", "m", "x")];
        for cfg in ["", "  ", "null"] {
            let c = std::ffi::CString::new(cfg).unwrap();
            let mut h = zero_err();
            let e = aimux_router_new(handles.as_ptr(), handles.len(), c.as_ptr(), &mut h);
            assert!(e.is_null(), "router with config {cfg:?}: {}", msg(e));
            assert!(h != 0);
        }
    }

    #[test]
    fn moa_new_builds_moa_model() {
        let refs = [
            mock_handle("mock", "ref-a", "A"),
            mock_handle("mock", "ref-b", "B"),
        ];
        let agg = mock_handle("mock", "aggregator", "agg");
        let mut h = zero_err();
        let e = aimux_moa_new(refs.as_ptr(), refs.len(), agg, std::ptr::null(), &mut h);
        assert!(e.is_null(), "{}", msg(e));
        assert!(h != 0, "moa handle must be non-zero");
        let model = get_model(h).expect("moa handle resolves");
        assert_eq!(model.provider(), "moa");
        assert_eq!(model.model_id(), "moa");
    }

    #[test]
    fn moa_new_rejects_bad_aggregator() {
        let refs = [mock_handle("mock", "ref-a", "A")];
        let mut h = zero_err();
        // aggregator handle 999999 does not exist.
        let e = aimux_moa_new(refs.as_ptr(), refs.len(), 999_999, std::ptr::null(), &mut h);
        assert_eq!(h, 0);
        assert_eq!(
            expect_ffi_error(e),
            "invalid or expired moa aggregator handle"
        );
    }

    #[test]
    fn composites_reject_any_dead_member_handle() {
        // A dead handle inside the array is a caller bug: reported, never
        // silently dropped (the composite would otherwise run with fewer
        // members than the caller believes).
        let live = mock_handle("mock", "live", "L");
        let dead = mock_handle("mock", "dead", "D");
        aimux_drop_handle(dead);
        let handles = [live, dead];

        let mut h = zero_err();
        let e = aimux_router_new(handles.as_ptr(), handles.len(), std::ptr::null(), &mut h);
        assert_eq!(h, 0);
        assert_eq!(
            expect_ffi_error(e),
            "invalid or expired router child handle"
        );

        let agg = mock_handle("mock", "aggregator", "agg");
        let e = aimux_moa_new(
            handles.as_ptr(),
            handles.len(),
            agg,
            std::ptr::null(),
            &mut h,
        );
        assert_eq!(h, 0);
        assert_eq!(
            expect_ffi_error(e),
            "invalid or expired moa reference handle"
        );

        // NULL with ref_len > 0 is a null pointer, same as router.
        let e = aimux_moa_new(std::ptr::null(), 2, agg, std::ptr::null(), &mut h);
        assert_eq!(h, 0);
        assert_eq!(expect_ffi_error(e), "reference_handles: must not be NULL");
    }

    #[test]
    fn moa_new_allows_zero_references() {
        // 0 references is valid (degrades to aggregator-only).
        let agg = mock_handle("mock", "aggregator", "agg");
        let mut h = zero_err();
        let e = aimux_moa_new(std::ptr::null(), 0, agg, std::ptr::null(), &mut h);
        assert!(e.is_null(), "{}", msg(e));
        assert!(h != 0, "moa with 0 references should succeed");
    }

    // ── Transcription streaming sessions (RFC-0028 Phase 2) ──────────────

    /// A mock transcription model whose `do_stream` echoes: one delta per
    /// received audio chunk, then final + finish after the audio ends.
    /// Honors the abort signal.
    struct MockStreamingTranscriber;

    #[async_trait::async_trait]
    impl aimux_core::transcription_model::TranscriptionModel for MockStreamingTranscriber {
        fn provider(&self) -> &str {
            "mock"
        }
        fn model_id(&self) -> &str {
            "mock-stream-stt"
        }
        async fn do_generate(
            &self,
            _options: &aimux_core::transcription_model::TranscriptionCallOptions,
        ) -> Result<aimux_core::transcription_model::TranscriptionResult, AiMuxError> {
            unimplemented!()
        }
        async fn do_stream(
            &self,
            options: aimux_core::transcription_model::TranscriptionStreamOptions,
        ) -> Result<aimux_core::transcription_model::TranscriptionStreamResult, AiMuxError>
        {
            use aimux_core::transcription_model::{
                AudioChunk, TranscriptionStreamPart, TranscriptionStreamResult,
            };
            use futures::StreamExt;
            let mut audio = options.audio;
            let abort = options.abort_signal.clone();
            let stream = async_stream::stream! {
                yield Ok(TranscriptionStreamPart::StreamStart { warnings: vec![] });
                let mut text = String::new();
                loop {
                    let aborted = async {
                        match &abort {
                            Some(s) => s.cancelled().await,
                            None => std::future::pending().await,
                        }
                    };
                    tokio::select! {
                        _ = aborted => {
                            yield Err(AiMuxError::Aborted);
                            break;
                        }
                        chunk = audio.next() => match chunk {
                            Some(AudioChunk::Binary(b)) => {
                                let s = format!("{} ", b.len());
                                text.push_str(&s);
                                yield Ok(TranscriptionStreamPart::TranscriptDelta {
                                    id: None,
                                    delta: s,
                                    provider_metadata: None,
                                });
                            }
                            Some(AudioChunk::Base64(s)) => {
                                text.push_str(&s);
                                yield Ok(TranscriptionStreamPart::TranscriptDelta {
                                    id: None,
                                    delta: s,
                                    provider_metadata: None,
                                });
                            }
                            None => {
                                yield Ok(TranscriptionStreamPart::TranscriptFinal {
                                    id: None,
                                    text: text.clone(),
                                    start_second: None,
                                    end_second: None,
                                    channel_index: None,
                                    provider_metadata: None,
                                });
                                yield Ok(TranscriptionStreamPart::Finish {
                                    text,
                                    segments: vec![],
                                    language: None,
                                    duration_in_seconds: None,
                                    provider_metadata: None,
                                });
                                break;
                            }
                        },
                    }
                }
            };
            Ok(TranscriptionStreamResult {
                stream: Box::pin(stream),
                request: None,
                response: None,
            })
        }
    }

    fn mock_transcriber_handle() -> u64 {
        intern_handle(HandleEntry::Transcription(Arc::new(
            MockStreamingTranscriber,
        )))
    }

    fn session(model: u64, abort: u64) -> u64 {
        let mut s = zero_err();
        let e = aimux_transcription_session_new(model, abort, std::ptr::null(), &mut s);
        assert!(e.is_null(), "{}", msg(e));
        assert!(s != 0, "session should start");
        s
    }

    /// One `next_part` pull: `(state, part)`; a non-NULL error is returned
    /// as `Err`.
    fn pull(session: u64, timeout_ms: i64) -> Result<(i32, Option<String>), *mut aimux_error_t> {
        let mut part: *mut c_char = std::ptr::null_mut();
        let mut state: i32 = 0;
        let e = aimux_transcription_next_part(session, timeout_ms, &mut part, &mut state);
        if !e.is_null() {
            assert!(part.is_null(), "failure must leave *out_part NULL");
            return Err(e);
        }
        Ok((state, take(part)))
    }

    fn pull_part(session: u64) -> String {
        match pull(session, 2_000) {
            Ok((AIMUX_TRANSCRIPTION_NEXT_PART_PART, Some(p))) => p,
            Ok(other) => panic!("expected a part, got {other:?}"),
            Err(e) => panic!("expected a part, got error: {}", msg(e)),
        }
    }

    #[test]
    fn transcription_session_full_lifecycle() {
        let model = mock_transcriber_handle();
        let session = session(model, 0);

        // Push two chunks + end-of-input.
        let chunk1 = [1u8, 2, 3];
        let chunk2 = [4u8, 5];
        assert!(aimux_transcription_push_audio(session, chunk1.as_ptr(), 3).is_null());
        assert!(aimux_transcription_push_audio(session, chunk2.as_ptr(), 2).is_null());
        assert!(aimux_transcription_input_done(session).is_null());

        // Pull parts: StreamStart, delta("3 "), delta("2 "), Final, Finish,
        // then ENDED.
        let p1 = pull_part(session);
        assert!(p1.contains("StreamStart"), "part 1: {p1}");
        let p2 = pull_part(session);
        assert!(p2.contains("3 "), "part 2: {p2}");
        let p3 = pull_part(session);
        assert!(p3.contains("2 "), "part 3: {p3}");
        let p4 = pull_part(session);
        assert!(p4.contains("TranscriptFinal"), "part 4: {p4}");
        let p5 = pull_part(session);
        assert!(p5.contains("Finish"), "part 5: {p5}");
        assert!(p5.contains("3 2 "), "finish text: {p5}");

        // Stream ended: NULL error, ENDED state, no part.
        assert_eq!(
            pull(session, 2_000).ok(),
            Some((AIMUX_TRANSCRIPTION_NEXT_PART_ENDED, None))
        );

        aimux_transcription_session_drop(session);
    }

    #[test]
    fn transcription_session_timeout_is_a_state_not_an_error() {
        let model = mock_transcriber_handle();
        let session = session(model, 0);
        // Consume StreamStart, then DON'T push audio: next_part times out
        // (the mock waits for more audio).
        let _ = pull_part(session);
        assert_eq!(
            pull(session, 50).ok(),
            Some((AIMUX_TRANSCRIPTION_NEXT_PART_TIMEOUT, None))
        );

        // Session still alive: push + finish works after the timeout.
        let chunk = [9u8];
        assert!(aimux_transcription_push_audio(session, chunk.as_ptr(), 1).is_null());
        assert!(aimux_transcription_input_done(session).is_null());
        let p = pull_part(session);
        assert!(p.contains("1 "), "post-timeout part: {p}");
        aimux_transcription_session_drop(session);
    }

    #[test]
    fn transcription_session_abort_via_signal() {
        let model = mock_transcriber_handle();
        let abort = crate::aimux_abort_signal_new();
        let session = session(model, abort);
        // Drain StreamStart.
        let _ = pull_part(session);

        // Abort; the mock yields Err(Aborted).
        crate::aimux_abort_signal_abort(abort);
        let e = pull(session, 2_000).expect_err("aborted stream must fail");
        assert_eq!(expect_aimux_error(e).0, AIMUX_E_ABORTED);

        // Push after abort fails or is buffered (the session's channels are
        // closing); either way no panic.
        let chunk = [1u8];
        let e = aimux_transcription_push_audio(session, chunk.as_ptr(), 1);
        aimux_error_free(e);
        aimux_transcription_session_drop(session);
    }

    #[test]
    fn transcription_session_drop_is_safe_and_idempotent() {
        let model = mock_transcriber_handle();
        let session = session(model, 0);
        aimux_transcription_session_drop(session);
        aimux_transcription_session_drop(session); // idempotent
        aimux_transcription_session_drop(0); // safe with 0
        aimux_transcription_session_drop(999_999); // safe with unknown

        // Operations on a dropped session fail with invalid handle.
        assert_eq!(
            expect_ffi_error(aimux_transcription_push_audio(session, std::ptr::null(), 0)),
            "invalid or expired transcription session handle"
        );
        assert_eq!(
            expect_ffi_error(aimux_transcription_input_done(session)),
            "invalid or expired transcription session handle"
        );
        let e = pull(session, 0).expect_err("dead session");
        assert_eq!(
            expect_ffi_error(e),
            "invalid or expired transcription session handle"
        );
    }

    #[test]
    fn transcription_session_drop_unblocks_infinite_next_part() {
        let model = mock_transcriber_handle();
        let session = session(model, 0);
        // Consume StreamStart; with no audio, the next pull waits forever
        // unless session_drop aborts the driver and closes the parts stream.
        let _ = pull_part(session);

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let puller = std::thread::spawn(move || {
            if let Err(e) = pull(session, -1) {
                aimux_error_free(e);
            }
            done_tx.send(()).unwrap();
        });
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            matches!(
                done_rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ),
            "next_part(-1) should still be blocked before drop"
        );

        let started = std::time::Instant::now();
        aimux_transcription_session_drop(session);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "session_drop should not wait for next_part"
        );
        done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("session_drop must wake next_part(-1)");
        puller.join().unwrap();
        drop_handle(model);
    }

    #[test]
    fn transcription_session_drop_unblocks_backpressured_push_audio() {
        let model = mock_transcriber_handle();
        let session = session(model, 0);
        let sent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let sent_by_pusher = Arc::clone(&sent);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let pusher = std::thread::spawn(move || {
            let chunk = [1u8];
            for _ in 0..1024 {
                let e = aimux_transcription_push_audio(session, chunk.as_ptr(), chunk.len());
                if !e.is_null() {
                    aimux_error_free(e);
                    break;
                }
                sent_by_pusher.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            done_tx.send(()).unwrap();
        });

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while sent.load(std::sync::atomic::Ordering::Relaxed) < 64
            && std::time::Instant::now() < deadline
        {
            std::thread::yield_now();
        }
        assert!(
            sent.load(std::sync::atomic::Ordering::Relaxed) >= 64,
            "pusher never filled the bounded audio path"
        );
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            matches!(
                done_rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ),
            "pusher unexpectedly completed without backpressure"
        );

        let started = std::time::Instant::now();
        aimux_transcription_session_drop(session);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "session_drop should not wait for push_audio"
        );
        done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("session_drop must wake a backpressured push_audio");
        pusher.join().unwrap();
        drop_handle(model);
    }

    #[test]
    fn transcription_session_new_rejects_bad_inputs() {
        let model = mock_transcriber_handle();
        let mut h = zero_err();
        // Bad JSON.
        let bad = std::ffi::CString::new("{not json").unwrap();
        let e = aimux_transcription_session_new(model, 0, bad.as_ptr(), &mut h);
        assert_eq!(h, 0);
        assert!(expect_ffi_error(e).starts_with("opts_json: invalid JSON:"));

        // Non-transcription model handle.
        let lang = mock_handle("mock", "m", "x");
        let e = aimux_transcription_session_new(lang, 0, std::ptr::null(), &mut h);
        assert_eq!(h, 0);
        assert_eq!(
            expect_ffi_error(e),
            "invalid or expired transcription handle"
        );

        // Bad abort handle.
        let e = aimux_transcription_session_new(model, 999_999, std::ptr::null(), &mut h);
        assert_eq!(h, 0);
        assert_eq!(expect_ffi_error(e), "invalid or expired abort handle");

        // NULL out-params on next_part.
        let mut part: *mut c_char = std::ptr::null_mut();
        assert_eq!(
            expect_ffi_error(aimux_transcription_next_part(
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut()
            )),
            "out_part: must not be NULL"
        );
        assert_eq!(
            expect_ffi_error(aimux_transcription_next_part(
                0,
                0,
                &mut part,
                std::ptr::null_mut()
            )),
            "out_state: must not be NULL"
        );
    }
}
