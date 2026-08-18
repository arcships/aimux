//! aimux-ffi: C ABI boundary for multi-language bindings.
//!
//! Provides an opaque handle registry + JSON wire boundary + push callback
//! stream. Only used by C ABI bindings (Swift / Kotlin / C). Native bindings
//! (Python / Node / Flutter) bypass this layer and use `aimux-providers`
//! directly.
//!
//! ## Memory ownership
//!
//! - Every function returning `*mut c_char` — constructors included —
//!   transfers ownership to the caller, who MUST free it with
//!   [`aimux_free_string`].
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

/// A type-erased FFI handle. One registry holds models and abort signals.
#[derive(Clone)]
enum ModelHandle {
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

type ModelRegistry = HashMap<u64, ModelHandle>;

static REGISTRY: OnceLock<Mutex<ModelRegistry>> = OnceLock::new();
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn registry() -> &'static Mutex<ModelRegistry> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register a model instance, returning its opaque `u64` handle.
///
/// Handles start at 1; 0 is reserved for "failure / invalid".
fn intern_model(model: Arc<dyn LanguageModel>) -> u64 {
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    registry()
        .lock()
        .expect("aimux-ffi: registry mutex poisoned")
        .insert(handle, ModelHandle::Language(model));
    handle
}

fn intern_handle(h: ModelHandle) -> u64 {
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    registry()
        .lock()
        .expect("aimux-ffi: registry mutex poisoned")
        .insert(handle, h);
    handle
}

/// Look up a model by handle, cloning the `Arc` out of the registry.
fn get_model(handle: u64) -> Option<Arc<dyn LanguageModel>> {
    match get_handle(handle)? {
        ModelHandle::Language(m) => Some(m),
        _ => None,
    }
}

/// Resolve a composite's model handles under one registry lock. This makes
/// the lookup all-or-nothing with respect to concurrent handle drops: either
/// every requested model is cloned, or the constructor fails without silently
/// changing the composite's membership.
fn get_models(handles: &[u64]) -> Option<Vec<Arc<dyn LanguageModel>>> {
    let registry = registry()
        .lock()
        .expect("aimux-ffi: registry mutex poisoned");
    handles
        .iter()
        .map(|handle| match registry.get(handle) {
            Some(ModelHandle::Language(model)) => Some(model.clone()),
            _ => None,
        })
        .collect()
}

/// Look up a provider by handle (RFC-0027 provider handles for list_models).
fn get_provider(handle: u64) -> Option<Arc<dyn aimux_core::provider::Provider>> {
    match get_handle(handle)? {
        ModelHandle::Provider(p) => Some(p),
        _ => None,
    }
}

/// Register a provider instance, returning its opaque `u64` handle.
fn intern_provider(provider: Arc<dyn aimux_core::provider::Provider>) -> u64 {
    intern_handle(ModelHandle::Provider(provider))
}

/// Look up any handle (multimodal).
fn get_handle(handle: u64) -> Option<ModelHandle> {
    registry()
        .lock()
        .expect("aimux-ffi: registry mutex poisoned")
        .get(&handle)
        .cloned()
}

fn get_abort_signal(handle: u64) -> Option<AbortSignal> {
    match get_handle(handle)? {
        ModelHandle::Abort(signal) => Some(signal),
        _ => None,
    }
}

/// Remove a handle from the registry (the model drops when the last ref goes).
/// Trace stores bound to the handle are released with it.
fn drop_handle(handle: u64) {
    registry()
        .lock()
        .expect("aimux-ffi: registry mutex poisoned")
        .remove(&handle);
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
    if matches!(registry.get(&handle), Some(ModelHandle::Abort(_))) {
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
    /// a failure sentinel + optional AimuxError instead (issue M7).
    static IN_FFI_BLOCK_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run a future on the shared runtime from an FFI entry point (issue M7).
///
/// Rejects re-entrant calls made from inside a stream callback, returning
/// [`AiMuxError::Other`] instead of letting tokio's nested `block_on` panic —
/// a non-unwind `extern "C"` boundary never lets the panic propagate, so the
/// process would terminate. The guard is released when the future completes,
/// including when it panics.
fn ffi_block_on<F, T>(f: F) -> Result<T, AiMuxError>
where
    F: std::future::Future<Output = T>,
{
    IN_FFI_BLOCK_ON.with(|flag| {
        if flag.replace(true) {
            return Err(AiMuxError::Other(
                "re-entrant FFI call from within a callback is not allowed".to_string(),
            ));
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
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Copy a NUL-terminated C string into an owned `String`.
///
/// Returns `None` for a null pointer or invalid UTF-8.
///
/// # Safety
///
/// `ptr` must be null or a valid NUL-terminated C string valid for the
/// duration of this call.
fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: caller guarantees `ptr` is a valid NUL-terminated C string.
    let cstr = unsafe { CStr::from_ptr(ptr) };
    cstr.to_str().ok().map(str::to_owned)
}

/// Parse the prompt JSON accepted by the FFI.
///
/// Accepts either a bare prompt value (`"text"` or `[{...}]`) or a wrapper
/// object `{"prompt": <value>}`.
fn parse_prompt(json: &str) -> Result<ModelPrompt, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let inner = match &value {
        serde_json::Value::Object(obj) if obj.len() == 1 && obj.contains_key("prompt") => {
            obj.get("prompt").expect("checked by guard")
        }
        _ => &value,
    };
    serde_json::from_value(inner.clone())
}

/// Parse the options JSON. Empty / `null` yields the default options.
fn parse_opts(json: &str) -> Result<GenerateTextOptions, serde_json::Error> {
    let trimmed = json.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return Ok(GenerateTextOptions::default());
    }
    serde_json::from_str(json)
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

// ── C AimuxError (aimux-error.h) ─────────────────────────────────────────────

pub const AIMUX_OK: i32 = 0;
pub const AIMUX_E_UNKNOWN: i32 = 1;
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
pub const AIMUX_E_OTHER: i32 = 14;
/// Recording write failed (sticky) — see `aimux_recording_try_flush`.
pub const AIMUX_E_RECORDING_WRITE: i32 = 15;
/// Recording writer thread unavailable — see `aimux_recording_try_flush`.
pub const AIMUX_E_RECORDING_WRITER_GONE: i32 = 16;
/// Recording flush ack timed out — see `aimux_recording_try_flush`.
pub const AIMUX_E_RECORDING_FLUSH_TIMEOUT: i32 = 17;

/// Layout must match `aimux-error.h` `AimuxError` (40 bytes).
///
/// On failure the callee overwrites every field; `message` is allocated with
/// `into_cstring_raw` and the caller releases it with `aimux_free_string`.
/// `error_value` is the lossless externally-tagged serde JSON of the source
/// [`AiMuxError`] (null when the failure was synthesized at the FFI boundary
/// and has no core error value). `reserved` is future ABI room (the
/// caller-allocated size can never change) and is zeroed on failure.
#[repr(C)]
pub struct CAimuxError {
    pub code: i32,
    pub status: i32,
    pub retry_ms: i64,
    pub message: *mut c_char,
    pub error_value: *mut c_char,
    pub reserved: [*mut c_void; 1],
}

fn aimux_error_code(err: &AiMuxError) -> i32 {
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

/// # Safety: `err` null or writable `CAimuxError`.
unsafe fn fill_error(err: *mut CAimuxError, code: i32, status: i32, retry_ms: i64, msg: &str) {
    if err.is_null() {
        return;
    }
    let e = unsafe { &mut *err };
    e.code = code;
    e.status = status;
    e.retry_ms = retry_ms;
    e.message = into_cstring_raw(msg.to_string());
    e.error_value = std::ptr::null_mut();
    e.reserved = [std::ptr::null_mut(); 1];
}

/// # Safety: `err` null or writable `CAimuxError`.
unsafe fn fill_from_aimux(err: *mut CAimuxError, ae: &AiMuxError) {
    let code = aimux_error_code(ae);
    let status = ae.status_code().map(|s| s as i32).unwrap_or(-1);
    let retry_ms = ae.retry_after_hint().unwrap_or(-1);
    let msg = ae.to_string();
    unsafe { fill_error(err, code, status, retry_ms, &msg) };
    if err.is_null() {
        return;
    }
    // Lossless machine-readable payload alongside the Display message.
    if let Ok(v) = serde_json::to_string(ae) {
        unsafe { (*err).error_value = into_cstring_raw(v) };
    }
}

const INVALID_ARGS: &str = "missing or invalid required argument(s): null or invalid UTF-8";

trait Sentinel {
    fn sentinel() -> Self;
}
impl Sentinel for u64 {
    fn sentinel() -> Self {
        0
    }
}
impl Sentinel for *mut c_char {
    fn sentinel() -> Self {
        std::ptr::null_mut()
    }
}
impl Sentinel for i32 {
    fn sentinel() -> Self {
        0
    }
}

/// Fill optional `err` and return the failure sentinel for `T`.
///
/// # Safety: `err` null or writable `CAimuxError`.
unsafe fn fail<T: Sentinel>(
    err: *mut CAimuxError,
    code: i32,
    status: i32,
    retry_ms: i64,
    msg: &str,
) -> T {
    unsafe { fill_error(err, code, status, retry_ms, msg) };
    T::sentinel()
}

/// # Safety: `err` null or writable `CAimuxError`.
unsafe fn fail_ai<T: Sentinel>(err: *mut CAimuxError, ae: &AiMuxError) -> T {
    unsafe { fill_from_aimux(err, ae) };
    T::sentinel()
}

/// # Safety: `err` null or writable `CAimuxError`.
unsafe fn fail_invalid_args<T: Sentinel>(err: *mut CAimuxError) -> T {
    unsafe { fail(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, INVALID_ARGS) }
}

/// # Safety: `err` null or writable `CAimuxError`.
unsafe fn fail_other<T: Sentinel>(err: *mut CAimuxError, msg: impl std::fmt::Display) -> T {
    unsafe { fail(err, AIMUX_E_OTHER, -1, -1, &msg.to_string()) }
}

/// Failure for a malformed JSON argument.
///
/// # Safety: `err` null or writable `CAimuxError`.
unsafe fn fail_json<T: Sentinel>(err: *mut CAimuxError, msg: impl std::fmt::Display) -> T {
    unsafe { fail(err, AIMUX_E_JSON_PARSE, -1, -1, &msg.to_string()) }
}

/// Failure for an invalid, expired, or wrong-typed handle.
///
/// # Safety: `err` null or writable `CAimuxError`.
unsafe fn fail_invalid_handle<T: Sentinel>(err: *mut CAimuxError, what: &str) -> T {
    unsafe {
        fail(
            err,
            AIMUX_E_INVALID_ARGUMENT,
            -1,
            -1,
            &format!("invalid or expired {what} handle"),
        )
    }
}

/// Invoke a stream callback (`on_part`/`on_done`) while catching any panic.
///
/// A panic inside a `extern "C" fn` callback would unwind across the FFI
/// boundary, which is undefined behavior (issue #64). This wrapper catches
/// the panic and converts it to a structured `AiMuxError::Other` so the host
/// process is not killed (release builds use `panic = "abort"`, but
/// dev/debug builds still unwind).
///
/// `AssertUnwindSafe` is required because the callback receives raw pointers
/// (`*const c_char`/`*mut c_void`) that are not `UnwindSafe`; this is sound
/// because we abort the stream on any panic rather than continuing.
fn invoke_stream_callback(callback_name: &str, f: impl FnOnce()) -> Result<(), AiMuxError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => Ok(()),
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&'static str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("<non-string panic>");
            Err(AiMuxError::Other(format!(
                "stream callback '{callback_name}' panicked: {msg}"
            )))
        }
    }
}

/// Build (key, model_id) from two C strings; None if either is null/invalid.
///
/// # Safety
///
/// `a` and `b` must each be null or point to a valid NUL-terminated C string.
unsafe fn parse_two_args(a: *const c_char, b: *const c_char) -> Option<(String, String)> {
    match (cstr_to_string(a), cstr_to_string(b)) {
        (Some(k), Some(m)) => Some((k, m)),
        _ => None,
    }
}

/// Parse four C string arguments; any null fails the whole call.
unsafe fn parse_four_args(
    a: *const c_char,
    b: *const c_char,
    c: *const c_char,
    d: *const c_char,
) -> Option<(String, String, String, String)> {
    match (
        cstr_to_string(a),
        cstr_to_string(b),
        cstr_to_string(c),
        cstr_to_string(d),
    ) {
        (Some(w), Some(x), Some(y), Some(z)) => Some((w, x, y, z)),
        _ => None,
    }
}

/// Run an async operation and return its result as JSON (caller frees).
fn run_and_serialize<F, T>(err: *mut CAimuxError, f: F) -> *mut c_char
where
    F: std::future::Future<Output = Result<T, AiMuxError>>,
    T: serde::Serialize,
{
    let result = ffi_block_on(f).and_then(|inner| inner);
    match result {
        Ok(r) => match serde_json::to_string(&r) {
            Ok(s) => into_cstring_raw(s),
            Err(e) => unsafe { fail_other(err, format!("serialize: {e}")) },
        },
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Parse the base_url argument; an empty string means unset.
fn parse_base_url(base_url: *const c_char) -> Option<String> {
    cstr_to_string(base_url).filter(|url| !url.is_empty())
}

/// Parse a JSON C-string argument into `T`. A null/invalid pointer maps to
/// `AIMUX_E_INVALID_ARGUMENT`; a JSON parse failure maps to `AIMUX_E_JSON_PARSE`.
fn parse_json_arg<T: DeserializeOwned>(
    json: *const c_char,
    name: &str,
) -> Result<T, (i32, String)> {
    let s = match cstr_to_string(json) {
        Some(s) => s,
        None => return Err((AIMUX_E_INVALID_ARGUMENT, format!("invalid {name}"))),
    };
    serde_json::from_str::<T>(&s).map_err(|e| (AIMUX_E_JSON_PARSE, format!("invalid {name}: {e}")))
}

/// Fail with the (code, message) pair produced by [`parse_json_arg`].
///
/// # Safety: `err` null or writable `CAimuxError`.
unsafe fn fail_code<T: Sentinel>(err: *mut CAimuxError, (code, msg): (i32, String)) -> T {
    unsafe { fail(err, code, -1, -1, &msg) }
}

/// Parse the optional `config_json` argument (`ProviderOptions`); empty or
/// "null" means unset.
fn parse_provider_options(
    config_json: *const c_char,
) -> Result<Option<ProviderOptions>, (i32, String)> {
    match cstr_to_string(config_json) {
        Some(s) if !s.trim().is_empty() && s.trim() != "null" => {
            serde_json::from_str::<ProviderOptions>(&s)
                .map(Some)
                .map_err(|e| (AIMUX_E_JSON_PARSE, format!("invalid config_json: {e}")))
        }
        _ => Ok(None),
    }
}

/// Read a config_json C string and normalize it for lenient deserialization.
///
/// NULL, empty, whitespace-only, and `"null"` all become `"{}"` (defaults),
/// matching `parse_provider_options` / `parse_opts` convention so direct C
/// callers get the documented "empty = defaults" behavior. Invalid UTF-8
/// (cstr_to_string returns None) is still an error.
fn normalize_config_json(config_json: *const c_char, err: *mut CAimuxError) -> Option<String> {
    if config_json.is_null() {
        return Some(String::from("{}"));
    }
    match cstr_to_string(config_json) {
        None => unsafe {
            fill_error(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, INVALID_ARGS);
            None
        },
        Some(s) if s.trim().is_empty() || s.trim() == "null" => Some(String::from("{}")),
        Some(s) => Some(s),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: provider constructors
// ─────────────────────────────────────────────────────────────────────────────

/// Create an OpenAI model instance.
///
/// Returns a non-zero handle on success, or 0 on failure filling `*err` (null
/// arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_new(
    api_key: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    match OpenAIProvider::new(OpenAIConfig::new(api_key)).language_model(&model_id) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create an OpenAI model instance with a custom base URL.
///
/// `base_url` may be null (defaults to the provider's standard URL).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match OpenAIProvider::new(config).language_model(&model_id) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create an Anthropic model instance.
///
/// Returns a non-zero handle on success, or 0 on failure filling `*err` (null
/// arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_new(
    api_key: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    match AnthropicProvider::new(AnthropicConfig::new(api_key)).language_model(&model_id) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create an Anthropic model instance with a custom base URL.
///
/// `base_url` may be null (defaults to the provider's standard URL).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = AnthropicConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match AnthropicProvider::new(config).language_model(&model_id) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create an Anthropic-on-AWS model instance (API key + region).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_aws_new(
    api_key: *const c_char,
    region: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let parsed = match (
        cstr_to_string(api_key),
        cstr_to_string(region),
        cstr_to_string(model_id),
    ) {
        (Some(k), Some(r), Some(m)) => Some((k, r, m)),
        _ => None,
    };
    let Some((api_key, region, model_id)) = parsed else {
        return unsafe { fail_invalid_args(err) };
    };
    match AnthropicAwsProvider::new(AnthropicAwsProviderConfig::with_api_key(api_key, region))
        .language_model(&model_id)
    {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create an Anthropic-on-AWS model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_aws_new_with_base(
    api_key: *const c_char,
    region: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let parsed = match (
        cstr_to_string(api_key),
        cstr_to_string(region),
        cstr_to_string(model_id),
    ) {
        (Some(k), Some(r), Some(m)) => Some((k, r, m)),
        _ => None,
    };
    let Some((api_key, region, model_id)) = parsed else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = AnthropicAwsProviderConfig::with_api_key(api_key, region);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match AnthropicAwsProvider::new(config).language_model(&model_id) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
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
    err: *mut CAimuxError,
) -> u64 {
    let parsed = match (
        cstr_to_string(api_key),
        cstr_to_string(resource_name),
        cstr_to_string(deployment),
    ) {
        (Some(k), Some(r), Some(d)) => Some((k, r, d)),
        _ => None,
    };
    let Some((api_key, resource_name, deployment)) = parsed else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = AzureConfig::new()
        .with_api_key(api_key)
        .with_resource_name(resource_name);
    if let Some(version) = parse_base_url(api_version) {
        config = config.with_api_version(version);
    }
    match AzureProvider::new(config) {
        Ok(p) => match p.language_model(&deployment) {
            Ok(m) => intern_model(Arc::from(m)),
            Err(e) => unsafe { fail_ai(err, &e) },
        },
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create an Azure OpenAI model instance with a custom base URL.
///
/// `api_version` may be null (uses the provider default). The deployment
/// name is passed as `model_id`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_azure_new_with_base(
    api_key: *const c_char,
    base_url: *const c_char,
    deployment: *const c_char,
    api_version: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let parsed = match (
        cstr_to_string(api_key),
        cstr_to_string(base_url),
        cstr_to_string(deployment),
    ) {
        (Some(k), Some(b), Some(d)) => Some((k, b, d)),
        _ => None,
    };
    let Some((api_key, base_url, deployment)) = parsed else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = AzureConfig::new()
        .with_api_key(api_key)
        .with_base_url(base_url);
    if let Some(version) = parse_base_url(api_version) {
        config = config.with_api_version(version);
    }
    match AzureProvider::new(config) {
        Ok(p) => match p.language_model(&deployment) {
            Ok(m) => intern_model(Arc::from(m)),
            Err(e) => unsafe { fail_ai(err, &e) },
        },
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create a Bedrock model instance (AWS SigV4 credentials).
///
/// `access_key_id` / `secret_access_key` / `region` are required.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_bedrock_new(
    access_key_id: *const c_char,
    secret_access_key: *const c_char,
    region: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((access_key_id, secret_access_key, region, model_id)) =
        (unsafe { parse_four_args(access_key_id, secret_access_key, region, model_id) })
    else {
        return unsafe { fail_invalid_args(err) };
    };
    match BedrockProvider::new(BedrockProviderConfig::new(
        access_key_id,
        secret_access_key,
        region,
    ))
    .language_model(&model_id)
    {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create a Bedrock model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_bedrock_new_with_base(
    access_key_id: *const c_char,
    secret_access_key: *const c_char,
    region: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((access_key_id, secret_access_key, region, model_id)) =
        (unsafe { parse_four_args(access_key_id, secret_access_key, region, model_id) })
    else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = BedrockProviderConfig::new(access_key_id, secret_access_key, region);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match BedrockProvider::new(config).language_model(&model_id) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create a Vertex AI model instance (GCP bearer token).
///
/// `access_token` / `project` / `location` are required.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_vertex_new(
    access_token: *const c_char,
    project: *const c_char,
    location: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((access_token, project, location, model_id)) =
        (unsafe { parse_four_args(access_token, project, location, model_id) })
    else {
        return unsafe { fail_invalid_args(err) };
    };
    match VertexProvider::new(VertexProviderConfig::new(access_token, project, location))
        .language_model(&model_id)
    {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create a Vertex AI model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_vertex_new_with_base(
    access_token: *const c_char,
    project: *const c_char,
    location: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((access_token, project, location, model_id)) =
        (unsafe { parse_four_args(access_token, project, location, model_id) })
    else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = VertexProviderConfig::new(access_token, project, location);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match VertexProvider::new(config).language_model(&model_id) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create a Cohere model instance.
///
/// Returns a non-zero handle on success, or 0 on failure filling `*err` (null
/// arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_new(
    api_key: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    match CohereProvider::new(CohereConfig::new(api_key)).language_model(&model_id) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create a Cohere model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = CohereConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match CohereProvider::new(config).language_model(&model_id) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create a Mistral model instance.
///
/// Returns a non-zero handle on success, or 0 on failure filling `*err` (null
/// arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_mistral_new(
    api_key: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    match MistralProvider::new(MistralConfig::new(api_key)).language_model(&model_id) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create a Mistral model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_mistral_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = MistralConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match MistralProvider::new(config).language_model(&model_id) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create an xAI model instance.
///
/// Returns a non-zero handle on success, or 0 on failure filling `*err` (null
/// arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_xai_new(
    api_key: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    match XAIProvider::new(XAIConfig::new(api_key)).language_model(&model_id) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Create an xAI model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_xai_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = XAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match XAIProvider::new(config).language_model(&model_id) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
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
/// Returns a non-zero handle on success, or 0 on failure filling `*err` (unknown
/// provider, bad config, missing env key, or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_new(
    name: *const c_char,
    api_key: *const c_char,
    model_id: *const c_char,
    config_json: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some(name) = cstr_to_string(name) else {
        return unsafe { fail_invalid_args(err) };
    };
    let Some(model_id) = cstr_to_string(model_id) else {
        return unsafe { fail_invalid_args(err) };
    };
    let key = cstr_to_string(api_key); // None => env var from registry entry
    let opts = match parse_provider_options(config_json) {
        Ok(o) => o,
        Err(e) => return unsafe { fail_code(err, e) },
    };
    match provider(&name, key, &model_id, opts) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Convenience: create a language model by provider name, reading the API key
/// from the provider's env var.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_from_env(
    name: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some(name) = cstr_to_string(name) else {
        return unsafe { fail_invalid_args(err) };
    };
    let Some(model_id) = cstr_to_string(model_id) else {
        return unsafe { fail_invalid_args(err) };
    };
    match provider(&name, None, &model_id, None) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
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
/// `aimux_provider_new`). Returns a non-zero handle on success, or 0 filling `*err`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_handle_new(
    name: *const c_char,
    api_key: *const c_char,
    config_json: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some(name) = cstr_to_string(name) else {
        return unsafe { fail_invalid_args(err) };
    };
    let key = cstr_to_string(api_key);
    let opts = match parse_provider_options(config_json) {
        Ok(o) => o,
        Err(e) => return unsafe { fail_code(err, e) },
    };
    match provider_handle(&name, key, opts) {
        Ok(p) => intern_provider(Arc::from(p)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// List models on a provider handle (RFC-0027 runtime discovery).
///
/// `handle` is from `aimux_provider_handle_new`. Returns a JSON array of
/// sparse `RuntimeModel` (id / owned_by / created) — **no community
/// enrichment**. To supplement with model specs, call `aimux_get_model_specs`
/// separately and merge in the host. Returns NULL on failure (fills `*err`).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_list_models(handle: u64, err: *mut CAimuxError) -> *mut c_char {
    let Some(p) = get_provider(handle) else {
        return unsafe { fail_invalid_handle(err, "provider") };
    };
    match ffi_block_on(p.list_models()) {
        Err(e) => unsafe { fail_ai(err, &e) },
        Ok(Err(e)) => unsafe { fail_ai(err, &e) },
        Ok(Ok(models)) => match serde_json::to_string(&models) {
            Ok(s) => into_cstring_raw(s),
            Err(e) => unsafe { fail_other(err, format!("serialize list_models: {e}")) },
        },
    }
}

/// Build a language model from a provider handle + model_id (RFC-0027).
///
/// `handle` is from `aimux_provider_handle_new`. Returns a JSON model handle
/// a non-zero handle (same as `aimux_provider_new`), usable with
/// `aimux_generate_text` etc.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_model(
    handle: u64,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some(p) = get_provider(handle) else {
        return unsafe { fail_invalid_handle(err, "provider") };
    };
    let Some(model_id) = cstr_to_string(model_id) else {
        return unsafe { fail_invalid_args(err) };
    };
    match p.language_model(&model_id) {
        Ok(m) => intern_model(Arc::from(m)),
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: model specs (RFC-0027) — get_model_specs
// ─────────────────────────────────────────────────────────────────────────────

/// Fetch the community model catalogue (anya2a). Returns a JSON-serialized
/// `Catalogue` (provider → model_id → ModelSpec), or NULL on failure.
///
/// `source_url` is an optional URL override (null = default anya2a endpoint).
/// This is a **thin fetch** — no caching, no FS writes. The host decides how
/// to cache/persist the result.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_get_model_specs(
    source_url: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let url = cstr_to_string(source_url);
    match ffi_block_on(aimux_providers::get_model_specs(url.as_deref())) {
        Err(e) => unsafe { fail_ai(err, &e) },
        Ok(Err(e)) => unsafe { fail_ai(err, &e) },
        Ok(Ok(cat)) => match serde_json::to_string(&cat) {
            Ok(s) => into_cstring_raw(s),
            Err(e) => unsafe { fail_other(err, format!("serialize catalogue: {e}")) },
        },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: non-streaming generation
// ─────────────────────────────────────────────────────────────────────────────

/// Non-streaming generation.
///
/// `prompt_json` is either a bare prompt value (`"text"` or a messages array)
/// or `{"prompt": <value>}`. `opts_json` is a serialized `GenerateTextOptions`
/// (empty / null for defaults).
///
/// Returns a JSON string — the serialized `GenerateTextResult`, or
/// NULL on failure (fills `*err`) — success JSON the caller MUST free with
/// [`aimux_free_string`]. Returns a null pointer only if allocation fails.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_generate_text(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let model = match get_model(handle) {
        Some(m) => m,
        None => return unsafe { fail_invalid_handle(err, "model") },
    };
    let prompt = match cstr_to_string(prompt_json) {
        Some(s) => match parse_prompt(&s) {
            Ok(p) => p,
            Err(e) => return unsafe { fail_json(err, format!("invalid prompt_json: {e}")) },
        },
        None => {
            return unsafe { fail(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, "invalid prompt_json") };
        }
    };
    let opts = match cstr_to_string(opts_json) {
        Some(s) => match parse_opts(&s) {
            Ok(o) => o,
            Err(e) => return unsafe { fail_json(err, format!("invalid opts_json: {e}")) },
        },
        None => GenerateTextOptions::default(),
    };
    run_and_serialize(
        err,
        async move { generate_text(&*model, prompt, opts).await },
    )
}

/// Generate a structured JSON object from the model (M12, RFC-0016).
///
/// Same signature as [`aimux_generate_text`]; returns a JSON string — the
/// serialized `GenerateObjectResult`, or NULL on failure (fills `*err`).
/// The caller passes `response_format: { "Json": { ... } }` via `opts_json`
/// to control the schema; the function applies JSON repair before parsing.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_generate_object(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let model = match get_model(handle) {
        Some(m) => m,
        None => return unsafe { fail_invalid_handle(err, "model") },
    };
    let prompt = match cstr_to_string(prompt_json) {
        Some(s) => match parse_prompt(&s) {
            Ok(p) => p,
            Err(e) => return unsafe { fail_json(err, format!("invalid prompt_json: {e}")) },
        },
        None => {
            return unsafe { fail(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, "invalid prompt_json") };
        }
    };
    let opts = match cstr_to_string(opts_json) {
        Some(s) => match parse_opts(&s) {
            Ok(o) => o,
            Err(e) => return unsafe { fail_json(err, format!("invalid opts_json: {e}")) },
        },
        None => GenerateTextOptions::default(),
    };
    run_and_serialize(
        err,
        async move { generate_object(&*model, prompt, opts).await },
    )
}

/// Consume a stream to completion and return the aggregated result (M11,
/// RFC-0016). Synchronous — blocks until the stream finishes.
///
/// Same signature as [`aimux_generate_text`]; returns a JSON string — the
/// serialized `StreamTextResultAggregated`, or NULL on failure (fills `*err`).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_consume_stream_text(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let model = match get_model(handle) {
        Some(m) => m,
        None => return unsafe { fail_invalid_handle(err, "model") },
    };
    let prompt = match cstr_to_string(prompt_json) {
        Some(s) => match parse_prompt(&s) {
            Ok(p) => p,
            Err(e) => return unsafe { fail_json(err, format!("invalid prompt_json: {e}")) },
        },
        None => {
            return unsafe { fail(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, "invalid prompt_json") };
        }
    };
    let opts = match cstr_to_string(opts_json) {
        Some(s) => match parse_opts(&s) {
            Ok(o) => o,
            Err(e) => return unsafe { fail_json(err, format!("invalid opts_json: {e}")) },
        },
        None => GenerateTextOptions::default(),
    };
    run_and_serialize(err, async move {
        let stream_result = stream_text(&*model, prompt, opts).await?;
        stream_result.consume().await
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
/// Returns non-zero (e.g. 1) on success; 0 on failure (fills `*err` when non-NULL).
/// No JSON error envelope.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_stream_text(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: extern "C" fn(*const c_char, *mut c_void),
    on_done: extern "C" fn(*mut c_void),
    stream_ctx: *mut c_void,
    err: *mut CAimuxError,
) -> i32 {
    stream_text_with_signal(
        handle,
        prompt_json,
        opts_json,
        on_part,
        on_done,
        stream_ctx,
        err,
        None,
    )
}

/// Create a per-call abort signal for a cancelable FFI operation.
///
/// The caller must release the returned handle with
/// [`aimux_abort_signal_drop`].
#[unsafe(no_mangle)]
pub extern "C" fn aimux_abort_signal_new() -> u64 {
    intern_handle(ModelHandle::Abort(AbortSignal::new()))
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
/// [`aimux_abort_signal_abort`] with `abort_handle` to stop this call.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_stream_text_with_abort(
    handle: u64,
    abort_handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: extern "C" fn(*const c_char, *mut c_void),
    on_done: extern "C" fn(*mut c_void),
    stream_ctx: *mut c_void,
    err: *mut CAimuxError,
) -> i32 {
    let abort_signal = match get_abort_signal(abort_handle) {
        Some(signal) => signal,
        None => {
            return unsafe { fail_invalid_handle(err, "abort") };
        }
    };
    stream_text_with_signal(
        handle,
        prompt_json,
        opts_json,
        on_part,
        on_done,
        stream_ctx,
        err,
        Some(abort_signal),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: OpenAI-compatible output (RFC-0026)
// ─────────────────────────────────────────────────────────────────────────────

/// Non-streaming text generation with OpenAI Chat Completions output.
///
/// Identical to [`aimux_generate_text`] except the returned JSON string is a
/// serialized `ChatCompletion` (OpenAI `chat.completion` object) rather than a
/// `GenerateTextResult`. Works with any provider — the result is always
/// standard OpenAI format.
///
/// Returns a JSON string — the serialized `ChatCompletion`, or
/// NULL on failure (fills `*err`) — success JSON the caller MUST free with
/// [`aimux_free_string`].
#[unsafe(no_mangle)]
pub extern "C" fn aimux_generate_text_as_openai(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let model = match get_model(handle) {
        Some(m) => m,
        None => return unsafe { fail_invalid_handle(err, "model") },
    };
    let prompt = match cstr_to_string(prompt_json) {
        Some(s) => match parse_prompt(&s) {
            Ok(p) => p,
            Err(e) => return unsafe { fail_json(err, format!("invalid prompt_json: {e}")) },
        },
        None => {
            return unsafe { fail(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, "invalid prompt_json") };
        }
    };
    let opts = match cstr_to_string(opts_json) {
        Some(s) => match parse_opts(&s) {
            Ok(o) => o,
            Err(e) => return unsafe { fail_json(err, format!("invalid opts_json: {e}")) },
        },
        None => GenerateTextOptions::default(),
    };
    run_and_serialize(err, async move {
        generate_text_as_openai(&*model, prompt, opts).await
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
/// failures return 0 and fill `*err` when non-NULL (same polarity as
/// [`aimux_stream_text`]).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_stream_text_as_openai(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: extern "C" fn(*const c_char, *mut c_void),
    on_done: extern "C" fn(*mut c_void),
    stream_ctx: *mut c_void,
    err: *mut CAimuxError,
) -> i32 {
    stream_text_as_openai_with_signal(
        handle,
        prompt_json,
        opts_json,
        on_part,
        on_done,
        stream_ctx,
        err,
        None,
    )
}

/// Cancelable streaming OpenAI-compatible output (see
/// [`aimux_stream_text_with_abort`]).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_stream_text_as_openai_with_abort(
    handle: u64,
    abort_handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: extern "C" fn(*const c_char, *mut c_void),
    on_done: extern "C" fn(*mut c_void),
    stream_ctx: *mut c_void,
    err: *mut CAimuxError,
) -> i32 {
    let abort_signal = match get_abort_signal(abort_handle) {
        Some(signal) => signal,
        None => {
            return unsafe { fail_invalid_handle(err, "abort") };
        }
    };
    stream_text_as_openai_with_signal(
        handle,
        prompt_json,
        opts_json,
        on_part,
        on_done,
        stream_ctx,
        err,
        Some(abort_signal),
    )
}

#[allow(clippy::too_many_arguments)]
fn stream_text_as_openai_with_signal(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: extern "C" fn(*const c_char, *mut c_void),
    on_done: extern "C" fn(*mut c_void),
    stream_ctx: *mut c_void,
    err: *mut CAimuxError,
    abort_signal: Option<AbortSignal>,
) -> i32 {
    let model = match get_model(handle) {
        Some(m) => m,
        None => return unsafe { fail_invalid_handle(err, "model") },
    };

    let prompt = match cstr_to_string(prompt_json) {
        Some(s) => match parse_prompt(&s) {
            Ok(p) => p,
            Err(e) => {
                return unsafe { fail_json(err, format!("invalid prompt_json: {e}")) };
            }
        },
        None => {
            return unsafe { fail(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, "invalid prompt_json") };
        }
    };

    let mut opts = match cstr_to_string(opts_json) {
        Some(s) => match parse_opts(&s) {
            Ok(o) => o,
            Err(e) => {
                return unsafe { fail_json(err, format!("invalid opts_json: {e}")) };
            }
        },
        None => GenerateTextOptions::default(),
    };

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
    // stream_ctx is only for C callbacks; not Send across the async boundary.
    let stream_ctx = stream_ctx as usize;

    let outcome = ffi_block_on(async move {
        let stream_result = match abort_signal.as_ref() {
            Some(signal) => {
                tokio::select! {
                    biased;
                    _ = signal.cancelled() => Err(AiMuxError::Aborted),
                    result = stream_text_as_openai(&*model, prompt, opts, stream_options) => result,
                }
            }
            None => stream_text_as_openai(&*model, prompt, opts, stream_options).await,
        };
        match stream_result {
            Ok(sr) => {
                let mut stream = sr.stream;
                loop {
                    let next = match abort_signal.as_ref() {
                        Some(signal) => {
                            tokio::select! {
                                biased;
                                _ = signal.cancelled() => {
                                    return Err(AiMuxError::Aborted);
                                }
                                item = stream.next() => item,
                            }
                        }
                        None => stream.next().await,
                    };
                    let Some(item) = next else {
                        break;
                    };
                    match item {
                        Ok(chunk) => {
                            let json =
                                serde_json::to_string(&chunk).unwrap_or_else(|_| "{}".to_string());
                            if let Ok(cstr) = CString::new(json) {
                                invoke_stream_callback("on_part", || {
                                    on_part(cstr.as_ptr(), stream_ctx as *mut c_void);
                                })?;
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
                invoke_stream_callback("on_done", || {
                    on_done(stream_ctx as *mut c_void);
                })?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    });
    match outcome {
        Ok(Ok(())) => 1,
        Ok(Err(e)) => unsafe { fail_ai(err, &e) },
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

#[allow(clippy::too_many_arguments)]
fn stream_text_with_signal(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: extern "C" fn(*const c_char, *mut c_void),
    on_done: extern "C" fn(*mut c_void),
    stream_ctx: *mut c_void,
    err: *mut CAimuxError,
    abort_signal: Option<AbortSignal>,
) -> i32 {
    let model = match get_model(handle) {
        Some(m) => m,
        None => return unsafe { fail_invalid_handle(err, "model") },
    };

    let prompt = match cstr_to_string(prompt_json) {
        Some(s) => match parse_prompt(&s) {
            Ok(p) => p,
            Err(e) => {
                return unsafe { fail_json(err, format!("invalid prompt_json: {e}")) };
            }
        },
        None => {
            return unsafe { fail(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, "invalid prompt_json") };
        }
    };

    let mut opts = match cstr_to_string(opts_json) {
        Some(s) => match parse_opts(&s) {
            Ok(o) => o,
            Err(e) => {
                return unsafe { fail_json(err, format!("invalid opts_json: {e}")) };
            }
        },
        None => GenerateTextOptions::default(),
    };
    opts.abort_signal = abort_signal.clone();
    let stream_ctx = stream_ctx as usize;

    let outcome = ffi_block_on(async move {
        let stream_result = match abort_signal.as_ref() {
            Some(signal) => {
                tokio::select! {
                    biased;
                    _ = signal.cancelled() => Err(AiMuxError::Aborted),
                    result = stream_text(&*model, prompt, opts) => result,
                }
            }
            None => stream_text(&*model, prompt, opts).await,
        };
        match stream_result {
            Ok(sr) => {
                let mut stream = sr.stream;
                loop {
                    let next = match abort_signal.as_ref() {
                        Some(signal) => {
                            tokio::select! {
                                biased;
                                _ = signal.cancelled() => {
                                    return Err(AiMuxError::Aborted);
                                }
                                item = stream.next() => item,
                            }
                        }
                        None => stream.next().await,
                    };
                    let Some(item) = next else {
                        break;
                    };
                    match item {
                        Ok(part) => {
                            let json =
                                serde_json::to_string(&part).unwrap_or_else(|_| "{}".to_string());
                            if let Ok(cstr) = CString::new(json) {
                                invoke_stream_callback("on_part", || {
                                    on_part(cstr.as_ptr(), stream_ctx as *mut c_void);
                                })?;
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
                invoke_stream_callback("on_done", || {
                    on_done(stream_ctx as *mut c_void);
                })?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    });
    match outcome {
        Ok(Ok(())) => 1,
        Ok(Err(e)) => unsafe { fail_ai(err, &e) },
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: resource management
// ─────────────────────────────────────────────────────────────────────────────

/// Release a model handle previously returned by `aimux_*_new`.
///
/// Safe to call with `0` (no-op).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_drop_handle(handle: u64) {
    if handle != 0 {
        drop_handle(handle);
    }
}

/// Free a C string previously returned by any aimux function that returns
/// `*mut c_char` — constructors, [`aimux_generate_text`], multimodal calls.
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
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).embedding_model(&model_id);
    intern_handle(ModelHandle::Embedding(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_embedding_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = OpenAIProvider::new(config).embedding_model(&model_id);
    intern_handle(ModelHandle::Embedding(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_embedding_new(
    api_key: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let model = CohereProvider::new(CohereConfig::new(api_key)).embedding_model(&model_id);
    intern_handle(ModelHandle::Embedding(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_embedding_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = CohereConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = CohereProvider::new(config).embedding_model(&model_id);
    intern_handle(ModelHandle::Embedding(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_embedding_new(
    api_key: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let model = GoogleProvider::new(GoogleConfig::new(api_key)).embedding_model(&model_id);
    intern_handle(ModelHandle::Embedding(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_embedding_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = GoogleConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = GoogleProvider::new(config).embedding_model(&model_id);
    intern_handle(ModelHandle::Embedding(Arc::new(model)))
}

/// Generate embeddings. `values_json` is a JSON array of strings.
/// Returns EmbeddingResult JSON (caller must free).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_embed(
    handle: u64,
    values_json: *const c_char,
    opts_json: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Embedding(m)) => m,
        _ => return unsafe { fail_invalid_handle(err, "embedding") },
    };
    let values_json = match cstr_to_string(values_json) {
        Some(s) => s,
        None => {
            return unsafe { fail(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, "invalid values_json") };
        }
    };
    let mut opts = aimux_core::embedding_model::EmbeddingCallOptions::new("");
    if let Some(s) = cstr_to_string(opts_json)
        && !s.trim().is_empty()
        && s.trim() != "null"
    {
        match serde_json::from_str::<aimux_core::embedding_model::EmbeddingCallOptions>(&s) {
            Ok(o) => opts = o,
            Err(e) => return unsafe { fail_json(err, format!("invalid opts: {e}")) },
        }
    }
    let values: Vec<String> = match serde_json::from_str(&values_json) {
        Ok(v) => v,
        Err(e) => return unsafe { fail_json(err, format!("invalid values: {e}")) },
    };
    opts.values = values;
    run_and_serialize(err, async move { model.do_embed(&opts).await })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Speech (TTS)
// ─────────────────────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_speech_new(
    api_key: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).speech(&model_id);
    intern_handle(ModelHandle::Speech(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_speech_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = OpenAIProvider::new(config).speech(&model_id);
    intern_handle(ModelHandle::Speech(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_speech_generate(
    handle: u64,
    opts_json: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Speech(m)) => m,
        _ => return unsafe { fail_invalid_handle(err, "speech") },
    };
    let opts: aimux_core::speech_model::SpeechCallOptions =
        match parse_json_arg(opts_json, "opts_json") {
            Ok(o) => o,
            Err(e) => return unsafe { fail_code(err, e) },
        };
    run_and_serialize(err, async move { model.do_generate(&opts).await })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Image
// ─────────────────────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_image_new(
    api_key: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).image(&model_id);
    intern_handle(ModelHandle::Image(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_image_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = OpenAIProvider::new(config).image(&model_id);
    intern_handle(ModelHandle::Image(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_image_new(
    api_key: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let model = GoogleProvider::new(GoogleConfig::new(api_key)).image(&model_id);
    intern_handle(ModelHandle::Image(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_image_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = GoogleConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = GoogleProvider::new(config).image(&model_id);
    intern_handle(ModelHandle::Image(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_image_generate(
    handle: u64,
    opts_json: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Image(m)) => m,
        _ => return unsafe { fail_invalid_handle(err, "image") },
    };
    let opts: aimux_core::image_model::ImageCallOptions =
        match parse_json_arg(opts_json, "opts_json") {
            Ok(o) => o,
            Err(e) => return unsafe { fail_code(err, e) },
        };
    run_and_serialize(err, async move { model.do_generate(&opts).await })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Transcription (non-streaming)
// ─────────────────────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_transcription_new(
    api_key: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).transcription(&model_id);
    intern_handle(ModelHandle::Transcription(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_transcription_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = OpenAIProvider::new(config).transcription(&model_id);
    intern_handle(ModelHandle::Transcription(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_transcription_generate(
    handle: u64,
    audio_base64: *const c_char,
    media_type: *const c_char,
    _opts_json: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Transcription(m)) => m,
        _ => return unsafe { fail_invalid_handle(err, "transcription") },
    };
    let audio_base64 = match cstr_to_string(audio_base64) {
        Some(s) => s,
        None => {
            return unsafe {
                fail(
                    err,
                    AIMUX_E_INVALID_ARGUMENT,
                    -1,
                    -1,
                    "invalid audio_base64",
                )
            };
        }
    };
    let media_type = match cstr_to_string(media_type) {
        Some(s) => s,
        None => {
            return unsafe { fail(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, "invalid media_type") };
        }
    };
    let opts = aimux_core::transcription_model::TranscriptionCallOptions::new(
        aimux_core::transcription_model::AudioInput::Base64(audio_base64),
        media_type,
    );
    run_and_serialize(err, async move { model.do_generate(&opts).await })
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
fn get_transcription_session(
    handle: u64,
    err: *mut CAimuxError,
) -> Option<Arc<transcription_session::TranscriptionFfiSession>> {
    match get_handle(handle) {
        Some(ModelHandle::TranscriptionSession(s)) => Some(s),
        _ => {
            // Statement position: annotate the sentinel return type.
            let _: i32 = unsafe { fail_invalid_handle(err, "transcription session") };
            None
        }
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
///
/// Returns a non-zero session handle, or 0 with `*err` filled.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_transcription_session_new(
    model_handle: u64,
    abort_handle: u64,
    opts_json: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let model = match get_handle(model_handle) {
        Some(ModelHandle::Transcription(m)) => m,
        _ => return unsafe { fail_invalid_handle(err, "transcription") },
    };
    let abort = if abort_handle == 0 {
        None
    } else {
        match get_handle(abort_handle) {
            Some(ModelHandle::Abort(signal)) => Some(signal),
            _ => return unsafe { fail_invalid_handle(err, "abort signal") },
        }
    };
    // NULL / empty / "null" = defaults (shared convention).
    let Some(opts_json) = normalize_config_json(opts_json, err) else {
        return 0;
    };
    let ffi_opts: TranscriptionSessionFfiOptions = match serde_json::from_str(&opts_json) {
        Ok(o) => o,
        Err(e) => return unsafe { fail_json(err, format!("invalid opts_json: {e}")) },
    };
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
    intern_handle(ModelHandle::TranscriptionSession(session))
}

/// Push one binary audio chunk into the session. **Blocking**: waits while
/// the internal channel is full (backpressure propagation; the caller's
/// capture loop throttles). Must not be called from within an aimux callback
/// (re-entrancy guard rejects it).
///
/// `data` may be NULL only when `len == 0` (no-op). Returns 1 on success, 0
/// with `*err` filled (session ended / input already finished / invalid args).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_transcription_push_audio(
    session: u64,
    data: *const u8,
    len: usize,
    err: *mut CAimuxError,
) -> i32 {
    let Some(session) = get_transcription_session(session, err) else {
        return 0;
    };
    if data.is_null() {
        if len == 0 {
            return 1; // no-op
        }
        return unsafe { fail_invalid_args(err) };
    }
    // SAFETY: caller guarantees `data` points to `len` valid bytes; the copy
    // happens synchronously before any await.
    let bytes = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
    match ffi_block_on(session.push_audio(bytes)) {
        Ok(Ok(())) => 1,
        Ok(Err(e)) | Err(e) => unsafe {
            let _: i32 = fail_ai(err, &e);
            0
        },
    }
}

/// Signal end-of-audio (the audio stream ends; the provider flushes). Safe to
/// call multiple times. Returns 1 on success, 0 with `*err` filled when the
/// session handle is invalid.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_transcription_input_done(session: u64, err: *mut CAimuxError) -> i32 {
    let Some(session) = get_transcription_session(session, err) else {
        return 0;
    };
    session.input_done();
    1
}

/// Pull the next transcription part (JSON-serialized `TranscriptionStreamPart`;
/// free with `aimux_free_string`).
///
/// `timeout_ms`: >0 = wait at most that long; 0 = immediate poll; <0 = wait
/// indefinitely. Returns NULL in three distinguishable cases — the caller
/// MUST pass a non-NULL `err`:
/// - `err.code == AIMUX_E_TIMEOUT` — no part in time; the session is still
///   live, call again.
/// - `err.code == AIMUX_OK` (untouched on success by convention; callers
///   following clear-then-call see 0) — the stream ended normally (a
///   `Finish` part was delivered earlier).
/// - any other error code — the stream failed (abort / API error), details
///   in `err`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_transcription_next_part(
    session: u64,
    timeout_ms: i64,
    err: *mut CAimuxError,
) -> *mut c_char {
    let Some(session) = get_transcription_session(session, err) else {
        return std::ptr::null_mut();
    };
    let timeout = if timeout_ms < 0 {
        None
    } else {
        Some(std::time::Duration::from_millis(timeout_ms as u64))
    };
    match ffi_block_on(session.next_part(timeout)) {
        // A part arrived.
        Ok(Ok(Some(Ok(part)))) => match serde_json::to_string(&part) {
            Ok(json) => into_cstring_raw(json),
            Err(e) => unsafe { fail_json(err, format!("serialize transcription part: {e}")) },
        },
        // The part stream itself errored (abort / API error) — final error.
        Ok(Ok(Some(Err(e)))) | Ok(Err(e)) | Err(e) => unsafe {
            let _: *mut c_char = fail_ai(err, &e);
            std::ptr::null_mut()
        },
        // Channel closed: normal end. NULL with err left untouched (OK by
        // the clear-then-call convention).
        Ok(Ok(None)) => std::ptr::null_mut(),
    }
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
    if let Some(ModelHandle::TranscriptionSession(s)) = removed {
        s.terminate();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Files
// ─────────────────────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_files_new(api_key: *const c_char, err: *mut CAimuxError) -> u64 {
    let Some(api_key) = cstr_to_string(api_key) else {
        return unsafe { fail_invalid_args(err) };
    };
    let files = OpenAIProvider::new(OpenAIConfig::new(api_key)).files();
    intern_handle(ModelHandle::Files(Arc::new(files)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_files_new_with_base(
    api_key: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some(api_key) = cstr_to_string(api_key) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let files = OpenAIProvider::new(config).files();
    intern_handle(ModelHandle::Files(Arc::new(files)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_file_upload(
    handle: u64,
    data_base64: *const c_char,
    media_type: *const c_char,
    _opts_json: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Files(m)) => m,
        _ => return unsafe { fail_invalid_handle(err, "files") },
    };
    let data_base64 = match cstr_to_string(data_base64) {
        Some(s) => s,
        None => {
            return unsafe { fail(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, "invalid data_base64") };
        }
    };
    let media_type = match cstr_to_string(media_type) {
        Some(s) => s,
        None => {
            return unsafe { fail(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, "invalid media_type") };
        }
    };
    let opts = aimux_core::files_model::UploadFileCallOptions::new(
        aimux_core::files_model::UploadFileData::Data {
            data: aimux_core::shared::FileBytes::Base64(data_base64),
        },
        media_type,
    );
    run_and_serialize(err, async move { model.upload_file(&opts).await })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Reranking
// ─────────────────────────────────────────────────────────────────────────────

/// Create a Cohere reranking model instance.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_reranking_new(
    api_key: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let model = CohereProvider::new(CohereConfig::new(api_key)).reranking_model(&model_id);
    intern_handle(ModelHandle::Reranking(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_reranking_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = CohereConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = CohereProvider::new(config).reranking_model(&model_id);
    intern_handle(ModelHandle::Reranking(Arc::new(model)))
}

/// Rerank documents. `opts_json` is JSON-serialized `RerankingCallOptions`
/// (must contain `query` and `documents`). Returns `RerankingResult` JSON
/// (caller must free), or NULL on failure (fills `*err` when non-NULL).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_rerank(
    handle: u64,
    opts_json: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Reranking(m)) => m,
        _ => return unsafe { fail_invalid_handle(err, "reranking") },
    };
    let opts: aimux_core::reranking_model::RerankingCallOptions =
        match parse_json_arg(opts_json, "opts_json") {
            Ok(o) => o,
            Err(e) => return unsafe { fail_code(err, e) },
        };
    run_and_serialize(err, async move { model.do_rerank(&opts).await })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Video
// ─────────────────────────────────────────────────────────────────────────────

/// Create a Google video model instance.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_video_new(
    api_key: *const c_char,
    model_id: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let model = GoogleProvider::new(GoogleConfig::new(api_key)).video(&model_id);
    intern_handle(ModelHandle::Video(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_video_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = GoogleConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = GoogleProvider::new(config).video(&model_id);
    intern_handle(ModelHandle::Video(Arc::new(model)))
}

/// Generate video. `opts_json` is JSON-serialized `VideoCallOptions`
/// (must contain `prompt`). Returns `VideoResult` JSON (caller must free),
/// or NULL on failure (fills `*err` when non-NULL).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_video_generate(
    handle: u64,
    opts_json: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Video(m)) => m,
        _ => return unsafe { fail_invalid_handle(err, "video") },
    };
    let opts: aimux_core::video_model::VideoCallOptions =
        match parse_json_arg(opts_json, "opts_json") {
            Ok(o) => o,
            Err(e) => return unsafe { fail_code(err, e) },
        };
    run_and_serialize(err, async move { model.do_generate(&opts).await })
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
    err: *mut CAimuxError,
) -> u64 {
    let Some(api_key) = cstr_to_string(api_key) else {
        return unsafe { fail_invalid_args(err) };
    };
    let model = TavilyProvider::new(TavilyConfig::new(api_key)).search_model();
    intern_handle(ModelHandle::Search(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_tavily_search_new_with_base(
    api_key: *const c_char,
    _model_id: *const c_char,
    base_url: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some(api_key) = cstr_to_string(api_key) else {
        return unsafe { fail_invalid_args(err) };
    };
    let mut config = TavilyConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = TavilyProvider::new(config).search_model();
    intern_handle(ModelHandle::Search(Arc::new(model)))
}

/// Execute a search. `opts_json` is JSON-serialized `SearchCallOptions`
/// (must contain `query`). Returns `SearchResult` JSON (caller must free),
/// or NULL on failure (fills `*err` when non-NULL).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_search(
    handle: u64,
    opts_json: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Search(m)) => m,
        _ => return unsafe { fail_invalid_handle(err, "search") },
    };
    let opts: aimux_core::search_model::SearchCallOptions =
        match parse_json_arg(opts_json, "opts_json") {
            Ok(o) => o,
            Err(e) => return unsafe { fail_code(err, e) },
        };
    run_and_serialize(err, async move { model.do_search(&opts).await })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Codex subscription helper (RFC-0018 §3.2)
// ─────────────────────────────────────────────────────────────────────────────

/// Refresh a Codex subscription access token (RFC-0018 §3.2).
///
/// Stateless: performs one OAuth `refresh_token` grant against
/// `auth.openai.com/oauth/token`. Returns
/// `{"access_token","refresh_token","expires_in_secs"}` JSON on success, or
/// the standard error JSON (caller must free with `aimux_free_string`).
/// The caller owns token persistence and the 401 → refresh → retry
/// orchestration — the library never stores credentials.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_codex_refresh(
    refresh_token: *const c_char,
    client_id: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let (Some(refresh_token), Some(client_id)) =
        (cstr_to_string(refresh_token), cstr_to_string(client_id))
    else {
        return unsafe { fail_invalid_args(err) };
    };
    run_and_serialize(err, async move {
        aimux_providers::codex_refresh(&refresh_token, &client_id).await
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
///              ("warn"). `AIMUX_LOG` (RUST_LOG-style) and `AIMUX_LOG_LEVEL`
///              env vars take precedence when set. Logs go to stderr.
/// @return Always 0.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_init_logging(level: *const c_char) -> i32 {
    if level.is_null() {
        aimux_providers::init_logging("warn");
        return 0;
    }
    match cstr_to_string(level) {
        Some(level) => aimux_providers::init_logging(&level),
        None => aimux_providers::init_logging("warn"),
    }
    0
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: session grouping (RFC-0024)
// ─────────────────────────────────────────────────────────────────────────────

/// Register the global session store (RFC-0024). Replaces any previous one.
/// Until called, calls are not grouped and the session query functions return
/// empty results. Returns 0.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_session_store_init() -> i32 {
    aimux_core::session::init_session_store(std::sync::Arc::new(
        aimux_core::session::SessionStore::new(),
    ));
    0
}

/// Enable/disable the global session inferer (RFC-0024, opt-in, off by
/// default). Explicit `session_id` values always win regardless.
/// `enabled` nonzero = on. Returns 0.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_session_infer_init(enabled: i32) -> i32 {
    aimux_core::session::init_session_infer(enabled != 0);
    0
}

/// Query: all calls of a session, ordered by step (RFC-0024).
///
/// Returns a JSON string — a serialized `SessionCall[]` (empty array if the
/// session is unknown or no store is registered), or NULL on
/// failure — that the caller MUST free with [`aimux_free_string`]. Returns a
/// null pointer only if allocation fails.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_session_calls(
    session_id: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let Some(id) = cstr_to_string(session_id) else {
        return unsafe { fail(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, "invalid session_id") };
    };
    match serde_json::to_string(&aimux_core::session::session_calls(&id)) {
        Ok(s) => into_cstring_raw(s),
        Err(e) => unsafe { fail_other(err, format!("serialize: {e}")) },
    }
}

/// Query: all known sessions (RFC-0024).
///
/// Returns a JSON string — a serialized `SessionView[]`, or NULL
/// on failure — that the caller MUST free with [`aimux_free_string`]. Returns
/// a null pointer only if allocation fails.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_list_sessions(err: *mut CAimuxError) -> *mut c_char {
    match serde_json::to_string(&aimux_core::session::list_sessions()) {
        Ok(s) => into_cstring_raw(s),
        Err(e) => unsafe { fail_other(err, format!("serialize: {e}")) },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: cache probing (RFC-0015)
// ─────────────────────────────────────────────────────────────────────────────

static TRACE_STORES: OnceLock<Mutex<HashMap<u64, Arc<RingTraceStore>>>> = OnceLock::new();

fn trace_stores() -> &'static Mutex<HashMap<u64, Arc<RingTraceStore>>> {
    TRACE_STORES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_trace_store(handle: u64) -> Option<Arc<RingTraceStore>> {
    trace_stores()
        .lock()
        .expect("aimux-ffi: trace registry mutex poisoned")
        .get(&handle)
        .cloned()
}

/// Wrap a model handle in a probe layer (RFC-0015). Returns a new handle that
/// can be used with `aimux_generate_text` / `aimux_stream_text` (probed) and
/// with the `aimux_trace_*` query functions. Returns a non-zero handle or 0 on failure (fills `*err`); drop with
/// `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_new(handle: u64, err: *mut CAimuxError) -> u64 {
    let Some(model) = get_model(handle) else {
        return unsafe { fail_invalid_handle(err, "model") };
    };
    let store = Arc::new(RingTraceStore::new());
    let layer = Arc::new(TraceLayer::new(model, store.clone()));
    let new_handle = intern_model(layer);
    trace_stores()
        .lock()
        .expect("aimux-ffi: trace registry mutex poisoned")
        .insert(new_handle, store);
    new_handle
}

/// Wrap a model handle in a probe layer WITH the built-in rules auditor
/// (RFC-0015 §4). `strict` nonzero = strict mode (self-hosted single
/// instance); zero = shared mode (safe default). Returns non-zero handle or 0;
/// drop with `aimux_drop_handle`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_new_audited(handle: u64, strict: i32, err: *mut CAimuxError) -> u64 {
    let Some(model) = get_model(handle) else {
        return unsafe { fail_invalid_handle(err, "model") };
    };
    let store = Arc::new(RingTraceStore::new());
    let layer = Arc::new(TraceLayer::new(model, store.clone()).with_rules_auditor(strict != 0));
    let new_handle = intern_model(layer);
    trace_stores()
        .lock()
        .expect("aimux-ffi: trace registry mutex poisoned")
        .insert(new_handle, store);
    new_handle
}

/// Query: aggregated probe statistics, filtered by `filter_json` (a serialized
/// `TraceFilter`; pass `"{}"` for all rows — NULL is rejected as an invalid
/// argument). Returns JSON `TraceStats[]` or NULL on failure; caller frees
/// with `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_aggregate(
    handle: u64,
    filter_json: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let Some(store) = get_trace_store(handle) else {
        return unsafe { fail_invalid_handle(err, "trace") };
    };
    let filter = match parse_json_arg::<TraceFilter>(filter_json, "filter_json") {
        Ok(f) => f,
        Err(e) => return unsafe { fail_code(err, e) },
    };
    match serde_json::to_string(&store.aggregate(&filter)) {
        Ok(s) => into_cstring_raw(s),
        Err(e) => unsafe { fail_other(err, format!("serialize: {e}")) },
    }
}

/// Query: one session's chain view. Returns JSON `SessionChainView` or
/// NULL on failure (e.g. unknown session); caller frees with `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_session_chain(
    handle: u64,
    session_id: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let Some(store) = get_trace_store(handle) else {
        return unsafe { fail_invalid_handle(err, "trace") };
    };
    let Some(id) = cstr_to_string(session_id) else {
        return unsafe { fail(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, "invalid session_id") };
    };
    match store.session_chain(&id) {
        Some(view) => match serde_json::to_string(&view) {
            Ok(s) => into_cstring_raw(s),
            Err(e) => unsafe { fail_other(err, format!("serialize: {e}")) },
        },
        None => unsafe { fail_other(err, "unknown session") },
    }
}

/// Query: one session's per-step cache-hit trajectory (RFC-0024 §4.3).
/// Returns a JSON array of `SessionStepStat` (empty for unknown sessions) or
/// NULL on failure; caller frees success with `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_session_trajectory(
    handle: u64,
    session_id: *const c_char,
    err: *mut CAimuxError,
) -> *mut c_char {
    let Some(store) = get_trace_store(handle) else {
        return unsafe { fail_invalid_handle(err, "trace") };
    };
    let Some(id) = cstr_to_string(session_id) else {
        return unsafe { fail(err, AIMUX_E_INVALID_ARGUMENT, -1, -1, "invalid session_id") };
    };
    match serde_json::to_string(&store.session_cache_trajectory(&id)) {
        Ok(s) => into_cstring_raw(s),
        Err(e) => unsafe { fail_other(err, format!("serialize: {e}")) },
    }
}

/// Export all probe records as JSONL (one `TraceRecord` per line). Returns a
/// JSON string (with embedded newlines) or NULL on failure; caller frees with
/// `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_export_jsonl(handle: u64, err: *mut CAimuxError) -> *mut c_char {
    let Some(store) = get_trace_store(handle) else {
        return unsafe { fail_invalid_handle(err, "trace") };
    };
    let mut buf = Vec::new();
    match store.export_jsonl(&mut buf) {
        Ok(()) => match String::from_utf8(buf) {
            Ok(s) => into_cstring_raw(s),
            Err(e) => unsafe { fail_other(err, format!("utf8: {e}")) },
        },
        Err(e) => unsafe { fail_other(err, format!("export: {e}")) },
    }
}

/// Clear all probe records of a trace handle. Returns 0.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_clear(handle: u64) -> i32 {
    match get_trace_store(handle) {
        Some(store) => {
            store.clear();
            0
        }
        None => -1,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: recording + mock replay (RFC-0023)
// ─────────────────────────────────────────────────────────────────────────────

/// Start recording (RFC-0023 P1/P2): writes complete `Recording` JSONL to
/// `{dir}/recordings.jsonl` (dir auto-created). Recording is **opt-in**.
/// Calling again with a different dir replaces the recorder. Returns 0 on
/// success, -1 on null `dir`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_init_recording(dir: *const c_char) -> i32 {
    let Some(dir) = cstr_to_string(dir) else {
        return -1;
    };
    aimux_core::recording::init_recording(Some(std::sync::Arc::new(
        aimux_core::recording::JsonlRecorder::new(dir),
    )));
    0
}

/// Start in-memory bounded recording (RFC-0023 P6): `RingRecorder` with `cap`
/// entries, FIFO eviction, dropped-count queryable. Returns 0 on success,
/// -1 when `cap == 0`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_init_recording_ring(cap: u64) -> i32 {
    if cap == 0 {
        return -1;
    }
    aimux_core::recording::init_recording(Some(std::sync::Arc::new(
        aimux_core::recording::RingRecorder::with_capacity(cap as usize),
    )));
    0
}

/// No-argument variant of [`aimux_init_recording_ring`]: initialize the global
/// recorder with a `RingRecorder` at the library default capacity (2048
/// entries, [`aimux_core::recording::RingRecorder::default`]). Ordinary callers
/// should prefer this entry point and leave the ring size to the library; pass
/// an explicit `cap` via [`aimux_init_recording_ring`] only when a different
/// size is required. Returns 0 on success.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_init_recording_ring_default() -> i32 {
    aimux_core::recording::init_recording(Some(std::sync::Arc::new(
        aimux_core::recording::RingRecorder::default(),
    )));
    0
}

/// Stop recording: global recorder = None (new calls are unrecorded). Returns 0.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_recording_stop() -> i32 {
    aimux_core::recording::init_recording(None);
    0
}

/// Flush the global recorder (blocks until JSONL is on disk; no-op for the
/// ring recorder). Returns 0.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_recording_flush() -> i32 {
    if let Some(rec) = aimux_core::recording::recorder() {
        rec.flush();
    }
    0
}

/// Flush the global recorder and **report write failures** (see #136).
///
/// Unlike [`aimux_recording_flush`] — which always returns 0 for
/// compatibility — this surface makes the recorder's durability observable
/// across the C ABI:
/// - `AIMUX_OK` — data confirmed on disk (also when recording was never
///   initialized: nothing to flush).
/// - `AIMUX_E_RECORDING_WRITE` — a prior write failed (e.g. ENOSPC); the
///   first error is sticky and every later flush keeps reporting it.
/// - `AIMUX_E_RECORDING_WRITER_GONE` — the writer thread is unavailable
///   (e.g. recording was initialized with an unwritable directory and
///   downgraded to a no-op recorder).
/// - `AIMUX_E_RECORDING_FLUSH_TIMEOUT` — no writer ack within 30s.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_recording_try_flush() -> i32 {
    let Some(rec) = aimux_core::recording::recorder() else {
        return AIMUX_OK;
    };
    use aimux_core::recording::RecordingError;
    match rec.try_flush() {
        Ok(()) => AIMUX_OK,
        Err(RecordingError::Write(_)) => AIMUX_E_RECORDING_WRITE,
        Err(RecordingError::WriterGone) => AIMUX_E_RECORDING_WRITER_GONE,
        Err(RecordingError::FlushTimeout) => AIMUX_E_RECORDING_FLUSH_TIMEOUT,
        Err(_) => AIMUX_E_UNKNOWN,
    }
}

/// Register external OpenAI-compatible providers from a JSON config string
/// (RFC-0020). `config_json` is `{ "providers": [ { "name": ..., "base_url":
/// ..., ... }, ... ] }`. Entries override same-named built-ins or add new
/// ones. Returns 1 on success, 0 on failure (with `err` filled).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_register_providers(
    config_json: *const c_char,
    err: *mut CAimuxError,
) -> i32 {
    let Some(json) = cstr_to_string(config_json) else {
        return unsafe { fail_invalid_args(err) };
    };
    match aimux_providers::load_providers_from_json(&json) {
        Ok(()) => 1,
        Err(e) => unsafe { fail_ai(err, &e) },
    }
}

/// Set the global proxy configuration (M6, RFC-0016). Must be called before
/// the first `aimux_generate_text` / `aimux_stream_text` call; a no-op (returns
/// 1) if the shared HTTP client is already initialised.
///
/// `config_json` is a serialized `ProxyConfig`:
/// `{ "http_url": "...", "https_url": "...", "all_url": "...", "no_proxy":
/// "..." }` (all fields optional; omitting all is equivalent to relying on the
/// `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY` env vars).
///
/// Returns 1 on success (including the already-initialised no-op), 0 on
/// failure (with `err` filled): null/invalid pointer or malformed JSON.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_init_proxy(config_json: *const c_char, err: *mut CAimuxError) -> i32 {
    let Some(json) = cstr_to_string(config_json) else {
        return unsafe { fail_invalid_args(err) };
    };
    let config: aimux_provider_utils::ProxyConfig = match serde_json::from_str(&json) {
        Ok(c) => c,
        Err(e) => return unsafe { fail_json(err, format!("invalid config_json: {e}")) },
    };
    // `init_proxy` returns false when the shared client is already up; treat
    // that as success (idempotent) so callers don't need to reason about
    // ordering races.
    let _ = aimux_provider_utils::init_proxy(config);
    1
}

/// Create a mock replay model from recorded JSONL (RFC-0023 P3). `recordings`
/// is one `Recording` JSON per line. Returns non-zero handle or 0
/// (the handle works with `aimux_generate_text` /
/// `aimux_stream_text`, no real API is sent); caller frees with
/// `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_mock_replay_new(
    recordings_jsonl: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let Some(recordings_jsonl) = cstr_to_string(recordings_jsonl) else {
        return unsafe {
            fail(
                err,
                AIMUX_E_INVALID_ARGUMENT,
                -1,
                -1,
                "invalid recordings_jsonl",
            )
        };
    };
    let mut recordings: Vec<aimux_core::recording::Recording> = Vec::new();
    for (idx, line) in recordings_jsonl.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(r) => recordings.push(r),
            Err(e) => {
                return unsafe { fail_json(err, format!("recordings line {}: {e}", idx + 1)) };
            }
        }
    }
    if recordings.is_empty() {
        return unsafe { fail_other(err, "no recordings") };
    }
    let model = aimux_core::replay::MockReplayModel::new(
        recordings[0].provider.provider.clone(),
        recordings[0].provider.model_id.clone(),
        recordings,
    );
    intern_model(Arc::new(model))
}

/// Create a `RouterModel` (RFC-0021) over the given child-model handles. The
/// returned handle is itself a model handle (works with `aimux_generate_text` /
/// `aimux_stream_text`); free it with `aimux_drop_handle`.
///
/// `handles` is an array of `len` existing model handles (e.g. returned by
/// `aimux_openai_new`). `config_json` selects the router + fallback policy:
/// `{ "router": "rule" | "weighted", "weights": [..], "fallback": "on_error" |
/// "none", "provider_name": "router", "model_id": "router" }`. All keys are
/// optional; defaults are `rule` / `on_error` / `"router"` / `"router"`.
///
/// Returns a non-zero handle on success, or 0 with `err` filled: null/invalid
/// pointer, bad JSON, zero-length `handles`, or any unknown child handle.
/// Child lookup is all-or-nothing; unknown children are never silently
/// dropped.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_router_new(
    handles: *const u64,
    len: usize,
    config_json: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    if handles.is_null() || len == 0 {
        return unsafe { fail_invalid_args(err) };
    }
    let handle_slice = unsafe { std::slice::from_raw_parts(handles, len) };
    let Some(models) = get_models(handle_slice) else {
        return unsafe { fail_other(err, "router: invalid child handle") };
    };
    // NULL / empty / "null" all mean "defaults" (matching parse_provider_options
    // convention). Invalid UTF-8 → invalid args.
    let Some(config_json) = normalize_config_json(config_json, err) else {
        return 0;
    };
    let cfg: RouterFfiConfig = match serde_json::from_str::<RouterFfiConfig>(&config_json) {
        Ok(c) => c,
        Err(e) => return unsafe { fail_json(err, format!("invalid config_json: {e}")) },
    };
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
    intern_model(Arc::new(model))
}

/// Create a `MoaModel` (RFC-0022) over the given reference handles + one
/// aggregator handle. The returned handle is a model handle (works with
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
/// Returns a non-zero handle on success, or 0 with `err` filled: null/invalid
/// pointer, bad JSON, an unknown aggregator handle, or any unknown reference
/// handle (references may be empty; aggregator may not). Model lookup is
/// all-or-nothing.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_moa_new(
    reference_handles: *const u64,
    ref_len: usize,
    aggregator: u64,
    config_json: *const c_char,
    err: *mut CAimuxError,
) -> u64 {
    let reference_slice: &[u64] = if ref_len == 0 {
        &[]
    } else if reference_handles.is_null() {
        return unsafe { fail_invalid_args(err) };
    } else {
        unsafe { std::slice::from_raw_parts(reference_handles, ref_len) }
    };
    let mut handles = Vec::with_capacity(ref_len + 1);
    handles.extend_from_slice(reference_slice);
    handles.push(aggregator);
    let Some(mut models) = get_models(&handles) else {
        return unsafe { fail_other(err, "moa: invalid reference or aggregator handle") };
    };
    let aggregator_model = models.pop().expect("aggregator handle is always present");
    let references = models;
    // NULL / empty / "null" all mean "defaults" (matching parse_provider_options
    // convention). Invalid UTF-8 → invalid args.
    let Some(config_json) = normalize_config_json(config_json, err) else {
        return 0;
    };
    let cfg: aimux_core::moa::MoaConfig = match serde_json::from_str(&config_json) {
        Ok(c) => c,
        Err(e) => return unsafe { fail_json(err, format!("invalid config_json: {e}")) },
    };
    let model = aimux_core::moa::MoaModel::new(references, aggregator_model, cfg);
    intern_model(Arc::new(model))
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

    use super::*;

    #[test]
    fn c_error_layout_is_40_bytes() {
        assert_eq!(std::mem::size_of::<CAimuxError>(), 40);
    }

    /// Pin the full 13-variant → code mapping and the status/retry derivation.
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
        for (e, code) in &cases {
            assert_eq!(aimux_error_code(e), *code, "variant {e:?}");
        }

        // fill_from_aimux derives status/retry_ms/message from the same source.
        let mut c = CAimuxError {
            code: AIMUX_OK,
            status: 0,
            retry_ms: 0,
            message: std::ptr::null_mut(),
            error_value: std::ptr::null_mut(),
            reserved: [std::ptr::null_mut(); 1],
        };
        // A 429 is an ApiCall error; code is AIMUX_E_API_CALL and the
        // classification/hint cross the ABI as status/retry_ms.
        let rl = AiMuxError::ApiCall(ApiCallError {
            status_code: Some(429),
            message: "slow down".into(),
            retry_after_ms: Some(1500),
            ..Default::default()
        });
        unsafe { fill_from_aimux(&mut c, &rl) };
        assert_eq!(c.code, AIMUX_E_API_CALL);
        assert_eq!(c.status, 429);
        assert_eq!(c.retry_ms, 1500);
        let m = unsafe { CStr::from_ptr(c.message) }.to_str().unwrap();
        assert!(m.contains("slow down"), "{m}");
        // error_value round-trips to the exact source enum value.
        let v = unsafe { CStr::from_ptr(c.error_value) }.to_str().unwrap();
        let back: AiMuxError = serde_json::from_str(v).unwrap();
        assert_eq!(back.retry_after_hint(), Some(1500), "{v}");
        assert_eq!(back.status_code(), Some(429), "{v}");
        unsafe { aimux_free_string(c.message) };
        unsafe { aimux_free_string(c.error_value) };
    }

    /// Interior NUL bytes must not corrupt or truncate the message.
    #[test]
    fn fill_error_sanitizes_interior_nul() {
        let mut c = CAimuxError {
            code: AIMUX_OK,
            status: -1,
            retry_ms: -1,
            message: std::ptr::null_mut(),
            error_value: std::ptr::null_mut(),
            reserved: [std::ptr::null_mut(); 1],
        };
        unsafe { fill_error(&mut c, AIMUX_E_OTHER, -1, -1, "a\0b") };
        let m = unsafe { CStr::from_ptr(c.message) }.to_str().unwrap();
        assert_eq!(m, "a\u{FFFD}b");
        unsafe { aimux_free_string(c.message) };
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
            Err(AiMuxError::Other(msg)) => {
                assert!(
                    msg.contains("on_part"),
                    "message should name the callback: {msg}"
                );
                assert!(
                    msg.contains("callback explosion"),
                    "message should include the panic payload: {msg}"
                );
            }
            other => panic!("expected AiMuxError::Other, got {other:?}"),
        }
    }

    #[test]
    fn stream_callback_catches_string_panic() {
        // A panic with a String payload must also be caught.
        let result = invoke_stream_callback("on_done", || {
            panic!("{}", "dynamic boom".to_string());
        });
        match result {
            Err(AiMuxError::Other(msg)) => {
                assert!(
                    msg.contains("on_done"),
                    "message should name the callback: {msg}"
                );
                assert!(
                    msg.contains("dynamic boom"),
                    "message should include the panic payload: {msg}"
                );
            }
            other => panic!("expected AiMuxError::Other, got {other:?}"),
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
            Err(AiMuxError::Other(msg)) => {
                assert!(
                    msg.contains("on_part"),
                    "message should name the callback: {msg}"
                );
                assert!(
                    msg.contains("<non-string panic>"),
                    "non-string payload should use placeholder: {msg}"
                );
            }
            other => panic!("expected AiMuxError::Other, got {other:?}"),
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
            Err(AiMuxError::Other(msg)) => {
                assert!(
                    msg.contains("on_done"),
                    "message should name the callback: {msg}"
                );
                assert!(
                    msg.contains("done callback failed"),
                    "message should include the panic payload: {msg}"
                );
            }
            other => panic!("expected AiMuxError::Other, got {other:?}"),
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
        let h = aimux_router_new(
            handles.as_ptr(),
            handles.len(),
            std::ptr::null(),
            std::ptr::null_mut(),
        );
        assert!(h != 0, "router handle must be non-zero");
        // Confirm it resolves to a model.
        let model = get_model(h).expect("router handle resolves");
        assert_eq!(model.provider(), "router");
        assert_eq!(model.model_id(), "router");
    }

    fn zero_err() -> CAimuxError {
        CAimuxError {
            code: AIMUX_OK,
            status: 0,
            retry_ms: 0,
            message: std::ptr::null_mut(),
            error_value: std::ptr::null_mut(),
            reserved: [std::ptr::null_mut(); 1],
        }
    }

    #[test]
    fn router_new_rejects_empty_handles() {
        let mut err = zero_err();
        let h = aimux_router_new(std::ptr::null(), 0, std::ptr::null(), &mut err);
        assert_eq!(h, 0);
        assert_eq!(err.code, AIMUX_E_INVALID_ARGUMENT);
    }

    #[test]
    fn router_new_rejects_any_invalid_child() {
        let handles = [mock_handle("mock", "valid", "ok"), 999_999];
        let mut err = zero_err();
        let h = aimux_router_new(handles.as_ptr(), handles.len(), std::ptr::null(), &mut err);
        assert_eq!(h, 0, "router must not silently drop an invalid child");
        assert_eq!(err.code, AIMUX_E_OTHER);
    }

    #[test]
    fn router_new_rejects_bad_json() {
        let handles = [mock_handle("mock", "m", "x")];
        let bad = std::ffi::CString::new("{not json").unwrap();
        let mut err = zero_err();
        let h = aimux_router_new(handles.as_ptr(), handles.len(), bad.as_ptr(), &mut err);
        assert_eq!(h, 0);
        assert_eq!(err.code, AIMUX_E_JSON_PARSE);
    }

    #[test]
    fn router_new_treats_empty_config_as_defaults() {
        // S1 guard: empty string / "null" config must NOT be a JSON_PARSE error
        // (consistent with other config-bearing FFI entry points).
        let handles = [mock_handle("mock", "m", "x")];
        for cfg in ["", "  ", "null"] {
            let c = std::ffi::CString::new(cfg).unwrap();
            let h = aimux_router_new(
                handles.as_ptr(),
                handles.len(),
                c.as_ptr(),
                std::ptr::null_mut(),
            );
            assert!(
                h != 0,
                "router with config {cfg:?} should succeed (defaults)"
            );
        }
    }

    #[test]
    fn moa_new_builds_moa_model() {
        let refs = [
            mock_handle("mock", "ref-a", "A"),
            mock_handle("mock", "ref-b", "B"),
        ];
        let agg = mock_handle("mock", "aggregator", "agg");
        let h = aimux_moa_new(
            refs.as_ptr(),
            refs.len(),
            agg,
            std::ptr::null(),
            std::ptr::null_mut(),
        );
        assert!(h != 0, "moa handle must be non-zero");
        let model = get_model(h).expect("moa handle resolves");
        assert_eq!(model.provider(), "moa");
        assert_eq!(model.model_id(), "moa");
    }

    #[test]
    fn moa_new_rejects_bad_aggregator() {
        let refs = [mock_handle("mock", "ref-a", "A")];
        let mut err = zero_err();
        // aggregator handle 999999 does not exist.
        let h = aimux_moa_new(
            refs.as_ptr(),
            refs.len(),
            999_999,
            std::ptr::null(),
            &mut err,
        );
        assert_eq!(h, 0);
        assert_eq!(err.code, AIMUX_E_OTHER);
    }

    #[test]
    fn moa_new_rejects_any_invalid_reference() {
        let refs = [mock_handle("mock", "ref-a", "A"), 999_999];
        let agg = mock_handle("mock", "aggregator", "agg");
        let mut err = zero_err();
        let h = aimux_moa_new(refs.as_ptr(), refs.len(), agg, std::ptr::null(), &mut err);
        assert_eq!(h, 0, "MoA must not silently drop an invalid reference");
        assert_eq!(err.code, AIMUX_E_OTHER);
    }

    #[test]
    fn moa_new_allows_zero_references() {
        // 0 references is valid (degrades to aggregator-only).
        let agg = mock_handle("mock", "aggregator", "agg");
        let h = aimux_moa_new(
            std::ptr::null(),
            0,
            agg,
            std::ptr::null(),
            std::ptr::null_mut(),
        );
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
        intern_handle(ModelHandle::Transcription(Arc::new(
            MockStreamingTranscriber,
        )))
    }

    /// Read a returned JSON string and free it.
    fn free_json(ptr: *mut c_char) -> String {
        unsafe {
            let s = std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned();
            crate::aimux_free_string(ptr);
            s
        }
    }

    #[test]
    fn transcription_session_full_lifecycle() {
        let model = mock_transcriber_handle();
        let mut err = zero_err();
        let session = aimux_transcription_session_new(model, 0, std::ptr::null(), &mut err);
        assert!(session != 0, "session should start");

        // Push two chunks + end-of-input.
        let chunk1 = [1u8, 2, 3];
        let chunk2 = [4u8, 5];
        assert_eq!(
            aimux_transcription_push_audio(session, chunk1.as_ptr(), 3, std::ptr::null_mut()),
            1
        );
        assert_eq!(
            aimux_transcription_push_audio(session, chunk2.as_ptr(), 2, std::ptr::null_mut()),
            1
        );
        assert_eq!(
            aimux_transcription_input_done(session, std::ptr::null_mut()),
            1
        );

        // Pull parts: StreamStart, delta("3 "), delta("2 "), Final, Finish,
        // then NULL + OK (ended).
        let mut err = zero_err();
        let p1 = free_json(aimux_transcription_next_part(session, 2_000, &mut err));
        assert!(p1.contains("StreamStart"), "part 1: {p1}");
        let p2 = free_json(aimux_transcription_next_part(session, 2_000, &mut err));
        assert!(p2.contains("3 "), "part 2: {p2}");
        let p3 = free_json(aimux_transcription_next_part(session, 2_000, &mut err));
        assert!(p3.contains("2 "), "part 3: {p3}");
        let p4 = free_json(aimux_transcription_next_part(session, 2_000, &mut err));
        assert!(p4.contains("TranscriptFinal"), "part 4: {p4}");
        let p5 = free_json(aimux_transcription_next_part(session, 2_000, &mut err));
        assert!(p5.contains("Finish"), "part 5: {p5}");
        assert!(p5.contains("3 2 "), "finish text: {p5}");

        // Stream ended: NULL with err untouched (AIMUX_OK).
        let mut err = zero_err();
        let end = aimux_transcription_next_part(session, 2_000, &mut err);
        assert!(end.is_null());
        assert_eq!(err.code, AIMUX_OK, "normal end must read as AIMUX_OK");

        aimux_transcription_session_drop(session);
    }

    #[test]
    fn transcription_session_timeout_is_retryable() {
        let model = mock_transcriber_handle();
        let session =
            aimux_transcription_session_new(model, 0, std::ptr::null(), std::ptr::null_mut());
        // Consume StreamStart, then DON'T push audio: next_part times out
        // (the mock waits for more audio).
        let _ = free_json(aimux_transcription_next_part(
            session,
            2_000,
            std::ptr::null_mut(),
        ));
        let mut err = zero_err();
        let timed_out = aimux_transcription_next_part(session, 50, &mut err);
        assert!(timed_out.is_null());
        assert_eq!(err.code, AIMUX_E_TIMEOUT);
        unsafe {
            crate::aimux_free_string(err.message);
        }

        // Session still alive: push + finish works after the timeout.
        let chunk = [9u8];
        assert_eq!(
            aimux_transcription_push_audio(session, chunk.as_ptr(), 1, std::ptr::null_mut()),
            1
        );
        assert_eq!(
            aimux_transcription_input_done(session, std::ptr::null_mut()),
            1
        );
        let p = free_json(aimux_transcription_next_part(
            session,
            2_000,
            &mut zero_err(),
        ));
        assert!(p.contains("1 "), "post-timeout part: {p}");
        aimux_transcription_session_drop(session);
    }

    #[test]
    fn transcription_session_abort_via_signal() {
        let model = mock_transcriber_handle();
        let abort = crate::aimux_abort_signal_new();
        let session =
            aimux_transcription_session_new(model, abort, std::ptr::null(), std::ptr::null_mut());
        // Drain StreamStart.
        let _ = free_json(aimux_transcription_next_part(
            session,
            2_000,
            std::ptr::null_mut(),
        ));

        // Abort; the mock yields Err(Aborted).
        crate::aimux_abort_signal_abort(abort);
        let mut err = zero_err();
        let p = aimux_transcription_next_part(session, 2_000, &mut err);
        assert!(p.is_null());
        assert_eq!(err.code, AIMUX_E_ABORTED);
        unsafe {
            crate::aimux_free_string(err.message);
        }

        // Push after abort fails (the session's channels are closing).
        let chunk = [1u8];
        let mut err = zero_err();
        let _ = aimux_transcription_push_audio(session, chunk.as_ptr(), 1, &mut err);
        aimux_transcription_session_drop(session);
    }

    #[test]
    fn transcription_session_drop_is_safe_and_idempotent() {
        let model = mock_transcriber_handle();
        let session =
            aimux_transcription_session_new(model, 0, std::ptr::null(), std::ptr::null_mut());
        aimux_transcription_session_drop(session);
        aimux_transcription_session_drop(session); // idempotent
        aimux_transcription_session_drop(0); // safe with 0
        aimux_transcription_session_drop(999_999); // safe with unknown

        // Operations on a dropped session fail with invalid handle.
        let mut err = zero_err();
        assert_eq!(
            aimux_transcription_push_audio(session, std::ptr::null(), 0, &mut err),
            0
        );
        assert_eq!(err.code, AIMUX_E_INVALID_ARGUMENT);
    }

    #[test]
    fn transcription_session_new_rejects_bad_inputs() {
        let model = mock_transcriber_handle();
        // Bad JSON.
        let bad = std::ffi::CString::new("{not json").unwrap();
        let mut err = zero_err();
        assert_eq!(
            aimux_transcription_session_new(model, 0, bad.as_ptr(), &mut err),
            0
        );
        assert_eq!(err.code, AIMUX_E_JSON_PARSE);
        unsafe {
            crate::aimux_free_string(err.message);
        }

        // Non-transcription model handle.
        let lang = mock_handle("mock", "m", "x");
        let mut err = zero_err();
        assert_eq!(
            aimux_transcription_session_new(lang, 0, std::ptr::null(), &mut err),
            0
        );
        assert_eq!(err.code, AIMUX_E_INVALID_ARGUMENT);
        unsafe {
            crate::aimux_free_string(err.message);
        }

        // Bad abort handle.
        let mut err = zero_err();
        assert_eq!(
            aimux_transcription_session_new(model, 999_999, std::ptr::null(), &mut err),
            0
        );
        assert_eq!(err.code, AIMUX_E_INVALID_ARGUMENT);
    }
}
