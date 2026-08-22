//! Map core failures into the JavaScript `Error` classes registered by the
//! package's JS entrypoint.
//!
//! Construction needs a napi `Env` (available when `ToNapiValue` runs on the
//! JS thread). Async methods therefore return [`AimuxResult`] instead of
//! `napi::Result`, so the error is materialised with `Env` present.

use std::collections::HashMap;

use aimux_core::{AiMuxError, recording::RecordingError};
use napi::bindgen_prelude::{
    Function, FunctionRef, JsValue, Object, Result as NapiResult, ToNapiValue, TypeName, ValueType,
};
use napi::{Env, Error, Status, sys};
use napi_derive::napi;

const ERROR_CLASS_NAMES: &[&str] = &[
    "APICallError",
    "JSONParseError",
    "InvalidResponseDataError",
    "ToolError",
    "InvalidArgumentError",
    "InvalidPromptError",
    "TokenExpiredError",
    "UnsupportedFunctionalityError",
    "NoSuchModelError",
    "NoSuchProviderError",
    "TimeoutError",
    "RequestAbortedError",
    "OtherError",
    "RecordingError",
];

type ErrorConstructor = FunctionRef<String, ()>;

struct ErrorConstructors(HashMap<&'static str, ErrorConstructor>);

/// Register the canonical JS error constructors for this Node environment.
///
/// Keeping the constructors in the environment, rather than a process-global,
/// preserves class identity and keeps Worker environments isolated.
#[napi(js_name = "__registerErrorClasses", ts_return_type = "void")]
pub fn register_error_classes(env: Env, constructors: Object) -> NapiResult<()> {
    let mut registered = HashMap::with_capacity(ERROR_CLASS_NAMES.len());
    for &name in ERROR_CLASS_NAMES {
        let constructor = constructors
            .get::<Function<String, ()>>(name)?
            .ok_or_else(|| {
                Error::new(
                    Status::InvalidArg,
                    format!("missing JavaScript error constructor: {name}"),
                )
            })?;
        registered.insert(name, constructor.create_ref()?);
    }

    let registry = ErrorConstructors(registered);
    if let Some(current) = env.get_instance_data::<ErrorConstructors>()? {
        *current = registry;
    } else {
        env.set_instance_data(registry, (), |_| {})?;
    }
    Ok(())
}

/// Failures of the binding's own layer (napi-rs side) — never disguised
/// as an `AiMuxError`. Surfaces in JS as napi-rs's own plain `Error`
/// (`code` = napi status: `InvalidArg` / `GenericFailure`).
#[derive(Clone, Debug)]
pub(crate) enum BindingError {
    /// A JSON *text* the binding transports did not parse (schema violations
    /// of well-formed JSON stay `AiMuxError::InvalidArgument`).
    InvalidWireJson {
        argument: &'static str,
        message: String,
    },
    /// A closed session / dropped handle handed to the binding.
    InvalidHandle { message: &'static str },
    /// The binding could not serialize a result to JSON.
    ResultSerialization { message: String },
    /// A binding invariant broke.
    InvariantViolation { message: String },
}

impl std::fmt::Display for BindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidWireJson { argument, message } => {
                write!(f, "{argument}: invalid JSON: {message}")
            }
            Self::InvalidHandle { message } => f.write_str(message),
            Self::ResultSerialization { message } => write!(f, "serialize result: {message}"),
            Self::InvariantViolation { message } => {
                write!(f, "binding invariant violation: {message}")
            }
        }
    }
}

/// One internal error carrier for Node binding returns.
///
/// The variants remain separate all the way to JS: `AiMux` becomes one of the
/// typed AimuxError subclasses, `Recording` becomes RecordingError, and
/// `Binding` becomes napi-rs's plain Error. This carrier does not merge the
/// public error families.
#[derive(Debug)]
pub(crate) enum AiMuxBindingError {
    AiMux(AiMuxError),
    Recording(RecordingError),
    Binding(BindingError),
}

impl From<AiMuxError> for AiMuxBindingError {
    fn from(error: AiMuxError) -> Self {
        Self::AiMux(error)
    }
}

impl From<&AiMuxError> for AiMuxBindingError {
    fn from(error: &AiMuxError) -> Self {
        Self::AiMux(error.clone())
    }
}

impl From<RecordingError> for AiMuxBindingError {
    fn from(error: RecordingError) -> Self {
        Self::Recording(error)
    }
}

impl From<BindingError> for AiMuxBindingError {
    fn from(error: BindingError) -> Self {
        Self::Binding(error)
    }
}

/// Parse a JSON text the binding transports (`prompt_json`, `opts_json`, …).
/// Malformed text → binding `InvalidWireJson`; well-formed JSON that violates
/// the schema → `AiMuxError::InvalidArgument` (what the core would say).
pub(crate) fn parse_wire_json<T: serde::de::DeserializeOwned>(
    argument: &'static str,
    json: &str,
) -> MResult<T> {
    serde_json::from_str(json).map_err(|e| match e.classify() {
        serde_json::error::Category::Data => AiMuxBindingError::from(AiMuxError::InvalidArgument(
            format!("invalid {argument}: {e}"),
        )),
        _ => BindingError::InvalidWireJson {
            argument,
            message: e.to_string(),
        }
        .into(),
    })
}

/// Serialize a result for the JS side; failure is a binding `ResultSerialization`.
pub(crate) fn serialize_result<T: serde::Serialize>(value: &T) -> MResult<String> {
    serde_json::to_string(value).map_err(|e| {
        BindingError::ResultSerialization {
            message: e.to_string(),
        }
        .into()
    })
}

fn aimux_error_class_name(error: &AiMuxError) -> &'static str {
    match error {
        AiMuxError::ApiCall(_) => "APICallError",
        AiMuxError::JsonParse(_) => "JSONParseError",
        AiMuxError::InvalidResponseData(_) => "InvalidResponseDataError",
        AiMuxError::Tool(_) => "ToolError",
        AiMuxError::InvalidArgument(_) => "InvalidArgumentError",
        AiMuxError::InvalidPrompt(_) => "InvalidPromptError",
        AiMuxError::TokenExpired(_) => "TokenExpiredError",
        AiMuxError::UnsupportedFunctionality(_) => "UnsupportedFunctionalityError",
        AiMuxError::NoSuchModel { .. } => "NoSuchModelError",
        AiMuxError::NoSuchProvider { .. } => "NoSuchProviderError",
        AiMuxError::Timeout(_) => "TimeoutError",
        AiMuxError::Aborted => "RequestAbortedError",
        AiMuxError::Other(_) => "OtherError",
    }
}

/// Convert one internal failure into its public JavaScript error family.
pub(crate) fn create_throwable(env: &Env, error: &AiMuxBindingError) -> NapiResult<Error> {
    match error {
        AiMuxBindingError::AiMux(error) => create_aimux_throwable(env, error),
        AiMuxBindingError::Recording(error) => create_recording_throwable(env, error),
        AiMuxBindingError::Binding(error) => Ok(create_binding_throwable(error)),
    }
}

fn new_registered_error<'env>(
    env: &'env Env,
    class_name: &str,
    message: &str,
) -> NapiResult<Object<'env>> {
    let constructors = env
        .get_instance_data::<ErrorConstructors>()?
        .ok_or_else(|| {
            Error::new(
                Status::GenericFailure,
                "JavaScript error constructors are not registered",
            )
        })?;
    let constructor = constructors.0.get(class_name).ok_or_else(|| {
        Error::new(
            Status::GenericFailure,
            format!("JavaScript error constructor was not registered: {class_name}"),
        )
    })?;
    let instance = constructor
        .borrow_back(env)?
        .new_instance(message.to_owned())?;
    instance.coerce_to_object()
}

/// Build the exact JS subclass registered for this core variant.
fn create_aimux_throwable(env: &Env, error: &AiMuxError) -> NapiResult<Error> {
    let message = error.to_string();
    let mut obj = new_registered_error(env, aimux_error_class_name(error), &message)?;
    if let Some(status) = error.status_code() {
        obj.set("status", i32::from(status))?;
    }
    if let AiMuxError::ApiCall(detail) = error {
        obj.set("retryable", error.is_retryable())?;
        if let Some(retry_ms) = error.retry_after_hint() {
            obj.set("retryMs", retry_ms)?;
        }
        if let Some(provider_code) = &detail.provider_code {
            obj.set("providerCode", provider_code.as_str())?;
        }
        if !detail.message.is_empty() {
            obj.set("providerMessage", detail.message.as_str())?;
        }
        if let Some(request_id) = &detail.request_id {
            obj.set("requestId", request_id.as_str())?;
        }
        if let Some(response_body) = &detail.response_body {
            obj.set("responseBody", response_body.as_str())?;
        }
    }
    match error {
        AiMuxError::NoSuchModel {
            model_id,
            model_type,
        } => {
            obj.set("modelId", model_id.as_str())?;
            if !model_type.is_empty() {
                obj.set("modelType", model_type.as_str())?;
            }
        }
        AiMuxError::NoSuchProvider { provider_id } => {
            obj.set("providerId", provider_id.as_str())?;
        }
        _ => {}
    }
    Ok(Error::from(obj.to_unknown()))
}

/// A [`BindingError`] as napi-rs's own plain `Error`: caller-side faults
/// (bad wire JSON, closed handle) → `InvalidArg`, everything else →
/// `GenericFailure`. napi-rs sets `code` from the status when it throws.
pub(crate) fn create_binding_throwable(b: &BindingError) -> Error {
    let status = match b {
        BindingError::InvalidWireJson { .. } | BindingError::InvalidHandle { .. } => {
            Status::InvalidArg
        }
        BindingError::ResultSerialization { .. } | BindingError::InvariantViolation { .. } => {
            Status::GenericFailure
        }
    };
    Error::new(status, b.to_string())
}

/// Fallible return type for napi methods. Converts to a value or rejects/throws
/// the corresponding public error family when `Env` is available on the JS
/// thread.
pub struct AimuxResult<T>(pub(crate) std::result::Result<T, AiMuxBindingError>);

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

/// Internal result alias used by helpers (`?` with [`AiMuxBindingError`]).
pub(crate) type MResult<T> = std::result::Result<T, AiMuxBindingError>;

impl<T> From<MResult<T>> for AimuxResult<T> {
    fn from(r: MResult<T>) -> Self {
        Self(r)
    }
}

/// Item yielded by the stream generator. `AsyncGenerator::next` returns
/// `napi::Result` without an `Env`, so errors ride the channel as typed
/// [`AiMuxBindingError`]s and convert into a structured throwable here, where
/// `ToNapiValue` runs on the JS thread with an `Env` present.
enum StreamItemValue {
    Json(String),
    Failure(AiMuxBindingError),
}

pub struct StreamItem {
    value: StreamItemValue,
}

impl StreamItem {
    pub(crate) fn json(value: String) -> Self {
        Self {
            value: StreamItemValue::Json(value),
        }
    }

    pub(crate) fn failure(error: AiMuxBindingError) -> Self {
        Self {
            value: StreamItemValue::Failure(error),
        }
    }
}

impl ToNapiValue for StreamItem {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> NapiResult<sys::napi_value> {
        match val.value {
            StreamItemValue::Json(s) => unsafe { String::to_napi_value(env, s) },
            StreamItemValue::Failure(m) => {
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

/// Build the registered JS `RecordingError`. It remains separate from the
/// `AimuxError` hierarchy.
pub(crate) fn create_recording_throwable(
    env: &Env,
    e: &aimux_core::recording::RecordingError,
) -> NapiResult<Error> {
    use aimux_core::recording::RecordingError as R;
    let code = match e {
        R::Init { .. } => "Init",
        R::OpenFile { .. } => "OpenFile",
        R::Spawn { .. } => "Spawn",
        R::WriterGone => "WriterGone",
        R::FlushTimeout => "FlushTimeout",
        R::Write(_) => "Write",
    };
    let message = e.to_string();
    let mut obj = new_registered_error(env, "RecordingError", &message)?;
    obj.set("code", code)?;
    Ok(Error::from(obj.to_unknown()))
}
