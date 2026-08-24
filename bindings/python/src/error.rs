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
use pyo3::types::PyAnyMethods;

// Base — catch-all for AiMuxError failures only. Recording and binding failures
// have independent public exception types below.
create_exception!(aimux, AimuxError, PyException, "AiMux failure");

create_exception!(aimux, APICallError, AimuxError, "API call failure");
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
create_exception!(
    aimux,
    NoSuchToolError,
    AimuxError,
    "Model called a tool that was not provided"
);
create_exception!(
    aimux,
    InvalidToolInputError,
    AimuxError,
    "Tool call input failed to parse or violated the tool's schema"
);
create_exception!(
    aimux,
    ToolCallRepairError,
    AimuxError,
    "Repair callback failed while handling an invalid tool call"
);
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
    Python::with_gil(|py| match raise_variant(py, e) {
        Ok(err) => err,
        Err(e) => e,
    })
}

fn raise_variant(py: Python<'_>, e: &AiMuxError) -> PyResult<PyErr> {
    Ok(PyErr::from_value_bound(variant_instance(py, e)?))
}

/// The exception instance for a variant, so `ToolCallRepair` can nest its
/// `original_error` as a real exception instance.
fn variant_instance<'py>(py: Python<'py>, e: &AiMuxError) -> PyResult<Bound<'py, PyAny>> {
    let typ = match e {
        AiMuxError::ApiCall(_) => py.get_type_bound::<APICallError>(),
        AiMuxError::JsonParse(_) => py.get_type_bound::<JSONParseError>(),
        AiMuxError::InvalidResponseData(_) => py.get_type_bound::<InvalidResponseDataError>(),
        AiMuxError::NoSuchTool { .. } => py.get_type_bound::<NoSuchToolError>(),
        AiMuxError::InvalidToolInput { .. } => py.get_type_bound::<InvalidToolInputError>(),
        AiMuxError::ToolCallRepair { .. } => py.get_type_bound::<ToolCallRepairError>(),
        AiMuxError::InvalidArgument(_) => py.get_type_bound::<InvalidArgumentError>(),
        AiMuxError::InvalidPrompt(_) => py.get_type_bound::<InvalidPromptError>(),
        AiMuxError::TokenExpired(_) => py.get_type_bound::<TokenExpiredError>(),
        AiMuxError::UnsupportedFunctionality(_) => {
            py.get_type_bound::<UnsupportedFunctionalityError>()
        }
        AiMuxError::NoSuchModel { .. } => py.get_type_bound::<NoSuchModelError>(),
        AiMuxError::NoSuchProvider { .. } => py.get_type_bound::<NoSuchProviderError>(),
        AiMuxError::Timeout(_) => py.get_type_bound::<APITimeoutError>(),
        AiMuxError::Aborted => py.get_type_bound::<RequestAbortedError>(),
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
            inst.setattr("request_id", d.request_id.as_deref())?;
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
        AiMuxError::NoSuchTool {
            tool_name,
            available_tools,
        } => {
            inst.setattr("tool_name", tool_name.as_str())?;
            inst.setattr("available_tools", available_tools.clone())?;
        }
        AiMuxError::InvalidToolInput {
            tool_name,
            tool_input,
            ..
        } => {
            inst.setattr("tool_name", tool_name.as_str())?;
            inst.setattr("tool_input", tool_input.as_str())?;
        }
        AiMuxError::ToolCallRepair { original_error, .. } => {
            inst.setattr("original_error", variant_instance(py, original_error)?)?;
        }
        _ => {}
    }
    Ok(inst)
}

/// Register the exception hierarchy on the Python module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("AimuxError", py.get_type_bound::<AimuxError>())?;
    m.add("APICallError", py.get_type_bound::<APICallError>())?;
    m.add("JSONParseError", py.get_type_bound::<JSONParseError>())?;
    m.add(
        "InvalidResponseDataError",
        py.get_type_bound::<InvalidResponseDataError>(),
    )?;
    m.add("NoSuchToolError", py.get_type_bound::<NoSuchToolError>())?;
    m.add(
        "InvalidToolInputError",
        py.get_type_bound::<InvalidToolInputError>(),
    )?;
    m.add(
        "ToolCallRepairError",
        py.get_type_bound::<ToolCallRepairError>(),
    )?;
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
