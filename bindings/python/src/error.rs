//! Map `aimux_core::AiMuxError` into an idiomatic Python exception hierarchy.
//!
//! Mirrors mature SDKs (OpenAI / Anthropic) and Vercel AI SDK style: a base
//! type plus explicit subclasses — not a single class with a string `code`.
//!
//! ```python
//! try:
//!     generate_text(model, "hi")
//! except RateLimitError as e:
//!     ...  # e.retry_ms, e.status
//! except AuthenticationError as e:
//!     ...
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

create_exception!(aimux, ProviderError, AimuxError, "Provider-layer failure");
create_exception!(aimux, HttpError, AimuxError, "HTTP transport failure");
create_exception!(aimux, JsonError, AimuxError, "JSON parse/serialize failure");
create_exception!(aimux, StreamError, AimuxError, "Streaming failure");
create_exception!(aimux, ToolError, AimuxError, "Tool-related failure");
create_exception!(aimux, InvalidArgumentError, AimuxError, "Invalid argument");
create_exception!(aimux, InvalidPromptError, AimuxError, "Invalid prompt");
create_exception!(aimux, RateLimitError, AimuxError, "Rate limited (HTTP 429)");
create_exception!(
    aimux,
    AuthenticationError,
    AimuxError,
    "Authentication failed (HTTP 401)"
);
create_exception!(aimux, TokenExpiredError, AimuxError, "Access token expired");
create_exception!(
    aimux,
    ModelNotFoundError,
    AimuxError,
    "Model not found (HTTP 404)"
);
create_exception!(
    aimux,
    UnsupportedError,
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
    UnknownProviderError,
    AimuxError,
    "Unknown provider name"
);
create_exception!(aimux, APICallError, AimuxError, "Provider API call failed");
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
        AiMuxError::Provider(_) => py.get_type_bound::<ProviderError>(),
        AiMuxError::Http(_) => py.get_type_bound::<HttpError>(),
        AiMuxError::Json(_) => py.get_type_bound::<JsonError>(),
        AiMuxError::Stream(_) => py.get_type_bound::<StreamError>(),
        AiMuxError::Tool(_) => py.get_type_bound::<ToolError>(),
        AiMuxError::InvalidArgument(_) => py.get_type_bound::<InvalidArgumentError>(),
        AiMuxError::InvalidPrompt(_) => py.get_type_bound::<InvalidPromptError>(),
        AiMuxError::RateLimited { .. } => py.get_type_bound::<RateLimitError>(),
        AiMuxError::Auth(_) => py.get_type_bound::<AuthenticationError>(),
        AiMuxError::TokenExpired(_) => py.get_type_bound::<TokenExpiredError>(),
        AiMuxError::ModelNotFound(_) => py.get_type_bound::<ModelNotFoundError>(),
        AiMuxError::Unsupported(_) => py.get_type_bound::<UnsupportedError>(),
        AiMuxError::NoSuchModel(_) => py.get_type_bound::<NoSuchModelError>(),
        AiMuxError::UnknownProvider(_) => py.get_type_bound::<UnknownProviderError>(),
        AiMuxError::ApiCall(_) => py.get_type_bound::<APICallError>(),
        AiMuxError::Timeout(_) => py.get_type_bound::<APITimeoutError>(),
        AiMuxError::Aborted => py.get_type_bound::<RequestAbortedError>(),
        AiMuxError::Other(_) => py.get_type_bound::<OtherError>(),
    };
    let inst = typ.call1((e.to_string(),))?;
    // None when the core has no value (openai/anthropic convention: int | None).
    inst.setattr("status", e.status_code().map(i32::from))?;
    inst.setattr("retry_ms", e.retry_after_hint())?;
    // Lossless externally-tagged JSON of the source error (parity with C ABI).
    inst.setattr("error_value", serde_json::to_string(e).ok())?;
    Ok(PyErr::from_value_bound(inst))
}

/// Register the exception hierarchy on the Python module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("AimuxError", py.get_type_bound::<AimuxError>())?;
    m.add("ProviderError", py.get_type_bound::<ProviderError>())?;
    m.add("HttpError", py.get_type_bound::<HttpError>())?;
    m.add("JsonError", py.get_type_bound::<JsonError>())?;
    m.add("StreamError", py.get_type_bound::<StreamError>())?;
    m.add("ToolError", py.get_type_bound::<ToolError>())?;
    m.add(
        "InvalidArgumentError",
        py.get_type_bound::<InvalidArgumentError>(),
    )?;
    m.add(
        "InvalidPromptError",
        py.get_type_bound::<InvalidPromptError>(),
    )?;
    m.add("RateLimitError", py.get_type_bound::<RateLimitError>())?;
    m.add(
        "AuthenticationError",
        py.get_type_bound::<AuthenticationError>(),
    )?;
    m.add(
        "TokenExpiredError",
        py.get_type_bound::<TokenExpiredError>(),
    )?;
    m.add(
        "ModelNotFoundError",
        py.get_type_bound::<ModelNotFoundError>(),
    )?;
    m.add("UnsupportedError", py.get_type_bound::<UnsupportedError>())?;
    m.add("NoSuchModelError", py.get_type_bound::<NoSuchModelError>())?;
    m.add(
        "UnknownProviderError",
        py.get_type_bound::<UnknownProviderError>(),
    )?;
    m.add("APICallError", py.get_type_bound::<APICallError>())?;
    m.add("APITimeoutError", py.get_type_bound::<APITimeoutError>())?;
    m.add(
        "RequestAbortedError",
        py.get_type_bound::<RequestAbortedError>(),
    )?;
    m.add("OtherError", py.get_type_bound::<OtherError>())?;
    Ok(())
}
