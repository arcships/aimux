//! aimux-ffi: C ABI boundary for multi-language bindings.
//!
//! Provides an opaque handle registry + JSON wire boundary + push callback
//! stream. Only used by C ABI bindings (Swift / Kotlin / C). Native bindings
//! (Python / Node / Flutter) bypass this layer and use `aimux-providers`
//! directly.
//!
//! ## Memory ownership
//!
//! - [`aimux_generate_text`] returns a `*mut c_char` owned by the caller; the
//!   caller MUST free it with [`aimux_free_string`].
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
//! invoked the FFI function, so they must not re-enter the FFI layer (doing so
//! would deadlock the runtime).
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
use aimux_core::generate::{GenerateTextOptions, generate_text, stream_text};
use aimux_core::language_model::LanguageModel;
use aimux_core::message::ModelPrompt;
use aimux_core::provider::Provider;
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
use aimux_providers::{ProviderOptions, provider};

use futures::StreamExt;
use tokio::runtime::Runtime;

// ─────────────────────────────────────────────────────────────────────────────
// Global state: handle registry + tokio runtime
// ─────────────────────────────────────────────────────────────────────────────

/// A type-erased model handle. One registry holds all modalities.
#[derive(Clone)]
enum ModelHandle {
    Language(Arc<dyn LanguageModel>),
    Embedding(Arc<dyn aimux_core::embedding_model::EmbeddingModel>),
    Speech(Arc<dyn aimux_core::speech_model::SpeechModel>),
    Image(Arc<dyn aimux_core::image_model::ImageModel>),
    Transcription(Arc<dyn aimux_core::transcription_model::TranscriptionModel>),
    Reranking(Arc<dyn aimux_core::reranking_model::RerankingModel>),
    Video(Arc<dyn aimux_core::video_model::VideoModel>),
    Search(Arc<dyn aimux_core::search_model::SearchModel>),
    Files(Arc<dyn aimux_core::files_model::Files>),
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

/// Look up any handle (multimodal).
fn get_handle(handle: u64) -> Option<ModelHandle> {
    registry()
        .lock()
        .expect("aimux-ffi: registry mutex poisoned")
        .get(&handle)
        .cloned()
}

/// Remove a handle from the registry (the model drops when the last ref goes).
fn drop_handle(handle: u64) {
    registry()
        .lock()
        .expect("aimux-ffi: registry mutex poisoned")
        .remove(&handle);
}

/// The shared tokio runtime driving all async provider calls.
fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Runtime::new().expect("aimux-ffi: failed to build tokio runtime")
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
fn into_cstring_raw(s: String) -> *mut c_char {
    CString::new(s)
        .map(|c| c.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// Build an error JSON string with error type, message, and optional status code.
///
/// Output: `{"error":"<message>","error_type":"<variant>","status_code":<u16|null>}`
fn error_json_raw(msg: impl std::fmt::Display) -> *mut c_char {
    into_cstring_raw(
        serde_json::json!({
            "error": msg.to_string(),
            "error_type": "Other",
            "status_code": null,
        })
        .to_string(),
    )
}

/// Build an error JSON string from an `AiMuxError`, preserving the variant
/// name and HTTP status code for programmatic use by bindings.
fn error_json_from(err: &AiMuxError) -> *mut c_char {
    into_cstring_raw(
        serde_json::json!({
            "error": err.to_string(),
            "error_type": err.error_type(),
            "status_code": err.status_code(),
        })
        .to_string(),
    )
}

/// Invoke the `on_error` callback with an error JSON string.
///
/// The pointer is valid only for the duration of the callback (no leak: the
/// backing `CString` is freed when this function returns).
fn fire_error(on_error: extern "C" fn(*const c_char), msg: impl std::fmt::Display) {
    let json = serde_json::json!({
        "error": msg.to_string(),
        "error_type": "Other",
        "status_code": null,
    })
    .to_string();
    if let Ok(cstr) = CString::new(json) {
        on_error(cstr.as_ptr());
    }
}

/// Like `fire_error` but preserves the `AiMuxError` variant name and status code.
fn fire_error_struct(on_error: extern "C" fn(*const c_char), err: &AiMuxError) {
    let json = serde_json::json!({
        "error": err.to_string(),
        "error_type": err.error_type(),
        "status_code": err.status_code(),
    })
    .to_string();
    if let Ok(cstr) = CString::new(json) {
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
    let result = runtime().block_on(f);
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

/// Create an OpenAI model instance, returning its opaque handle.
///
/// Returns `0` on failure (null arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_new(api_key: *const c_char, model_id: *const c_char) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    OpenAIProvider::new(OpenAIConfig::new(api_key))
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Create an OpenAI model instance with a custom base URL.
///
/// `base_url` may be null (defaults to the provider's standard URL).
/// Returns `0` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    OpenAIProvider::new(config)
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Create an Anthropic model instance, returning its opaque handle.
///
/// Returns `0` on failure (null arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_new(api_key: *const c_char, model_id: *const c_char) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    AnthropicProvider::new(AnthropicConfig::new(api_key))
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Create an Anthropic model instance with a custom base URL.
///
/// `base_url` may be null (defaults to the provider's standard URL).
/// Returns `0` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let mut config = AnthropicConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    AnthropicProvider::new(config)
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Create an Anthropic-on-AWS model instance (API key + region).
///
/// Returns `0` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_aws_new(
    api_key: *const c_char,
    region: *const c_char,
    model_id: *const c_char,
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
        return 0;
    };
    AnthropicAwsProvider::new(AnthropicAwsProviderConfig::with_api_key(api_key, region))
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Create an Anthropic-on-AWS model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_anthropic_aws_new_with_base(
    api_key: *const c_char,
    region: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
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
        return 0;
    };
    let mut config = AnthropicAwsProviderConfig::with_api_key(api_key, region);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    AnthropicAwsProvider::new(config)
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Create an Azure OpenAI model instance (API key + resource name).
///
/// `api_version` may be null (uses the provider default). The deployment name
/// is passed as `model_id`. Returns `0` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_azure_new(
    api_key: *const c_char,
    resource_name: *const c_char,
    deployment: *const c_char,
    api_version: *const c_char,
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
        return 0;
    };
    let mut config = AzureConfig::new()
        .with_api_key(api_key)
        .with_resource_name(resource_name);
    if let Some(version) = parse_base_url(api_version) {
        config = config.with_api_version(version);
    }
    match AzureProvider::new(config) {
        Ok(p) => p
            .language_model(&deployment)
            .map(|m| intern_model(Arc::from(m)))
            .unwrap_or(0),
        Err(_) => 0,
    }
}

/// Create an Azure OpenAI model instance with a custom base URL.
///
/// `api_version` may be null (uses the provider default). The deployment name
/// is passed as `model_id`. Returns `0` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_azure_new_with_base(
    api_key: *const c_char,
    base_url: *const c_char,
    deployment: *const c_char,
    api_version: *const c_char,
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
        return 0;
    };
    let mut config = AzureConfig::new()
        .with_api_key(api_key)
        .with_base_url(base_url);
    if let Some(version) = parse_base_url(api_version) {
        config = config.with_api_version(version);
    }
    match AzureProvider::new(config) {
        Ok(p) => p
            .language_model(&deployment)
            .map(|m| intern_model(Arc::from(m)))
            .unwrap_or(0),
        Err(_) => 0,
    }
}

/// Create a Bedrock model instance (AWS SigV4 credentials).
///
/// `access_key_id` / `secret_access_key` / `region` are required.
/// Returns `0` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_bedrock_new(
    access_key_id: *const c_char,
    secret_access_key: *const c_char,
    region: *const c_char,
    model_id: *const c_char,
) -> u64 {
    let Some((access_key_id, secret_access_key, region, model_id)) =
        (unsafe { parse_four_args(access_key_id, secret_access_key, region, model_id) })
    else {
        return 0;
    };
    BedrockProvider::new(BedrockProviderConfig::new(
        access_key_id,
        secret_access_key,
        region,
    ))
    .language_model(&model_id)
    .map(|m| intern_model(Arc::from(m)))
    .unwrap_or(0)
}

/// Create a Bedrock model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_bedrock_new_with_base(
    access_key_id: *const c_char,
    secret_access_key: *const c_char,
    region: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((access_key_id, secret_access_key, region, model_id)) =
        (unsafe { parse_four_args(access_key_id, secret_access_key, region, model_id) })
    else {
        return 0;
    };
    let mut config = BedrockProviderConfig::new(access_key_id, secret_access_key, region);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    BedrockProvider::new(config)
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Create a Vertex AI model instance (GCP bearer token).
///
/// `access_token` / `project` / `location` are required.
/// Returns `0` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_vertex_new(
    access_token: *const c_char,
    project: *const c_char,
    location: *const c_char,
    model_id: *const c_char,
) -> u64 {
    let Some((access_token, project, location, model_id)) =
        (unsafe { parse_four_args(access_token, project, location, model_id) })
    else {
        return 0;
    };
    VertexProvider::new(VertexProviderConfig::new(access_token, project, location))
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Create a Vertex AI model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_vertex_new_with_base(
    access_token: *const c_char,
    project: *const c_char,
    location: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((access_token, project, location, model_id)) =
        (unsafe { parse_four_args(access_token, project, location, model_id) })
    else {
        return 0;
    };
    let mut config = VertexProviderConfig::new(access_token, project, location);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    VertexProvider::new(config)
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Create a Cohere model instance, returning its opaque handle.
///
/// Returns `0` on failure (null arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_new(api_key: *const c_char, model_id: *const c_char) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    CohereProvider::new(CohereConfig::new(api_key))
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Create a Cohere model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let mut config = CohereConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    CohereProvider::new(config)
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Create a Mistral model instance, returning its opaque handle.
///
/// Returns `0` on failure (null arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_mistral_new(api_key: *const c_char, model_id: *const c_char) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    MistralProvider::new(MistralConfig::new(api_key))
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Create a Mistral model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_mistral_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let mut config = MistralConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    MistralProvider::new(config)
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Create an xAI model instance, returning its opaque handle.
///
/// Returns `0` on failure (null arguments or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_xai_new(api_key: *const c_char, model_id: *const c_char) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    XAIProvider::new(XAIConfig::new(api_key))
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Create an xAI model instance with a custom base URL.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_xai_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let mut config = XAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    XAIProvider::new(config)
        .language_model(&model_id)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
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
/// Returns the opaque handle (0 on failure: unknown provider, bad config,
/// missing env key, or invalid model id).
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_new(
    name: *const c_char,
    api_key: *const c_char,
    model_id: *const c_char,
    config_json: *const c_char,
) -> u64 {
    let Some(name) = cstr_to_string(name) else {
        return 0;
    };
    let Some(model_id) = cstr_to_string(model_id) else {
        return 0;
    };
    let key = cstr_to_string(api_key); // None => env var from registry entry
    let opts = match cstr_to_string(config_json) {
        Some(s) if !s.trim().is_empty() && s.trim() != "null" => {
            match serde_json::from_str::<ProviderOptions>(&s) {
                Ok(o) => Some(o),
                Err(_) => return 0,
            }
        }
        _ => None,
    };
    provider(&name, key, &model_id, opts)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
}

/// Convenience: create a language model by provider name, reading the API key
/// from the provider's env var. Returns `0` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_provider_from_env(name: *const c_char, model_id: *const c_char) -> u64 {
    let Some(name) = cstr_to_string(name) else {
        return 0;
    };
    let Some(model_id) = cstr_to_string(model_id) else {
        return 0;
    };
    provider(&name, None, &model_id, None)
        .map(|m| intern_model(Arc::from(m)))
        .unwrap_or(0)
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
    let prompt = match cstr_to_string(prompt_json).and_then(|s| parse_prompt(&s).ok()) {
        Some(p) => p,
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
    let model = match get_model(handle) {
        Some(m) => m,
        None => {
            fire_error(on_error, "invalid handle");
            return;
        }
    };

    let prompt = match cstr_to_string(prompt_json).and_then(|s| parse_prompt(&s).ok()) {
        Some(p) => p,
        None => {
            fire_error(on_error, "invalid prompt_json");
            return;
        }
    };

    let opts = match cstr_to_string(opts_json) {
        Some(s) => match parse_opts(&s) {
            Ok(o) => o,
            Err(e) => {
                fire_error(on_error, format!("invalid opts_json: {e}"));
                return;
            }
        },
        None => GenerateTextOptions::default(),
    };

    runtime().block_on(async move {
        let stream_result = stream_text(&*model, prompt, opts).await;
        match stream_result {
            Ok(sr) => {
                let mut stream = sr.stream;
                while let Some(item) = stream.next().await {
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

/// Free a C string previously returned by [`aimux_generate_text`].
///
/// # Safety
///
/// `ptr` must be null or a pointer previously produced by
/// [`aimux_generate_text`] (i.e. via `CString::into_raw`). Passing any other
/// pointer is undefined behavior.
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

/// Create an OpenAI embedding model instance. Returns 0 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_embedding_new(
    api_key: *const c_char,
    model_id: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).embedding_model(&model_id);
    intern_handle(ModelHandle::Embedding(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_embedding_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
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
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let model = CohereProvider::new(CohereConfig::new(api_key)).embedding_model(&model_id);
    intern_handle(ModelHandle::Embedding(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_embedding_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
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
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let model = GoogleProvider::new(GoogleConfig::new(api_key)).embedding_model(&model_id);
    intern_handle(ModelHandle::Embedding(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_embedding_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
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
pub extern "C" fn aimux_openai_speech_new(api_key: *const c_char, model_id: *const c_char) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).speech(&model_id);
    intern_handle(ModelHandle::Speech(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_speech_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = OpenAIProvider::new(config).speech(&model_id);
    intern_handle(ModelHandle::Speech(Arc::new(model)))
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
pub extern "C" fn aimux_openai_image_new(api_key: *const c_char, model_id: *const c_char) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).image(&model_id);
    intern_handle(ModelHandle::Image(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_image_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = OpenAIProvider::new(config).image(&model_id);
    intern_handle(ModelHandle::Image(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_image_new(api_key: *const c_char, model_id: *const c_char) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let model = GoogleProvider::new(GoogleConfig::new(api_key)).image(&model_id);
    intern_handle(ModelHandle::Image(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_image_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let mut config = GoogleConfig::new(api_key);
    if let Some(url) = parse_base_url(base_url) {
        config = config.with_base_url(url);
    }
    let model = GoogleProvider::new(config).image(&model_id);
    intern_handle(ModelHandle::Image(Arc::new(model)))
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
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let model = OpenAIProvider::new(OpenAIConfig::new(api_key)).transcription(&model_id);
    intern_handle(ModelHandle::Transcription(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_transcription_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
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
pub extern "C" fn aimux_openai_files_new(api_key: *const c_char) -> u64 {
    let Some(api_key) = cstr_to_string(api_key) else {
        return 0;
    };
    let files = OpenAIProvider::new(OpenAIConfig::new(api_key)).files();
    intern_handle(ModelHandle::Files(Arc::new(files)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_openai_files_new_with_base(
    api_key: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some(api_key) = cstr_to_string(api_key) else {
        return 0;
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

/// Create a Cohere reranking model instance. Returns 0 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_reranking_new(
    api_key: *const c_char,
    model_id: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let model = CohereProvider::new(CohereConfig::new(api_key)).reranking_model(&model_id);
    intern_handle(ModelHandle::Reranking(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_cohere_reranking_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
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

/// Create a Google video model instance. Returns 0 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_video_new(api_key: *const c_char, model_id: *const c_char) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
    };
    let model = GoogleProvider::new(GoogleConfig::new(api_key)).video(&model_id);
    intern_handle(ModelHandle::Video(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_google_video_new_with_base(
    api_key: *const c_char,
    model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some((api_key, model_id)) = (unsafe { parse_two_args(api_key, model_id) }) else {
        return 0;
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
/// symmetry but ignored (Tavily uses a fixed endpoint). Returns 0 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_tavily_search_new(api_key: *const c_char, _model_id: *const c_char) -> u64 {
    let Some(api_key) = cstr_to_string(api_key) else {
        return 0;
    };
    let model = TavilyProvider::new(TavilyConfig::new(api_key)).search_model();
    intern_handle(ModelHandle::Search(Arc::new(model)))
}

#[unsafe(no_mangle)]
pub extern "C" fn aimux_tavily_search_new_with_base(
    api_key: *const c_char,
    _model_id: *const c_char,
    base_url: *const c_char,
) -> u64 {
    let Some(api_key) = cstr_to_string(api_key) else {
        return 0;
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
