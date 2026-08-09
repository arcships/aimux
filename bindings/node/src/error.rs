//! Map `aimux_core::AiMuxError` into a real JS `Error` with properties.
//!
//! Native sets `name` to the subclass (`AuthenticationError`, …), plus
//! `message` / `status` / `retryMs`. The TS layer (`error.ts`) rehydrates into
//! the matching class so callers use `instanceof`.
//!
//! Construction needs a napi `Env` (available when `ToNapiValue` runs on the
//! JS thread). Async methods therefore return [`AimuxResult`] instead of
//! `napi::Result`, so the error is materialised with `Env` present.

use std::ptr;

use aimux_core::AiMuxError;
use napi::bindgen_prelude::{
    Object, Result as NapiResult, ToNapiValue, TypeName, ValueType, check_status,
};
use napi::{Env, Error, Unknown, sys};

/// Flattened fields extracted from core (and local binding failures).
#[derive(Clone, Debug)]
pub struct MappedError {
    pub code: String,
    pub message: String,
    pub status: i32,
    pub retry_ms: i64,
    /// Lossless externally-tagged serde JSON of the source `AiMuxError`
    /// (e.g. `{"RateLimited":{...}}`); `None` if serialization failed.
    pub error_value: Option<String>,
}

impl std::fmt::Display for MappedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<&AiMuxError> for MappedError {
    fn from(e: &AiMuxError) -> Self {
        Self {
            code: e.error_type().to_string(),
            message: e.to_string(),
            status: e.status_code().map(i32::from).unwrap_or(-1),
            retry_ms: e.retry_after_hint().unwrap_or(-1),
            error_value: serde_json::to_string(e).ok(),
        }
    }
}

/// JS `Error.name` for each core variant (matches `bindings/node/src/error.ts`).
fn error_class_name(code: &str) -> &'static str {
    match code {
        "Provider" => "ProviderError",
        "Http" => "HttpError",
        "Json" => "JsonError",
        "Stream" => "StreamError",
        "Tool" => "ToolError",
        "InvalidArgument" => "InvalidArgumentError",
        "InvalidPrompt" => "InvalidPromptError",
        "RateLimited" => "RateLimitedError",
        "Auth" => "AuthenticationError",
        "TokenExpired" => "TokenExpiredError",
        "ModelNotFound" => "ModelNotFoundError",
        "Unsupported" => "UnsupportedError",
        "NoSuchModel" => "NoSuchModelError",
        "UnknownProvider" => "UnknownProviderError",
        "ApiCall" => "APICallError",
        "Timeout" => "TimeoutError",
        "Aborted" => "RequestAbortedError",
        "Other" => "OtherError",
        _ => "AimuxError",
    }
}

/// Build a JS `Error` with real properties and wrap it as `napi::Error`
/// (preserves the object via `napi_ref`, so throw/reject keep the fields).
pub(crate) fn create_throwable(env: &Env, m: &MappedError) -> NapiResult<Error> {
    let env_raw = env.raw();
    let mut code = ptr::null_mut();
    let mut msg = ptr::null_mut();
    let mut js_error = ptr::null_mut();

    check_status!(
        unsafe {
            sys::napi_create_string_utf8(
                env_raw,
                m.code.as_ptr().cast(),
                m.code.len() as isize,
                &mut code,
            )
        },
        "create error code string failed"
    )?;
    check_status!(
        unsafe {
            sys::napi_create_string_utf8(
                env_raw,
                m.message.as_ptr().cast(),
                m.message.len() as isize,
                &mut msg,
            )
        },
        "create error message string failed"
    )?;
    check_status!(
        unsafe { sys::napi_create_error(env_raw, code, msg, &mut js_error) },
        "napi_create_error failed"
    )?;

    let mut obj = Object::from_raw(env_raw, js_error);
    // Class name for TS rehydrate (`AuthenticationError`, `RateLimitedError`, …).
    obj.set("name", error_class_name(&m.code))?;
    obj.set("status", m.status)?;
    obj.set("retryMs", m.retry_ms)?;
    if let Some(ev) = &m.error_value {
        obj.set("errorValue", ev.as_str())?;
    }
    // `code` is already set by napi_create_error from the first argument (core variant).

    let unknown = unsafe { Unknown::from_raw_unchecked(env_raw, js_error) };
    Ok(Error::from(unknown))
}

/// Fallible return type for napi methods. Converts to a value or rejects/throws
/// a structured `AimuxError` when `Env` is available on the JS thread.
pub struct AimuxResult<T>(pub(crate) std::result::Result<T, MappedError>);

impl<T: ToNapiValue> ToNapiValue for AimuxResult<T> {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> NapiResult<sys::napi_value> {
        match val.0 {
            Ok(v) => unsafe { T::to_napi_value(env, v) },
            Err(m) => {
                let env = Env::from_raw(env);
                Err(create_throwable(&env, &m)?)
            }
        }
    }
}

impl<T: TypeName> TypeName for AimuxResult<T> {
    fn type_name() -> &'static str {
        T::type_name()
    }

    fn value_type() -> ValueType {
        T::value_type()
    }
}

/// Internal result alias used by helpers (`?` with [`MappedError`]).
pub(crate) type MResult<T> = std::result::Result<T, MappedError>;

impl<T> From<MResult<T>> for AimuxResult<T> {
    fn from(r: MResult<T>) -> Self {
        Self(r)
    }
}

/// Item yielded by the stream generator. `AsyncGenerator::next` returns
/// `napi::Result` without an `Env`, so errors ride the channel as typed
/// [`MappedError`]s and convert into a structured throwable here, where
/// `ToNapiValue` runs on the JS thread with an `Env` present.
pub enum StreamItem {
    Json(String),
    Failure(MappedError),
}

impl ToNapiValue for StreamItem {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> NapiResult<sys::napi_value> {
        match val {
            StreamItem::Json(s) => unsafe { String::to_napi_value(env, s) },
            StreamItem::Failure(m) => {
                let env = Env::from_raw(env);
                Err(create_throwable(&env, &m)?)
            }
        }
    }
}

impl TypeName for StreamItem {
    fn type_name() -> &'static str {
        "string"
    }

    fn value_type() -> ValueType {
        ValueType::String
    }
}
