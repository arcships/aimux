//! Map `aimux_core::AiMuxError` into a real JS `Error` with properties.
//!
//! Native sets `name` to the subclass (`APICallError`, …), plus
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
    /// Stored retry verdict (`ApiCallError::is_retryable`).
    pub retryable: bool,
    /// Provider's machine-readable error code (`ApiCallError::provider_code`).
    pub provider_code: Option<String>,
    /// Provider-assigned request id (`ApiCallError::request_id`).
    pub request_id: Option<String>,
    /// Raw response body, verbatim (`ApiCallError::response_body`).
    pub response_body: Option<String>,
    /// Lossless externally-tagged serde JSON of the source `AiMuxError`
    /// (e.g. `{"ApiCall":{...}}`); `None` if serialization failed.
    pub error_value: Option<String>,
}

impl std::fmt::Display for MappedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<&AiMuxError> for MappedError {
    fn from(e: &AiMuxError) -> Self {
        let detail = match e {
            AiMuxError::ApiCall(d) => Some(d),
            _ => None,
        };
        let code = match e {
            AiMuxError::ApiCall(_) => "ApiCall",
            AiMuxError::JsonParse(_) => "JsonParse",
            AiMuxError::InvalidResponseData(_) => "InvalidResponseData",
            AiMuxError::Tool(_) => "Tool",
            AiMuxError::InvalidArgument(_) => "InvalidArgument",
            AiMuxError::InvalidPrompt(_) => "InvalidPrompt",
            AiMuxError::TokenExpired(_) => "TokenExpired",
            AiMuxError::UnsupportedFunctionality(_) => "UnsupportedFunctionality",
            AiMuxError::NoSuchModel { .. } => "NoSuchModel",
            AiMuxError::NoSuchProvider { .. } => "NoSuchProvider",
            AiMuxError::Timeout(_) => "Timeout",
            AiMuxError::Aborted => "Aborted",
            AiMuxError::Other(_) => "Other",
        };
        Self {
            code: code.to_string(),
            message: e.to_string(),
            status: e.status_code().map(i32::from).unwrap_or(-1),
            retry_ms: e.retry_after_hint().unwrap_or(-1),
            retryable: e.is_retryable(),
            provider_code: detail.and_then(|d| d.provider_code.clone()),
            request_id: detail.and_then(|d| d.request_id.clone()),
            response_body: detail.and_then(|d| d.response_body.clone()),
            error_value: serde_json::to_string(e).ok(),
        }
    }
}

/// JS `Error.name` for each core variant (matches `bindings/node/src/error.ts`).
fn error_class_name(code: &str) -> &'static str {
    match code {
        "ApiCall" => "APICallError",
        "JsonParse" => "JSONParseError",
        "InvalidResponseData" => "InvalidResponseDataError",
        "Tool" => "ToolError",
        "InvalidArgument" => "InvalidArgumentError",
        "InvalidPrompt" => "InvalidPromptError",
        "TokenExpired" => "TokenExpiredError",
        "UnsupportedFunctionality" => "UnsupportedFunctionalityError",
        "NoSuchModel" => "NoSuchModelError",
        "NoSuchProvider" => "NoSuchProviderError",
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
    // Class name for TS rehydrate (`APICallError`, `TokenExpiredError`, …).
    obj.set("name", error_class_name(&m.code))?;
    obj.set("status", m.status)?;
    obj.set("retryMs", m.retry_ms)?;
    obj.set("retryable", m.retryable)?;
    if let Some(pc) = &m.provider_code {
        obj.set("providerCode", pc.as_str())?;
    }
    if let Some(rid) = &m.request_id {
        obj.set("requestId", rid.as_str())?;
    }
    if let Some(rb) = &m.response_body {
        obj.set("responseBody", rb.as_str())?;
    }
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
