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
//! the runtime; the thread-local guard in [`ffi_block_on`] does that
//! (issue M7).
#![allow(clippy::not_unsafe_ptr_arg_deref)]
// `extern "C"` entry points dereference raw pointers (`*const c_char`) by
// design: the C ABI contract requires callers to pass valid pointers (see
// memory-ownership docs above), so the functions are safe only on the C side.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::de::DeserializeOwned;

use aimux_core::AiMuxError;
use aimux_core::generate::{
    GenerateTextOptions, generate_text, generate_text_as_openai, stream_text, stream_text_as_openai,
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
    /// Stream callbacks (`on_part`/`on_done`/`on_error`) run synchronously on
    /// the same thread/call-stack that entered the FFI function, so a callback
    /// that calls back into the FFI layer would enter a second `block_on` on
    /// this thread. tokio rejects nested `block_on` with a **panic**; Rust's
    /// non-unwind `extern "C"` ABI terminates the process rather than letting
    /// the panic propagate (and `panic = "abort"` terminates at the panic site).
    /// [`ffi_block_on`] checks this guard and turns that re-entrant call into
    /// an error envelope instead (issue M7).
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

/// Build the error envelope every FFI error path returns:
/// `{"error":"<message>","error_type":"<variant>","status_code":<u16|null>}`.
fn error_json(msg: impl std::fmt::Display, error_type: &str, status_code: Option<u16>) -> String {
    serde_json::json!({
        "error": msg.to_string(),
        "error_type": error_type,
        "status_code": status_code,
    })
    .to_string()
}

/// Build an owned error JSON string for a plain message (`error_type: "Other"`).
fn error_json_raw(msg: impl std::fmt::Display) -> *mut c_char {
    into_cstring_raw(error_json(msg, "Other", None))
}

/// Build an error JSON string from an `AiMuxError`, preserving the variant
/// name and HTTP status code for programmatic use by bindings.
fn error_json_from(err: &AiMuxError) -> *mut c_char {
    into_cstring_raw(error_json(err, err.error_type(), err.status_code()))
}

/// Build the success envelope every constructor returns: `{"handle":<u64>}`.
/// `handle` is always >0 (0 is never interned).
fn handle_json(handle: u64) -> *mut c_char {
    into_cstring_raw(serde_json::json!({ "handle": handle }).to_string())
}

/// Recorded when a constructor's C-string arguments are null or not UTF-8.
const INVALID_ARGS: &str = "missing or invalid required argument(s): null or invalid UTF-8";

/// Error envelope for null / invalid-UTF-8 constructor arguments
/// (`error_type: "InvalidArgument"`).
fn invalid_args_json() -> *mut c_char {
    into_cstring_raw(error_json(INVALID_ARGS, "InvalidArgument", None))
}

/// Invoke the `on_error` callback with an error JSON string.
///
/// The pointer is valid only for the duration of the callback (no leak: the
/// backing `CString` is freed when this function returns).
fn fire_error(on_error: extern "C" fn(*const c_char), msg: impl std::fmt::Display) {
    if let Ok(cstr) = CString::new(error_json(msg, "Other", None)) {
        on_error(cstr.as_ptr());
    }
}

/// Like `fire_error` but preserves the `AiMuxError` variant name and status code.
fn fire_error_struct(on_error: extern "C" fn(*const c_char), err: &AiMuxError) {
    if let Ok(cstr) = CString::new(error_json(err, err.error_type(), err.status_code())) {
        on_error(cstr.as_ptr());
    }
}

/// 从两个 C 字符串构造 (key, model_id)，失败返回 None。
///
/// # Safety
///
/// 调用者必须确保 `a` 和 `b` 要么是 null，要么指向有效的以 NUL 结尾的 C 字符串。
unsafe fn parse_two_args(a: *const c_char, b: *const c_char) -> Option<(String, String)> {
    match (cstr_to_string(a), cstr_to_string(b)) {
        (Some(k), Some(m)) => Some((k, m)),
        _ => None,
    }
}

/// 解析四个 C 字符串参数；任一为 null 则整体失败。
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

/// 执行一个 async 操作并返回 JSON 字符串（caller 必须 free）。
fn run_and_serialize<F, T>(_model_msg: &str, f: F) -> *mut c_char
where
    F: std::future::Future<Output = Result<T, AiMuxError>>,
    T: serde::Serialize,
{
    let result = ffi_block_on(f).and_then(|inner| inner);
    match result {
        Ok(r) => serde_json::to_string(&r)
            .map(into_cstring_raw)
            .unwrap_or_else(|e| error_json_raw(format!("serialize: {e}"))),
        Err(e) => error_json_from(&e),
    }
}

/// 解析 base_url 参数，空字符串视为未设置。
fn parse_base_url(base_url: *const c_char) -> Option<String> {
    cstr_to_string(base_url).filter(|url| !url.is_empty())
}

/// 解析 JSON C 字符串参数为类型 `T`，失败返回错误 JSON `*mut c_char`。
fn parse_json_arg<T: DeserializeOwned>(json: *const c_char, name: &str) -> Result<T, *mut c_char> {
    let s = match cstr_to_string(json) {
        Some(s) => s,
        None => return Err(error_json_raw(format!("invalid {name}"))),
    };
    match serde_json::from_str::<T>(&s) {
        Ok(v) => Ok(v),
        Err(e) => Err(error_json_raw(format!("invalid {name}: {e}"))),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: provider constructors
// ─────────────────────────────────────────────────────────────────────────────

/// Create an OpenAI model instance.
///
/// Returns `{"handle":<u64>}` on success, `{"error":...}` on failure (null
/// arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_new(api_key: *const c_char, model_id: *const c_char) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    match OpenAIProvider::new(OpenAIConfig::new(api_key)).language_model(&model_id) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
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
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match OpenAIProvider::new(config).language_model(&model_id) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
    }
}

/// Create an Anthropic model instance.
///
/// Returns `{"handle":<u64>}` on success, `{"error":...}` on failure (null
/// arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_new(
    api_key: *const c_char,
    model_id: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    match AnthropicProvider::new(AnthropicConfig::new(api_key)).language_model(&model_id) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
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
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let mut config = AnthropicConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match AnthropicProvider::new(config).language_model(&model_id) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
    }
}

/// Create an Anthropic-on-AWS model instance (API key + region).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_aws_new(
    api_key: *const c_char,
    region: *const c_char,
    model_id: *const c_char,
) -> *mut c_char {
    let parsed = match (
        cstr_to_string(api_key),
        cstr_to_string(region),
        cstr_to_string(model_id),
    ) {
        (Some(k), Some(r), Some(m)) => Some((k, r, m)),
        _ => None,
    };
    let Some((api_key, region, model_id)) = parsed else {
        return invalid_args_json();
    };
    match AnthropicAwsProvider::new(AnthropicAwsProviderConfig::with_api_key(api_key, region))
        .language_model(&model_id)
    {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
    }
}

/// Create an Anthropic-on-AWS model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_aws_new_with_base(
    api_key: *const c_char,
    region: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let parsed = match (
        cstr_to_string(api_key),
        cstr_to_string(region),
        cstr_to_string(model_id),
    ) {
        (Some(k), Some(r), Some(m)) => Some((k, r, m)),
        _ => None,
    };
    let Some((api_key, region, model_id)) = parsed else {
        return invalid_args_json();
    };
    let mut config = AnthropicAwsProviderConfig::with_api_key(api_key, region);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match AnthropicAwsProvider::new(config).language_model(&model_id) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
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
) -> *mut c_char {
    let parsed = match (
        cstr_to_string(api_key),
        cstr_to_string(resource_name),
        cstr_to_string(deployment),
    ) {
        (Some(k), Some(r), Some(d)) => Some((k, r, d)),
        _ => None,
    };
    let Some((api_key, resource_name, deployment)) = parsed else {
        return invalid_args_json();
    };
    let mut config = AzureConfig::new()
        .with_api_key(api_key)
        .with_resource_name(resource_name);
    if let Some(version) = parse_base_url(api_version) {
        config = config.with_api_version(version);
    }
    match AzureProvider::new(config) {
        Ok(p) => match p.language_model(&deployment) {
            Ok(m) => handle_json(intern_model(Arc::from(m))),
            Err(e) => error_json_from(&e),
        },
        Err(e) => error_json_from(&e),
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
) -> *mut c_char {
    let parsed = match (
        cstr_to_string(api_key),
        cstr_to_string(base_url),
        cstr_to_string(deployment),
    ) {
        (Some(k), Some(b), Some(d)) => Some((k, b, d)),
        _ => None,
    };
    let Some((api_key, base_url, deployment)) = parsed else {
        return invalid_args_json();
    };
    let mut config = AzureConfig::new()
        .with_api_key(api_key)
        .with_base_url(base_url);
    if let Some(version) = parse_base_url(api_version) {
        config = config.with_api_version(version);
    }
    match AzureProvider::new(config) {
        Ok(p) => match p.language_model(&deployment) {
            Ok(m) => handle_json(intern_model(Arc::from(m))),
            Err(e) => error_json_from(&e),
        },
        Err(e) => error_json_from(&e),
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
) -> *mut c_char {
    let Some((access_key_id, secret_access_key, region, model_id)) =
        (unsafe { parse_four_args(access_key_id, secret_access_key, region, model_id) })
    else {
        return invalid_args_json();
    };
    match BedrockProvider::new(BedrockProviderConfig::new(
        access_key_id,
        secret_access_key,
        region,
    ))
    .language_model(&model_id)
    {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
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
) -> *mut c_char {
    let Some((access_key_id, secret_access_key, region, model_id)) =
        (unsafe { parse_four_args(access_key_id, secret_access_key, region, model_id) })
    else {
        return invalid_args_json();
    };
    let mut config = BedrockProviderConfig::new(access_key_id, secret_access_key, region);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match BedrockProvider::new(config).language_model(&model_id) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
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
) -> *mut c_char {
    let Some((access_token, project, location, model_id)) =
        (unsafe { parse_four_args(access_token, project, location, model_id) })
    else {
        return invalid_args_json();
    };
    match VertexProvider::new(VertexProviderConfig::new(access_token, project, location))
        .language_model(&model_id)
    {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
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
) -> *mut c_char {
    let Some((access_token, project, location, model_id)) =
        (unsafe { parse_four_args(access_token, project, location, model_id) })
    else {
        return invalid_args_json();
    };
    let mut config = VertexProviderConfig::new(access_token, project, location);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match VertexProvider::new(config).language_model(&model_id) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
    }
}

/// Create a Cohere model instance.
///
/// Returns `{"handle":<u64>}` on success, `{"error":...}` on failure (null
/// arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_new(api_key: *const c_char, model_id: *const c_char) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    match CohereProvider::new(CohereConfig::new(api_key)).language_model(&model_id) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
    }
}

/// Create a Cohere model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let mut config = CohereConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match CohereProvider::new(config).language_model(&model_id) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
    }
}

/// Create a Mistral model instance.
///
/// Returns `{"handle":<u64>}` on success, `{"error":...}` on failure (null
/// arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_mistral_new(
    api_key: *const c_char,
    model_id: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    match MistralProvider::new(MistralConfig::new(api_key)).language_model(&model_id) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
    }
}

/// Create a Mistral model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_mistral_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let mut config = MistralConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match MistralProvider::new(config).language_model(&model_id) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
    }
}

/// Create an xAI model instance.
///
/// Returns `{"handle":<u64>}` on success, `{"error":...}` on failure (null
/// arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_xai_new(api_key: *const c_char, model_id: *const c_char) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    match XAIProvider::new(XAIConfig::new(api_key)).language_model(&model_id) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
    }
}

/// Create an xAI model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_xai_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let mut config = XAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    match XAIProvider::new(config).language_model(&model_id) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
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
/// Returns `{"handle":<u64>}` on success, `{"error":...}` on failure (unknown
/// provider, bad config, missing env key, or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_new(
    name: *const c_char,
    api_key: *const c_char,
    model_id: *const c_char,
    config_json: *const c_char,
) -> *mut c_char {
    let Some(name) = cstr_to_string(name) else {
        return invalid_args_json();
    };
    let Some(model_id) = cstr_to_string(model_id) else {
        return invalid_args_json();
    };
    let key = cstr_to_string(api_key); // None => env var from registry entry
    let opts = match cstr_to_string(config_json) {
        Some(s) if !s.trim().is_empty() && s.trim() != "null" => {
            match serde_json::from_str::<ProviderOptions>(&s) {
                Ok(o) => Some(o),
                Err(e) => {
                    return into_cstring_raw(error_json(
                        format!("invalid config_json: {e}"),
                        "Json",
                        None,
                    ));
                }
            }
        }
        _ => None,
    };
    match provider(&name, key, &model_id, opts) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
    }
}

/// Convenience: create a language model by provider name, reading the API key
/// from the provider's env var.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_from_env(
    name: *const c_char,
    model_id: *const c_char,
) -> *mut c_char {
    let Some(name) = cstr_to_string(name) else {
        return invalid_args_json();
    };
    let Some(model_id) = cstr_to_string(model_id) else {
        return invalid_args_json();
    };
    match provider(&name, None, &model_id, None) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
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
/// `aimux_provider_new`). Returns a JSON handle `{"handle": <u64>}`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_handle_new(
    name: *const c_char,
    api_key: *const c_char,
    config_json: *const c_char,
) -> *mut c_char {
    let Some(name) = cstr_to_string(name) else {
        return invalid_args_json();
    };
    let key = cstr_to_string(api_key);
    let opts = match cstr_to_string(config_json) {
        Some(s) if !s.trim().is_empty() && s.trim() != "null" => {
            match serde_json::from_str::<ProviderOptions>(&s) {
                Ok(o) => Some(o),
                Err(e) => {
                    return into_cstring_raw(error_json(
                        format!("invalid config_json: {e}"),
                        "Json",
                        None,
                    ));
                }
            }
        }
        _ => None,
    };
    match provider_handle(&name, key, opts) {
        Ok(p) => handle_json(intern_provider(Arc::from(p))),
        Err(e) => error_json_from(&e),
    }
}

/// List models on a provider handle (RFC-0027 runtime discovery).
///
/// `handle` is from `aimux_provider_handle_new`. Returns a JSON array of
/// `ResolvedModel` (id + optional spec), or `{"error":"..."}` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_list_models(handle: u64) -> *mut c_char {
    let Some(p) = get_provider(handle) else {
        return into_cstring_raw(error_json(
            "invalid or expired provider handle".to_string(),
            "InvalidHandle",
            None,
        ));
    };
    // Block on the async list_models via a transient runtime. The FFI is
    // sync-returning; the binding layer (napi/PyO3/etc.) bridges async.
    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => tokio::runtime::Runtime::new()
            .map_err(|e| {
                into_cstring_raw(error_json(
                    format!("cannot create tokio runtime: {e}"),
                    "Runtime",
                    None,
                ))
            })
            .expect("aimux-ffi: cannot create tokio runtime")
            .handle()
            .clone(),
    };
    match rt.block_on(p.list_models()) {
        Ok(models) => {
            let json = serde_json::to_string(&models)
                .unwrap_or_else(|e| format!(r#"{{"error":"serialize list_models: {e}"}}"#));
            into_cstring_raw(json)
        }
        Err(e) => error_json_from(&e),
    }
}

/// Build a language model from a provider handle + model_id (RFC-0027).
///
/// `handle` is from `aimux_provider_handle_new`. Returns a JSON model handle
/// `{"handle": <u64>}` (same as `aimux_provider_new`), usable with
/// `aimux_generate_text` etc.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_model(handle: u64, model_id: *const c_char) -> *mut c_char {
    let Some(p) = get_provider(handle) else {
        return into_cstring_raw(error_json(
            "invalid or expired provider handle".to_string(),
            "InvalidHandle",
            None,
        ));
    };
    let Some(model_id) = cstr_to_string(model_id) else {
        return invalid_args_json();
    };
    match p.language_model(&model_id) {
        Ok(m) => handle_json(intern_model(Arc::from(m))),
        Err(e) => error_json_from(&e),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: model specs (RFC-0027) — get_model_specs
// ─────────────────────────────────────────────────────────────────────────────

/// Fetch the community model catalogue (anya2a). Returns a JSON-serialized
/// `Catalogue` (provider → model_id → ModelSpec), or `{"error":"..."}`.
///
/// `source_url` is an optional URL override (null = default anya2a endpoint).
/// This is a **thin fetch** — no caching, no FS writes. The host decides how
/// to cache/persist the result.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_get_model_specs(source_url: *const c_char) -> *mut c_char {
    let url = cstr_to_string(source_url);
    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => tokio::runtime::Runtime::new()
            .expect("aimux-ffi: cannot create tokio runtime")
            .handle()
            .clone(),
    };
    match rt.block_on(aimux_providers::get_model_specs(url.as_deref())) {
        Ok(cat) => {
            let json = serde_json::to_string(&cat)
                .unwrap_or_else(|e| format!(r#"{{"error":"serialize catalogue: {e}"}}"#));
            into_cstring_raw(json)
        }
        Err(e) => error_json_from(&e),
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
/// `{"error":"..."}` on failure — that the caller MUST free with
/// [`aimux_free_string`]. Returns a null pointer only if allocation fails.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_generate_text(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
) -> *mut c_char {
    let model = match get_model(handle) {
        Some(m) => m,
        None => return error_json_raw("invalid handle"),
    };
    let prompt = match cstr_to_string(prompt_json) {
        Some(s) => match parse_prompt(&s) {
            Ok(p) => p,
            Err(e) => return error_json_raw(format!("invalid prompt_json: {e}")),
        },
        None => return error_json_raw("invalid prompt_json"),
    };
    let opts = match cstr_to_string(opts_json) {
        Some(s) => match parse_opts(&s) {
            Ok(o) => o,
            Err(e) => return error_json_raw(format!("invalid opts_json: {e}")),
        },
        None => GenerateTextOptions::default(),
    };
    run_and_serialize("generate_text", async move {
        generate_text(&*model, prompt, opts).await
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: streaming generation (push callbacks)
// ─────────────────────────────────────────────────────────────────────────────

/// Streaming generation with push callbacks.
///
/// Blocks the calling thread until the stream completes (synchronous +
/// callback mode). Callbacks are invoked in the same call stack:
/// - `on_part(json)`: each `StreamPart` serialized as JSON. The pointer is
///   valid only during the call.
/// - `on_done()`: invoked once when the stream ends normally.
/// - `on_error(err_json)`: invoked on a stream-level error (an `Err` from the
///   stream, or failure to start streaming). Valid only during the call.
///
/// `StreamPart::Error` (a provider-reported mid-stream error) is delivered via
/// `on_part` like any other part; the C caller may parse it to react.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_stream_text(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: extern "C" fn(*const c_char),
    on_done: extern "C" fn(),
    on_error: extern "C" fn(*const c_char),
) {
    stream_text_with_signal(
        handle,
        prompt_json,
        opts_json,
        on_part,
        on_done,
        on_error,
        None,
    );
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
    on_part: extern "C" fn(*const c_char),
    on_done: extern "C" fn(),
    on_error: extern "C" fn(*const c_char),
) {
    let abort_signal = match get_abort_signal(abort_handle) {
        Some(signal) => signal,
        None => {
            fire_error(on_error, "invalid abort handle");
            return;
        }
    };
    stream_text_with_signal(
        handle,
        prompt_json,
        opts_json,
        on_part,
        on_done,
        on_error,
        Some(abort_signal),
    );
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
/// `{"error":"..."}` on failure — that the caller MUST free with
/// [`aimux_free_string`].
#[unsafe(no_mangle)]
pub extern "C" fn aimux_generate_text_as_openai(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
) -> *mut c_char {
    let model = match get_model(handle) {
        Some(m) => m,
        None => return error_json_raw("invalid handle"),
    };
    let prompt = match cstr_to_string(prompt_json) {
        Some(s) => match parse_prompt(&s) {
            Ok(p) => p,
            Err(e) => return error_json_raw(format!("invalid prompt_json: {e}")),
        },
        None => return error_json_raw("invalid prompt_json"),
    };
    let opts = match cstr_to_string(opts_json) {
        Some(s) => match parse_opts(&s) {
            Ok(o) => o,
            Err(e) => return error_json_raw(format!("invalid opts_json: {e}")),
        },
        None => GenerateTextOptions::default(),
    };
    run_and_serialize("generate_text_as_openai", async move {
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
/// `StreamPart::Error` is mapped to a content delta + finish chunk; stream-level
/// errors are delivered via `on_error` as usual.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_stream_text_as_openai(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: extern "C" fn(*const c_char),
    on_done: extern "C" fn(),
    on_error: extern "C" fn(*const c_char),
) {
    stream_text_as_openai_with_signal(
        handle,
        prompt_json,
        opts_json,
        on_part,
        on_done,
        on_error,
        None,
    );
}

/// Cancelable streaming OpenAI-compatible output (see
/// [`aimux_stream_text_with_abort`]).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_stream_text_as_openai_with_abort(
    handle: u64,
    abort_handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: extern "C" fn(*const c_char),
    on_done: extern "C" fn(),
    on_error: extern "C" fn(*const c_char),
) {
    let abort_signal = match get_abort_signal(abort_handle) {
        Some(signal) => signal,
        None => {
            fire_error(on_error, "invalid abort handle");
            return;
        }
    };
    stream_text_as_openai_with_signal(
        handle,
        prompt_json,
        opts_json,
        on_part,
        on_done,
        on_error,
        Some(abort_signal),
    );
}

#[allow(clippy::too_many_arguments)]
fn stream_text_as_openai_with_signal(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: extern "C" fn(*const c_char),
    on_done: extern "C" fn(),
    on_error: extern "C" fn(*const c_char),
    abort_signal: Option<AbortSignal>,
) {
    let model = match get_model(handle) {
        Some(m) => m,
        None => {
            fire_error(on_error, "invalid handle");
            return;
        }
    };

    let prompt = match cstr_to_string(prompt_json) {
        Some(s) => match parse_prompt(&s) {
            Ok(p) => p,
            Err(e) => {
                fire_error(on_error, format!("invalid prompt_json: {e}"));
                return;
            }
        },
        None => {
            fire_error(on_error, "invalid prompt_json");
            return;
        }
    };

    let mut opts = match cstr_to_string(opts_json) {
        Some(s) => match parse_opts(&s) {
            Ok(o) => o,
            Err(e) => {
                fire_error(on_error, format!("invalid opts_json: {e}"));
                return;
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
                .and_then(|b| b.as_bool())
                .unwrap_or(true),
            include_reasoning: v
                .get("include_reasoning")
                .and_then(|b| b.as_bool())
                .unwrap_or(true),
        })
        .unwrap_or_default();

    opts.abort_signal = abort_signal.clone();

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
                                    fire_error_struct(on_error, &AiMuxError::Aborted);
                                    return;
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
                                on_part(cstr.as_ptr());
                            }
                        }
                        Err(e) => {
                            fire_error_struct(on_error, &e);
                            return;
                        }
                    }
                }
                on_done();
            }
            Err(e) => fire_error_struct(on_error, &e),
        }
    });
    // A re-entrant call (FFI invoked from inside a callback) fails here with
    // an error envelope instead of panicking across the C boundary (M7).
    if let Err(err) = outcome {
        fire_error_struct(on_error, &err);
    }
}

#[allow(clippy::too_many_arguments)]
fn stream_text_with_signal(
    handle: u64,
    prompt_json: *const c_char,
    opts_json: *const c_char,
    on_part: extern "C" fn(*const c_char),
    on_done: extern "C" fn(),
    on_error: extern "C" fn(*const c_char),
    abort_signal: Option<AbortSignal>,
) {
    let model = match get_model(handle) {
        Some(m) => m,
        None => {
            fire_error(on_error, "invalid handle");
            return;
        }
    };

    let prompt = match cstr_to_string(prompt_json) {
        Some(s) => match parse_prompt(&s) {
            Ok(p) => p,
            Err(e) => {
                fire_error(on_error, format!("invalid prompt_json: {e}"));
                return;
            }
        },
        None => {
            fire_error(on_error, "invalid prompt_json");
            return;
        }
    };

    let mut opts = match cstr_to_string(opts_json) {
        Some(s) => match parse_opts(&s) {
            Ok(o) => o,
            Err(e) => {
                fire_error(on_error, format!("invalid opts_json: {e}"));
                return;
            }
        },
        None => GenerateTextOptions::default(),
    };
    opts.abort_signal = abort_signal.clone();

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
                                    fire_error_struct(on_error, &AiMuxError::Aborted);
                                    return;
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
                            // `cstr` lives for this block: pointer is valid
                            // during `on_part`, freed when the block ends.
                            if let Ok(cstr) = CString::new(json) {
                                on_part(cstr.as_ptr());
                            }
                        }
                        Err(e) => {
                            fire_error_struct(on_error, &e);
                            return;
                        }
                    }
                }
                on_done();
            }
            Err(e) => fire_error_struct(on_error, &e),
        }
    });
    // A re-entrant call (FFI invoked from inside a callback) fails here with
    // an error envelope instead of panicking across the C boundary (M7).
    if let Err(err) = outcome {
        fire_error_struct(on_error, &err);
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
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).embedding_model(&model_id);
    handle_json(intern_handle(ModelHandle::Embedding(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_embedding_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = OpenAIProvider::new(config).embedding_model(&model_id);
    handle_json(intern_handle(ModelHandle::Embedding(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_embedding_new(
    api_key: *const c_char,
    model_id: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let model = CohereProvider::new(CohereConfig::new(api_key)).embedding_model(&model_id);
    handle_json(intern_handle(ModelHandle::Embedding(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_embedding_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let mut config = CohereConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = CohereProvider::new(config).embedding_model(&model_id);
    handle_json(intern_handle(ModelHandle::Embedding(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_embedding_new(
    api_key: *const c_char,
    model_id: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let model = GoogleProvider::new(GoogleConfig::new(api_key)).embedding_model(&model_id);
    handle_json(intern_handle(ModelHandle::Embedding(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_embedding_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let mut config = GoogleConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = GoogleProvider::new(config).embedding_model(&model_id);
    handle_json(intern_handle(ModelHandle::Embedding(Arc::new(model))))
}

/// Generate embeddings. `values_json` is a JSON array of strings.
/// Returns EmbeddingResult JSON (caller must free).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_embed(
    handle: u64,
    values_json: *const c_char,
    opts_json: *const c_char,
) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Embedding(m)) => m,
        _ => return error_json_raw("invalid embedding handle"),
    };
    let values_json = match cstr_to_string(values_json) {
        Some(s) => s,
        None => return error_json_raw("invalid values_json"),
    };
    let mut opts = aimux_core::embedding_model::EmbeddingCallOptions::new("");
    if let Some(s) = cstr_to_string(opts_json)
        && !s.trim().is_empty()
        && s.trim() != "null"
    {
        match serde_json::from_str::<aimux_core::embedding_model::EmbeddingCallOptions>(&s) {
            Ok(o) => opts = o,
            Err(e) => return error_json_raw(format!("invalid opts: {e}")),
        }
    }
    let values: Vec<String> = match serde_json::from_str(&values_json) {
        Ok(v) => v,
        Err(e) => return error_json_raw(format!("invalid values: {e}")),
    };
    opts.values = values;
    run_and_serialize("embed", async move { model.do_embed(&opts).await })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Speech (TTS)
// ─────────────────────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_speech_new(
    api_key: *const c_char,
    model_id: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).speech(&model_id);
    handle_json(intern_handle(ModelHandle::Speech(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_speech_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = OpenAIProvider::new(config).speech(&model_id);
    handle_json(intern_handle(ModelHandle::Speech(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_speech_generate(handle: u64, opts_json: *const c_char) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Speech(m)) => m,
        _ => return error_json_raw("invalid speech handle"),
    };
    let opts: aimux_core::speech_model::SpeechCallOptions =
        match parse_json_arg(opts_json, "opts_json") {
            Ok(o) => o,
            Err(e) => return e,
        };
    run_and_serialize("speech", async move { model.do_generate(&opts).await })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Image
// ─────────────────────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_image_new(
    api_key: *const c_char,
    model_id: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).image(&model_id);
    handle_json(intern_handle(ModelHandle::Image(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_image_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = OpenAIProvider::new(config).image(&model_id);
    handle_json(intern_handle(ModelHandle::Image(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_image_new(
    api_key: *const c_char,
    model_id: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let model = GoogleProvider::new(GoogleConfig::new(api_key)).image(&model_id);
    handle_json(intern_handle(ModelHandle::Image(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_image_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let mut config = GoogleConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = GoogleProvider::new(config).image(&model_id);
    handle_json(intern_handle(ModelHandle::Image(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_image_generate(handle: u64, opts_json: *const c_char) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Image(m)) => m,
        _ => return error_json_raw("invalid image handle"),
    };
    let opts: aimux_core::image_model::ImageCallOptions =
        match parse_json_arg(opts_json, "opts_json") {
            Ok(o) => o,
            Err(e) => return e,
        };
    run_and_serialize("image", async move { model.do_generate(&opts).await })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Transcription (non-streaming)
// ─────────────────────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_transcription_new(
    api_key: *const c_char,
    model_id: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).transcription(&model_id);
    handle_json(intern_handle(ModelHandle::Transcription(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_transcription_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = OpenAIProvider::new(config).transcription(&model_id);
    handle_json(intern_handle(ModelHandle::Transcription(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_transcription_generate(
    handle: u64,
    audio_base64: *const c_char,
    media_type: *const c_char,
    _opts_json: *const c_char,
) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Transcription(m)) => m,
        _ => return error_json_raw("invalid transcription handle"),
    };
    let audio_base64 = match cstr_to_string(audio_base64) {
        Some(s) => s,
        None => return error_json_raw("invalid audio_base64"),
    };
    let media_type = match cstr_to_string(media_type) {
        Some(s) => s,
        None => return error_json_raw("invalid media_type"),
    };
    let opts = aimux_core::transcription_model::TranscriptionCallOptions::new(
        aimux_core::transcription_model::AudioInput::Base64(audio_base64),
        media_type,
    );
    run_and_serialize(
        "transcription",
        async move { model.do_generate(&opts).await },
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Files
// ─────────────────────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_files_new(api_key: *const c_char) -> *mut c_char {
    let Some(api_key) = cstr_to_string(api_key) else {
        return invalid_args_json();
    };
    let files = OpenAIProvider::new(OpenAIConfig::new(api_key)).files();
    handle_json(intern_handle(ModelHandle::Files(Arc::new(files))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_files_new_with_base(
    api_key: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let Some(api_key) = cstr_to_string(api_key) else {
        return invalid_args_json();
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let files = OpenAIProvider::new(config).files();
    handle_json(intern_handle(ModelHandle::Files(Arc::new(files))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_file_upload(
    handle: u64,
    data_base64: *const c_char,
    media_type: *const c_char,
    _opts_json: *const c_char,
) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Files(m)) => m,
        _ => return error_json_raw("invalid files handle"),
    };
    let data_base64 = match cstr_to_string(data_base64) {
        Some(s) => s,
        None => return error_json_raw("invalid data_base64"),
    };
    let media_type = match cstr_to_string(media_type) {
        Some(s) => s,
        None => return error_json_raw("invalid media_type"),
    };
    let opts = aimux_core::files_model::UploadFileCallOptions::new(
        aimux_core::files_model::UploadFileData::Data {
            data: aimux_core::shared::FileBytes::Base64(data_base64),
        },
        media_type,
    );
    run_and_serialize("file_upload", async move { model.upload_file(&opts).await })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Reranking
// ─────────────────────────────────────────────────────────────────────────────

/// Create a Cohere reranking model instance.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_reranking_new(
    api_key: *const c_char,
    model_id: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let model = CohereProvider::new(CohereConfig::new(api_key)).reranking_model(&model_id);
    handle_json(intern_handle(ModelHandle::Reranking(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_reranking_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let mut config = CohereConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = CohereProvider::new(config).reranking_model(&model_id);
    handle_json(intern_handle(ModelHandle::Reranking(Arc::new(model))))
}

/// Rerank documents. `opts_json` is JSON-serialized `RerankingCallOptions`
/// (must contain `query` and `documents`). Returns `RerankingResult` JSON
/// (caller must free), or `{"error":"..."}` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_rerank(handle: u64, opts_json: *const c_char) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Reranking(m)) => m,
        _ => return error_json_raw("invalid reranking handle"),
    };
    let opts: aimux_core::reranking_model::RerankingCallOptions =
        match parse_json_arg(opts_json, "opts_json") {
            Ok(o) => o,
            Err(e) => return e,
        };
    run_and_serialize("rerank", async move { model.do_rerank(&opts).await })
}

// ─────────────────────────────────────────────────────────────────────────────
// C ABI: Video
// ─────────────────────────────────────────────────────────────────────────────

/// Create a Google video model instance.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_video_new(
    api_key: *const c_char,
    model_id: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let model = GoogleProvider::new(GoogleConfig::new(api_key)).video(&model_id);
    handle_json(intern_handle(ModelHandle::Video(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_video_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return invalid_args_json();
    };
    let mut config = GoogleConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = GoogleProvider::new(config).video(&model_id);
    handle_json(intern_handle(ModelHandle::Video(Arc::new(model))))
}

/// Generate video. `opts_json` is JSON-serialized `VideoCallOptions`
/// (must contain `prompt`). Returns `VideoResult` JSON (caller must free),
/// or `{"error":"..."}` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_video_generate(handle: u64, opts_json: *const c_char) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Video(m)) => m,
        _ => return error_json_raw("invalid video handle"),
    };
    let opts: aimux_core::video_model::VideoCallOptions =
        match parse_json_arg(opts_json, "opts_json") {
            Ok(o) => o,
            Err(e) => return e,
        };
    run_and_serialize("video", async move { model.do_generate(&opts).await })
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
) -> *mut c_char {
    let Some(api_key) = cstr_to_string(api_key) else {
        return invalid_args_json();
    };
    let model = TavilyProvider::new(TavilyConfig::new(api_key)).search_model();
    handle_json(intern_handle(ModelHandle::Search(Arc::new(model))))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_tavily_search_new_with_base(
    api_key: *const c_char,
    _model_id: *const c_char,
    base_url: *const c_char,
) -> *mut c_char {
    let Some(api_key) = cstr_to_string(api_key) else {
        return invalid_args_json();
    };
    let mut config = TavilyConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = TavilyProvider::new(config).search_model();
    handle_json(intern_handle(ModelHandle::Search(Arc::new(model))))
}

/// Execute a search. `opts_json` is JSON-serialized `SearchCallOptions`
/// (must contain `query`). Returns `SearchResult` JSON (caller must free),
/// or `{"error":"..."}` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_search(handle: u64, opts_json: *const c_char) -> *mut c_char {
    let model = match get_handle(handle) {
        Some(ModelHandle::Search(m)) => m,
        _ => return error_json_raw("invalid search handle"),
    };
    let opts: aimux_core::search_model::SearchCallOptions =
        match parse_json_arg(opts_json, "opts_json") {
            Ok(o) => o,
            Err(e) => return e,
        };
    run_and_serialize("search", async move { model.do_search(&opts).await })
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
) -> *mut c_char {
    let (Some(refresh_token), Some(client_id)) =
        (cstr_to_string(refresh_token), cstr_to_string(client_id))
    else {
        return error_json_raw("invalid arguments");
    };
    run_and_serialize("codex_refresh", async move {
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
/// session is unknown or no store is registered), or `{"error":"..."}` on
/// failure — that the caller MUST free with [`aimux_free_string`]. Returns a
/// null pointer only if allocation fails.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_session_calls(session_id: *const c_char) -> *mut c_char {
    let Some(id) = cstr_to_string(session_id) else {
        return error_json_raw("invalid session_id");
    };
    match serde_json::to_string(&aimux_core::session::session_calls(&id)) {
        Ok(s) => into_cstring_raw(s),
        Err(e) => error_json_raw(format!("serialize: {e}")),
    }
}

/// Query: all known sessions (RFC-0024).
///
/// Returns a JSON string — a serialized `SessionView[]`, or `{"error":"..."}`
/// on failure — that the caller MUST free with [`aimux_free_string`]. Returns
/// a null pointer only if allocation fails.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_list_sessions() -> *mut c_char {
    match serde_json::to_string(&aimux_core::session::list_sessions()) {
        Ok(s) => into_cstring_raw(s),
        Err(e) => error_json_raw(format!("serialize: {e}")),
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
/// with the `aimux_trace_*` query functions. Returns `{"handle":<u64>}` or
/// `{"error":...}` (null args / invalid handle); caller frees with
/// `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_new(handle: u64) -> *mut c_char {
    let Some(model) = get_model(handle) else {
        return error_json_raw("invalid handle");
    };
    let store = Arc::new(RingTraceStore::new());
    let layer = Arc::new(TraceLayer::new(model, store.clone()));
    let new_handle = intern_model(layer);
    trace_stores()
        .lock()
        .expect("aimux-ffi: trace registry mutex poisoned")
        .insert(new_handle, store);
    handle_json(new_handle)
}

/// Wrap a model handle in a probe layer WITH the built-in rules auditor
/// (RFC-0015 §4). `strict` nonzero = strict mode (self-hosted single
/// instance); zero = shared mode (safe default). Returns `{"handle":<u64>}`
/// or `{"error":...}`; caller frees with `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_new_audited(handle: u64, strict: i32) -> *mut c_char {
    let Some(model) = get_model(handle) else {
        return error_json_raw("invalid handle");
    };
    let store = Arc::new(RingTraceStore::new());
    let layer = Arc::new(TraceLayer::new(model, store.clone()).with_rules_auditor(strict != 0));
    let new_handle = intern_model(layer);
    trace_stores()
        .lock()
        .expect("aimux-ffi: trace registry mutex poisoned")
        .insert(new_handle, store);
    handle_json(new_handle)
}

/// Query: aggregated probe statistics, filtered by `filter_json` (a serialized
/// `TraceFilter`, NULL = all). Returns JSON `TraceStats[]` or `{"error":...}`;
/// caller frees with `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_aggregate(handle: u64, filter_json: *const c_char) -> *mut c_char {
    let Some(store) = get_trace_store(handle) else {
        return error_json_raw("invalid trace handle");
    };
    let filter = match parse_json_arg::<TraceFilter>(filter_json, "filter_json") {
        Ok(f) => f,
        Err(e) => return e,
    };
    match serde_json::to_string(&store.aggregate(&filter)) {
        Ok(s) => into_cstring_raw(s),
        Err(e) => error_json_raw(format!("serialize: {e}")),
    }
}

/// Query: one session's chain view. Returns JSON `SessionChainView` or
/// `{"error":...}` (unknown session); caller frees with `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_session_chain(handle: u64, session_id: *const c_char) -> *mut c_char {
    let Some(store) = get_trace_store(handle) else {
        return error_json_raw("invalid trace handle");
    };
    let Some(id) = cstr_to_string(session_id) else {
        return error_json_raw("invalid session_id");
    };
    match store.session_chain(&id) {
        Some(view) => match serde_json::to_string(&view) {
            Ok(s) => into_cstring_raw(s),
            Err(e) => error_json_raw(format!("serialize: {e}")),
        },
        None => error_json_raw("unknown session"),
    }
}

/// Query: one session's per-step cache-hit trajectory (RFC-0024 §4.3).
/// Returns a JSON array of `SessionStepStat` (empty for unknown sessions) or
/// `{"error":...}`; caller frees with `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_session_trajectory(
    handle: u64,
    session_id: *const c_char,
) -> *mut c_char {
    let Some(store) = get_trace_store(handle) else {
        return error_json_raw("invalid trace handle");
    };
    let Some(id) = cstr_to_string(session_id) else {
        return error_json_raw("invalid session_id");
    };
    match serde_json::to_string(&store.session_cache_trajectory(&id)) {
        Ok(s) => into_cstring_raw(s),
        Err(e) => error_json_raw(format!("serialize: {e}")),
    }
}

/// Export all probe records as JSONL (one `TraceRecord` per line). Returns a
/// JSON string (with embedded newlines) or `{"error":...}`; caller frees with
/// `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_trace_export_jsonl(handle: u64) -> *mut c_char {
    let Some(store) = get_trace_store(handle) else {
        return error_json_raw("invalid trace handle");
    };
    let mut buf = Vec::new();
    match store.export_jsonl(&mut buf) {
        Ok(()) => match String::from_utf8(buf) {
            Ok(s) => into_cstring_raw(s),
            Err(e) => error_json_raw(format!("utf8: {e}")),
        },
        Err(e) => error_json_raw(format!("export: {e}")),
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

/// Create a mock replay model from recorded JSONL (RFC-0023 P3). `recordings`
/// is one `Recording` JSON per line. Returns `{"handle":<u64>}` or
/// `{"error":...}` (the handle works with `aimux_generate_text` /
/// `aimux_stream_text`, no real API is sent); caller frees with
/// `aimux_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_mock_replay_new(recordings_jsonl: *const c_char) -> *mut c_char {
    let Some(recordings_jsonl) = cstr_to_string(recordings_jsonl) else {
        return error_json_raw("invalid recordings_jsonl");
    };
    let mut recordings: Vec<aimux_core::recording::Recording> = Vec::new();
    for (idx, line) in recordings_jsonl.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(r) => recordings.push(r),
            Err(e) => return error_json_raw(format!("recordings line {}: {e}", idx + 1)),
        }
    }
    if recordings.is_empty() {
        return error_json_raw("no recordings");
    }
    let model = aimux_core::replay::MockReplayModel::new(
        recordings[0].provider.provider.clone(),
        recordings[0].provider.model_id.clone(),
        recordings,
    );
    handle_json(intern_model(Arc::new(model)))
}
