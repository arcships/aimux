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
//!     ...  # any engine / binding failure
//! ```

use aimux_core::AiMuxError;
use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyAnyMethods;

// Base — catch-all for any aimux failure.
create_exception!(aimux, AimuxError, PyException, "Engine or binding failure");

create_exception!(aimux, APICallError, AimuxError, "API call failure");
create_exception!(aimux, JSONParseError, AimuxError, "JSON parse/serialize failure");
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

pub(crate) fn to_py_err(e: &AiMuxError) -> PyErr {
    Python::with_gil(|py| match raise_variant(py, e) {
        Ok(err) => err,
        Err(e) => e,
    })
}

fn raise_variant(py: Python<'_>, e: &AiMuxError) -> PyResult<PyErr> {
    let typ = match e {
        AiMuxError::ApiCall(_) => py.get_type_bound::<APICallError>(),
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
        AiMuxError::Aborted => py.get_type_bound::<RequestAbortedError>(),
        AiMuxError::Other(_) => py.get_type_bound::<OtherError>(),
    };
    let detail = match e {
        AiMuxError::ApiCall(d) => Some(d),
        _ => None,
    };
    let inst = typ.call1((e.to_string(),))?;
    // None when the core has no value (openai/anthropic convention: int | None).
    inst.setattr("status", e.status_code().map(i32::from))?;
    inst.setattr("retry_ms", e.retry_after_hint())?;
    inst.setattr("retryable", e.is_retryable())?;
    // Structured fields from `ApiCallError` (AI SDK `APICallError` analogues).
    inst.setattr(
        "provider_code",
        detail.and_then(|d| d.provider_code.as_deref()),
    )?;
    inst.setattr(
        "response_body",
        detail.and_then(|d| d.response_body.as_deref()),
    )?;
    inst.setattr(
        "request_id",
        detail.and_then(|d| d.request_id.as_deref()),
    )?;
    // Lossless externally-tagged JSON of the source error (parity with C ABI).
    inst.setattr("error_value", serde_json::to_string(e).ok())?;
    Ok(PyErr::from_value_bound(inst))
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
    Ok(())
}
