//! Project the shared envelope once; Python only constructs exception objects.
use std::time::SystemTime;

use aimux_ops::envelope::{ErrorEnvelope, OpsError};
use pyo3::prelude::*;

pub(crate) fn ops_error(py: Python<'_>, error: OpsError) -> PyErr {
    envelope_error(py, &error.envelope(SystemTime::now()))
}

pub(crate) fn envelope_error(py: Python<'_>, envelope: &ErrorEnvelope) -> PyErr {
    let build = || -> PyResult<PyErr> {
        let json = serde_json::to_string(envelope)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
        let value = py.import_bound("json")?.call_method1("loads", (json,))?;
        let error = py
            .import_bound("aimux._errors")?
            .call_method1("from_envelope", (value,))?;
        Ok(PyErr::from_value_bound(error))
    };
    build().unwrap_or_else(|error| error)
}
