//! Thin synchronous Python adapter for the shared asynchronous ops dispatcher.
#![allow(unsafe_op_in_unsafe_fn, unexpected_cfgs, clippy::useless_conversion)]

mod bridge;
mod error;

#[cfg(feature = "contract-tests")]
#[path = "../../node/__test__/native_scenarios.rs"]
mod native_scenarios;

include!(concat!(env!("OUT_DIR"), "/contract.rs"));

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use aimux_core::AbortSignal;
use aimux_ops::dispatch::{self, Ctx};
use aimux_ops::envelope::{BindingError, OpsError, ProtocolError};
use aimux_ops::handle::{HandleId, HandleRegistry, Owner};
use pyo3::prelude::*;
use serde_json::Value;

use bridge::{StreamIterator, wait_call};

// Process lifetime: Rust statics are not dropped at interpreter shutdown. Each
// InterpreterOwner queues release_owner on this still-live runtime; late Python
// finalizers therefore cannot encounter a runtime that we have already dropped.
static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();

fn runtime() -> Result<&'static tokio::runtime::Runtime, OpsError> {
    RUNTIME
        .get_or_init(|| tokio::runtime::Runtime::new().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|message| OpsError::Binding(BindingError::CallbackFailure(message.clone())))
}

fn context(owner: Owner, abort: AbortSignal) -> Ctx {
    Ctx {
        owner,
        abort: Some(abort),
    }
}

fn invalid_state(message: &str) -> OpsError {
    OpsError::Protocol(ProtocolError::InvalidState(message.into()))
}

fn parse_handle(value: &str) -> Result<HandleId, OpsError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(OpsError::Protocol(ProtocolError::InvalidParams(
            "handle must be a decimal u64 string".into(),
        )));
    }
    value
        .parse()
        .map(HandleId)
        .map_err(|_| OpsError::Protocol(ProtocolError::InvalidParams("handle exceeds u64".into())))
}

fn parse_params(text: &str) -> Result<Value, OpsError> {
    serde_json::from_str(text).map_err(|error| {
        OpsError::Binding(BindingError::InvalidWireJson {
            argument: "params_json".into(),
            message: error.to_string(),
        })
    })
}

fn serialize(value: &impl serde::Serialize) -> Result<String, OpsError> {
    serde_json::to_string(value)
        .map_err(|error| OpsError::Binding(BindingError::ResultSerialization(error.to_string())))
}

fn interpreter_id(py: Python<'_>) -> PyResult<i64> {
    // SAFETY: Python's token guarantees an attached thread holding the GIL.
    // Both functions are in CPython's stable ABI from Python 3.9 onward.
    let id = unsafe { pyo3::ffi::PyInterpreterState_GetID(pyo3::ffi::PyInterpreterState_Get()) };
    if id < 0 {
        return Err(PyErr::fetch(py));
    }
    Ok(id)
}

struct InterpreterOwner {
    token: Owner,
}

impl Drop for InterpreterOwner {
    fn drop(&mut self) {
        if let Some(Ok(rt)) = RUNTIME.get() {
            let owner = self.token;
            // Resource destructors may perform I/O. Interpreter teardown only
            // enqueues cleanup; no registry lock or GIL crosses the join.
            rt.spawn_blocking(move || HandleRegistry::global().release_owner(owner));
        }
    }
}

fn current_owner(py: Python<'_>) -> PyResult<Arc<InterpreterOwner>> {
    static OWNERS: OnceLock<Mutex<HashMap<i64, Weak<InterpreterOwner>>>> = OnceLock::new();
    let interpreter = interpreter_id(py)?;
    let mut owners = OWNERS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(owner) = owners.get(&interpreter).and_then(Weak::upgrade) {
        return Ok(owner);
    }
    let token = Owner::new_python().map_err(|e| error::ops_error(py, e))?;
    let owner = Arc::new(InterpreterOwner { token });
    owners.retain(|_, owner| owner.strong_count() > 0);
    owners.insert(interpreter, Arc::downgrade(&owner));
    Ok(owner)
}

fn check_owner(py: Python<'_>, owner: Owner) -> PyResult<()> {
    if current_owner(py)?.token != owner {
        return Err(error::ops_error(
            py,
            invalid_state("object belongs to another interpreter"),
        ));
    }
    Ok(())
}

// The module keeps the token alive even between Client instances.
#[pyclass]
struct OwnerGuard {
    _owner: Arc<InterpreterOwner>,
}

#[pyclass(module = "aimux.aimux")]
struct Abort {
    owner: Arc<InterpreterOwner>,
    signal: AbortSignal,
}

#[pymethods]
impl Abort {
    #[new]
    fn new(py: Python<'_>) -> PyResult<Self> {
        Ok(Self {
            owner: current_owner(py)?,
            signal: AbortSignal::new(),
        })
    }

    fn cancel(&self, py: Python<'_>) -> PyResult<()> {
        check_owner(py, self.owner.token)?;
        self.signal.abort();
        Ok(())
    }

    #[getter]
    fn cancelled(&self) -> bool {
        self.signal.is_aborted()
    }
}

fn signal_for(
    py: Python<'_>,
    owner: Owner,
    abort: Option<PyRef<'_, Abort>>,
) -> PyResult<AbortSignal> {
    match abort {
        Some(abort) => {
            check_owner(py, abort.owner.token)?;
            if abort.owner.token != owner {
                return Err(error::ops_error(py, invalid_state("foreign abort signal")));
            }
            Ok(abort.signal.clone())
        }
        None => Ok(AbortSignal::new()),
    }
}

/// All operation names, handle validation and wire schemas belong to aimux-ops.
#[pyclass(module = "aimux.aimux")]
struct Client {
    owner: Arc<InterpreterOwner>,
}

#[pymethods]
impl Client {
    #[new]
    fn new(py: Python<'_>) -> PyResult<Self> {
        let owner = current_owner(py)?;
        py.allow_threads(runtime)
            .map_err(|e| error::ops_error(py, e))?;
        Ok(Self { owner })
    }

    #[pyo3(signature = (handle, op, params_json="{}", abort=None))]
    fn call(
        &self,
        py: Python<'_>,
        handle: String,
        op: String,
        params_json: &str,
        abort: Option<PyRef<'_, Abort>>,
    ) -> PyResult<String> {
        check_owner(py, self.owner.token)?;
        let handle = parse_handle(&handle).map_err(|e| error::ops_error(py, e))?;
        let params = parse_params(params_json).map_err(|e| error::ops_error(py, e))?;
        let signal = signal_for(py, self.owner.token, abort)?;
        let ctx = context(self.owner.token, signal.clone());
        // No Python references cross this boundary, including in construction,
        // discovery, poll and recorder flush operations.
        wait_call(py, signal, async move {
            #[cfg(feature = "contract-tests")]
            if op == "test.teardown" {
                let owner = InterpreterOwner {
                    token: Owner::new_python()?,
                };
                let token = owner.token;
                let id = native_scenarios::operation(token).await;
                // Exercise the same Drop implementation as the module guard and
                // Client, without retiring this test interpreter's live owner.
                drop(owner);
                tokio::time::timeout(std::time::Duration::from_secs(3), async {
                    while !native_scenarios::released(token, id) {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                let runtime_alive = runtime()?.spawn(async { true }).await.unwrap();
                return serialize(&serde_json::json!({
                    "released": true, "runtimeAlive": runtime_alive,
                }));
            }
            #[cfg(feature = "contract-tests")]
            if op == "test.scenario" {
                return serialize(&native_scenarios::replay(params, ctx.owner).await);
            }
            let value = dispatch::call(ctx, handle, &op, params).await?;
            serialize(&value)
        })
    }

    /// Binary admission is bounded by the shared operation service.
    fn operation_push(&self, py: Python<'_>, handle: String, data: &[u8]) -> PyResult<()> {
        check_owner(py, self.owner.token)?;
        let handle = parse_handle(&handle).map_err(|e| error::ops_error(py, e))?;
        if data.len() > 1024 * 1024 {
            return Err(error::ops_error(
                py,
                OpsError::Protocol(ProtocolError::ResourceLimit(
                    "audio chunk exceeds 1 MiB".into(),
                )),
            ));
        }
        let data = data.to_vec();
        let owner = self.owner.token;
        py.allow_threads(move || {
            dispatch::operation_push(&context(owner, AbortSignal::new()), handle, &data)
        })
        .map_err(|e| error::ops_error(py, e))
    }

    /// GC teardown only queues work; explicit Handle.close uses checked call.
    fn drop_later(&self, py: Python<'_>, handle: String) -> PyResult<()> {
        check_owner(py, self.owner.token)?;
        let handle = parse_handle(&handle).map_err(|e| error::ops_error(py, e))?;
        let owner = self.owner.token;
        let rt = py
            .allow_threads(runtime)
            .map_err(|e| error::ops_error(py, e))?;
        rt.spawn(async move {
            let _ = dispatch::call(
                context(owner, AbortSignal::new()),
                handle,
                dispatch::HANDLE_DROP,
                serde_json::json!({}),
            )
            .await;
        });
        Ok(())
    }

    #[pyo3(signature = (handle, op, params_json="{}", abort=None))]
    fn stream(
        &self,
        py: Python<'_>,
        handle: String,
        op: String,
        params_json: &str,
        abort: Option<PyRef<'_, Abort>>,
    ) -> PyResult<StreamIterator> {
        check_owner(py, self.owner.token)?;
        let handle = parse_handle(&handle).map_err(|e| error::ops_error(py, e))?;
        let params = parse_params(params_json).map_err(|e| error::ops_error(py, e))?;
        let signal = signal_for(py, self.owner.token, abort)?;
        StreamIterator::start(py, self.owner.clone(), handle, op, params, signal)
    }
}

#[pyfunction]
fn manifest_json(py: Python<'_>) -> PyResult<String> {
    py.allow_threads(|| serialize(&aimux_ops::manifest::manifest()))
        .map_err(|e| error::ops_error(py, e))
}

#[pyfunction]
fn manifest_digest(py: Python<'_>) -> String {
    py.allow_threads(aimux_ops::manifest::manifest_digest)
}

#[pymodule]
fn aimux(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let owner = current_owner(m.py())?;
    m.add(
        "_owner_guard",
        Py::new(m.py(), OwnerGuard { _owner: owner })?,
    )?;
    m.add_class::<Client>()?;
    m.add_class::<Abort>()?;
    m.add_class::<StreamIterator>()?;
    m.add_function(wrap_pyfunction!(manifest_json, m)?)?;
    m.add_function(wrap_pyfunction!(manifest_digest, m)?)?;
    m.add("EXPECTED_MANIFEST_DIGEST", EXPECTED_MANIFEST_DIGEST)?;
    m.add("EXPECTED_ABI_VERSION", EXPECTED_ABI_VERSION)?;
    m.add("EXPECTED_OPS_VERSION", EXPECTED_OPS_VERSION)?;
    m.add("ABI_VERSION", aimux_ops::op::ABI_VERSION)?;
    m.add("OPS_VERSION", aimux_ops::op::OPS_VERSION)?;
    Ok(())
}
