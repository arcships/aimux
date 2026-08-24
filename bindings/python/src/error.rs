//! Map `aimux_core::AiMuxError` into an idiomatic Python exception hierarchy.
//!
//! Mirrors mature SDKs (OpenAI / Anthropic) and Vercel AI SDK style: a base
//! type plus explicit subclasses — not a single class with a string `code`.
//!
//! ```python
//! try:
//!     generate_text(model, "hi")
//! except APICallError as e:
//!     if e.status == 429:      # classification is the status field,
//!         ...                  # exactly like AI SDK APICallError.statusCode
//!     elif e.status == 401:
//!         ...
//! except AimuxError:
//!     ...  # any AiMuxError failure
//! ```
//!
//! Failures of the pyo3 binding itself are not aimux types: they surface as
//! Python's own `ValueError` (bad wire input / closed object) or
//! `RuntimeError` (result serialization / broken invariant).

use aimux_core::{AiMuxError, recording::RecordingError as CoreRecordingError};
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAnyMethods, PyList};

// Base — catch-all for AiMuxError failures only. Recording and binding failures
// have independent public exception types below.
create_exception!(aimux, AimuxError, PyException, "AiMux failure");

create_exception!(aimux, APICallError, AimuxError, "API call failure");
create_exception!(aimux, RetryError, AimuxError, "Operation retry failure");
create_exception!(
    aimux,
    JSONParseError,
    AimuxError,
    "Provider response JSON parse failure"
);
create_exception!(
    aimux,
    InvalidResponseDataError,
    AimuxError,
    "Invalid response data"
);
create_exception!(aimux, ToolError, AimuxError, "Tool-related failure");
create_exception!(aimux, InvalidArgumentError, AimuxError, "Invalid argument");
create_exception!(aimux, InvalidPromptError, AimuxError, "Invalid prompt");
create_exception!(aimux, TokenExpiredError, AimuxError, "Access token expired");
create_exception!(
    aimux,
    UnsupportedFunctionalityError,
    AimuxError,
    "Unsupported functionality"
);
create_exception!(
    aimux,
    NoSuchModelError,
    AimuxError,
    "No such model in registry"
);
create_exception!(
    aimux,
    NoSuchProviderError,
    AimuxError,
    "No such provider in registry"
);
create_exception!(aimux, APITimeoutError, AimuxError, "Request timed out");
create_exception!(aimux, RequestAbortedError, AimuxError, "Request aborted");
create_exception!(aimux, OtherError, AimuxError, "Unclassified failure");
// The recorder's failure type. A second, unrelated error type in the core
// (`aimux_core::recording::RecordingError`), so a second, unrelated exception
// here: it derives from Python's `Exception`, not from `AimuxError`.
create_exception!(
    aimux,
    RecordingError,
    pyo3::exceptions::PyException,
    "Recorder could not confirm data on disk"
);

/// What the binding itself can get wrong. Internal routing only — projected
/// onto Python's builtin `ValueError` / `RuntimeError` by [`binding_py_err`].
#[derive(Debug, Clone)]
pub(crate) enum BindingError {
    /// A JSON *text* the binding transports did not parse.
    InvalidWireJson {
        argument: &'static str,
        message: String,
    },
    /// A closed session / dropped handle handed to the binding.
    InvalidHandle { expected: &'static str },
    /// The binding could not serialize a result to JSON.
    ResultSerialization { message: String },
    /// A binding invariant broke.
    InvariantViolation { message: String },
}

pub(crate) fn binding_py_err(e: &BindingError) -> PyErr {
    match e {
        BindingError::InvalidWireJson { argument, message } => {
            PyValueError::new_err(format!("{argument}: invalid JSON: {message}"))
        }
        BindingError::InvalidHandle { expected } => {
            PyValueError::new_err(format!("{expected} is closed"))
        }
        BindingError::ResultSerialization { message } => {
            PyRuntimeError::new_err(format!("serialize result: {message}"))
        }
        BindingError::InvariantViolation { message } => PyRuntimeError::new_err(message.clone()),
    }
}

/// Parse a JSON text the binding transports (`prompt_json` / `opts_json` /
/// `config_json` / …). Malformed text → binding `InvalidWireJson`; text that
/// parses but violates the schema → `AiMuxError::InvalidArgument` (what the
/// core would say about the value).
pub(crate) fn wire_json<T: serde::de::DeserializeOwned>(
    argument: &'static str,
    s: &str,
) -> PyResult<T> {
    serde_json::from_str(s).map_err(|e| match e.classify() {
        serde_json::error::Category::Data => to_py_err(&AiMuxError::InvalidArgument(format!(
            "invalid {argument}: {e}"
        ))),
        _ => binding_py_err(&BindingError::InvalidWireJson {
            argument,
            message: e.to_string(),
        }),
    })
}

/// Serialize a result for the wire; failure is the binding's, not an AiMuxError.
pub(crate) fn serialize_result<T: serde::Serialize>(v: &T) -> PyResult<String> {
    serde_json::to_string(v).map_err(|e| {
        binding_py_err(&BindingError::ResultSerialization {
            message: e.to_string(),
        })
    })
}

/// One internal error carrier for failures crossing the Python binding.
///
/// The variants remain separate through the conversion point: `AiMux` becomes
/// an `AimuxError` subclass, `Recording` becomes `RecordingError`, and
/// `Binding` becomes Python's builtin `ValueError` / `RuntimeError`.
#[derive(Debug)]
pub(crate) enum AiMuxBindingError {
    AiMux(AiMuxError),
    Recording(CoreRecordingError),
    Binding(BindingError),
}

impl From<AiMuxError> for AiMuxBindingError {
    fn from(e: AiMuxError) -> Self {
        Self::AiMux(e)
    }
}

impl From<CoreRecordingError> for AiMuxBindingError {
    fn from(e: CoreRecordingError) -> Self {
        Self::Recording(e)
    }
}

impl From<BindingError> for AiMuxBindingError {
    fn from(e: BindingError) -> Self {
        Self::Binding(e)
    }
}

impl AiMuxBindingError {
    pub(crate) fn to_py_err(&self) -> PyErr {
        match self {
            Self::AiMux(e) => crate::error::to_py_err(e),
            Self::Recording(e) => recording_py_err(e),
            Self::Binding(e) => binding_py_err(e),
        }
    }
}

/// Raise `RecordingError` for a core recording failure. `code` mirrors the
/// Rust variant name: "Init" / "OpenFile" / "Spawn" / "WriterGone" /
/// "FlushTimeout" / "Write".
fn recording_py_err(e: &CoreRecordingError) -> PyErr {
    use CoreRecordingError as R;
    let code = match e {
        R::Init { .. } => "Init",
        R::OpenFile { .. } => "OpenFile",
        R::Spawn { .. } => "Spawn",
        R::WriterGone => "WriterGone",
        R::FlushTimeout => "FlushTimeout",
        R::Write(_) => "Write",
    };
    Python::with_gil(|py| {
        let build = || -> PyResult<PyErr> {
            let inst = py
                .get_type_bound::<RecordingError>()
                .call1((e.to_string(),))?;
            inst.setattr("code", code)?;
            Ok(PyErr::from_value_bound(inst))
        };
        build().unwrap_or_else(|err| err)
    })
}

pub(crate) fn to_py_err(e: &AiMuxError) -> PyErr {
    Python::with_gil(|py| match exception_instance(py, e) {
        Ok(instance) => PyErr::from_value_bound(instance),
        Err(e) => e,
    })
}

fn exception_instance<'py>(
    py: Python<'py>,
    e: &AiMuxError,
) -> PyResult<Bound<'py, PyAny>> {
    let typ = match e {
        AiMuxError::ApiCall(_) => py.get_type_bound::<APICallError>(),
        AiMuxError::Retry(_) => py.get_type_bound::<RetryError>(),
        AiMuxError::JsonParse(_) => py.get_type_bound::<JSONParseError>(),
        AiMuxError::InvalidResponseData(_) => py.get_type_bound::<InvalidResponseDataError>(),
        AiMuxError::Tool(_) => py.get_type_bound::<ToolError>(),
        AiMuxError::InvalidArgument(_) => py.get_type_bound::<InvalidArgumentError>(),
        AiMuxError::InvalidPrompt(_) => py.get_type_bound::<InvalidPromptError>(),
        AiMuxError::TokenExpired(_) => py.get_type_bound::<TokenExpiredError>(),
        AiMuxError::UnsupportedFunctionality(_) => {
            py.get_type_bound::<UnsupportedFunctionalityError>()
        }
        AiMuxError::NoSuchModel { .. } => py.get_type_bound::<NoSuchModelError>(),
        AiMuxError::NoSuchProvider { .. } => py.get_type_bound::<NoSuchProviderError>(),
        AiMuxError::Timeout(_) => py.get_type_bound::<APITimeoutError>(),
        AiMuxError::Aborted(_) => py.get_type_bound::<RequestAbortedError>(),
        AiMuxError::Other(_) => py.get_type_bound::<OtherError>(),
    };
    let inst = typ.call1((e.to_string(),))?;
    match e {
        AiMuxError::ApiCall(d) => {
            // Optional API-call fields use None, following normal Python SDK
            // conventions; unrelated exception classes do not expose them.
            inst.setattr("status", e.status_code().map(i32::from))?;
            inst.setattr("retryable", e.is_retryable())?;
            inst.setattr("retry_ms", e.retry_after_hint())?;
            inst.setattr("provider_code", d.provider_code.as_deref())?;
            // str(e) is the composed "API call error: HTTP 429: …"; this is
            // the failure's own text. Usually the provider's words — ours on
            // a transport failure or an unreadable body (response_body is
            // the evidence either way).
            inst.setattr(
                "provider_message",
                (!d.message.is_empty()).then_some(d.message.as_str()),
            )?;
            inst.setattr("response_body", d.response_body.as_deref())?;
            inst.setattr("url", (!d.url.is_empty()).then_some(d.url.as_str()))?;
            // JSON null projects to Python None, so no separate absent case.
            inst.setattr(
                "request_body_values",
                json_value_to_python(py, &d.request_body_values)?,
            )?;
            inst.setattr("response_headers", d.response_headers.clone())?;
            inst.setattr(
                "data",
                match &d.data {
                    Some(data) => Some(json_value_to_python(py, data)?),
                    None => None,
                },
            )?;
        }
        AiMuxError::Retry(retry) => {
            // Attempt history, oldest first; each entry is itself a full
            // exception instance (recursing through this same projection).
            let errors = PyList::empty_bound(py);
            for error in &retry.errors {
                errors.append(exception_instance(py, error)?)?;
            }
            inst.setattr(
                "reason",
                match retry.reason {
                    aimux_core::RetryErrorReason::MaxRetriesExceeded => "maxRetriesExceeded",
                    aimux_core::RetryErrorReason::ErrorNotRetryable => "errorNotRetryable",
                },
            )?;
            // A deserialized history may be empty; last_error is None then.
            let last_error = match errors.len() {
                0 => None,
                n => Some(errors.get_item(n - 1)?),
            };
            inst.setattr("errors", &errors)?;
            inst.setattr("last_error", last_error)?;
        }
        AiMuxError::TokenExpired(_) => {
            inst.setattr("status", 401)?;
        }
        AiMuxError::NoSuchModel {
            model_id,
            model_type,
        } => {
            inst.setattr("model_id", model_id.as_str())?;
            inst.setattr(
                "model_type",
                (!model_type.is_empty()).then_some(model_type.as_str()),
            )?;
        }
        AiMuxError::NoSuchProvider { provider_id } => {
            inst.setattr("provider_id", provider_id.as_str())?;
        }
        _ => {}
    }
    Ok(inst)
}

fn json_value_to_python<'py>(
    py: Python<'py>,
    value: &serde_json::Value,
) -> PyResult<Bound<'py, PyAny>> {
    py.import_bound("json")?.call_method1(
        "loads",
        (serde_json::to_string(value).expect("serde_json::Value is always serializable"),),
    )
}

/// Register the exception hierarchy on the Python module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("AimuxError", py.get_type_bound::<AimuxError>())?;
    m.add("APICallError", py.get_type_bound::<APICallError>())?;
    m.add("RetryError", py.get_type_bound::<RetryError>())?;
    m.add("JSONParseError", py.get_type_bound::<JSONParseError>())?;
    m.add(
        "InvalidResponseDataError",
        py.get_type_bound::<InvalidResponseDataError>(),
    )?;
    m.add("ToolError", py.get_type_bound::<ToolError>())?;
    m.add(
        "InvalidArgumentError",
        py.get_type_bound::<InvalidArgumentError>(),
    )?;
    m.add(
        "InvalidPromptError",
        py.get_type_bound::<InvalidPromptError>(),
    )?;
    m.add(
        "TokenExpiredError",
        py.get_type_bound::<TokenExpiredError>(),
    )?;
    m.add(
        "UnsupportedFunctionalityError",
        py.get_type_bound::<UnsupportedFunctionalityError>(),
    )?;
    m.add("NoSuchModelError", py.get_type_bound::<NoSuchModelError>())?;
    m.add(
        "NoSuchProviderError",
        py.get_type_bound::<NoSuchProviderError>(),
    )?;
    m.add("APITimeoutError", py.get_type_bound::<APITimeoutError>())?;
    m.add(
        "RequestAbortedError",
        py.get_type_bound::<RequestAbortedError>(),
    )?;
    m.add("OtherError", py.get_type_bound::<OtherError>())?;
    m.add("RecordingError", py.get_type_bound::<RecordingError>())?;
    Ok(())
}
