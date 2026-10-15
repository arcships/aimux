//! Async napi adapter for aimux-ops. No C ABI or blocking runtime is used.
//! Futures run on napi-rs's Tokio runtime; JS values never cross into those futures.
mod bridge;
mod error;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::SystemTime;

use aimux_core::AbortSignal;
use aimux_ops::dispatch::{self, Ctx};
use aimux_ops::envelope::{ErrorEnvelope, OpsError, ProtocolError};
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::Value;

pub use bridge::FrameIterator;
use error::{WireResult, envelope};

#[derive(Default)]
struct Resources {
    closed: bool,
    handles: HashSet<u64>,
    active: HashMap<u64, Weak<AbortSignal>>,
    // Operations must keep the context cancellation signal until handle.drop.
    operations: HashMap<u64, Arc<AbortSignal>>,
}

struct Realm {
    owner: Owner,
    next_request: AtomicU64,
    resources: Mutex<Resources>,
}

impl Realm {
    fn lock(&self) -> std::sync::MutexGuard<'_, Resources> {
        self.resources
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn context(&self, signal: &AbortSignal) -> Ctx {
        Ctx {
            owner: self.owner,
            abort: Some(signal.clone()),
            now: SystemTime::now,
        }
    }

    fn request(
        self: &Arc<Self>,
        signal: Arc<AbortSignal>,
    ) -> std::result::Result<Request, ErrorEnvelope> {
        let mut resources = self.lock();
        if resources.closed {
            return Err(invalid_state("Node environment is closed"));
        }
        let id = self
            .next_request
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| envelope(OpsError::Protocol(ProtocolError::HandleExhausted)))?;
        resources.active.insert(id, Arc::downgrade(&signal));
        Ok(Request {
            realm: self.clone(),
            id,
            signal,
        })
    }

    fn remember(&self, op: &str, result: &Value, signal: &Arc<AbortSignal>) {
        // Only lifecycle results carry handles. Ordinary model JSON is never scanned.
        if !matches!(
            op,
            "provider.create"
                | "provider.default"
                | "provider.custom"
                | "provider.model"
                | "provider.wrap"
                | "provider.files"
                | "registry.create"
                | "registry.provider"
                | "model.resolve"
                | "model.wrap"
                | "composite.router"
                | "composite.moa"
                | "transcription.start"
                | "abort.create"
                | "transport.create"
                | "credential.create"
                | "middleware.create"
                | "trace.create"
                | "recorder.create"
                | "sessionStore.create"
                | "replay.create"
                | "replay.capabilities"
        ) {
            return;
        }
        let mut handles = Vec::new();
        collect_handles(result, &mut handles);
        let mut resources = self.lock();
        for id in handles {
            if resources.closed {
                signal.abort();
                drop(HandleRegistry::global().remove(self.owner, HandleId(id)));
            } else {
                resources.handles.insert(id);
                if op == "transcription.start" {
                    resources.operations.insert(id, signal.clone());
                }
            }
        }
    }

    fn cleanup(&self) {
        let (active, operations) = {
            let mut resources = self.lock();
            resources.closed = true;
            resources.handles.clear();
            (
                std::mem::take(&mut resources.active),
                std::mem::take(&mut resources.operations),
            )
        };
        for signal in active.into_values().filter_map(|signal| signal.upgrade()) {
            signal.abort();
        }
        for signal in operations.into_values() {
            signal.abort();
        }
        // Release also covers handles returned by raw dispatch calls.
        HandleRegistry::global().release_owner(self.owner);
    }
}

struct Request {
    realm: Arc<Realm>,
    id: u64,
    signal: Arc<AbortSignal>,
}

impl Drop for Request {
    fn drop(&mut self) {
        self.realm.lock().active.remove(&self.id);
    }
}

fn invalid_state(message: &str) -> ErrorEnvelope {
    envelope(OpsError::Protocol(ProtocolError::InvalidState(
        message.to_owned(),
    )))
}

fn handle(value: &str) -> std::result::Result<HandleId, ErrorEnvelope> {
    if value.is_empty()
        || !value.bytes().all(|c| c.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(envelope(OpsError::Protocol(ProtocolError::InvalidParams(
            "handle must be a canonical decimal string".into(),
        ))));
    }
    value.parse().map(HandleId).map_err(|_| {
        envelope(OpsError::Protocol(ProtocolError::InvalidParams(
            "handle exceeds u64".into(),
        )))
    })
}

fn collect_handles(value: &Value, result: &mut Vec<u64>) {
    match value {
        Value::String(id) => {
            if let Ok(HandleId(id)) = handle(id) {
                if id != 0 {
                    result.push(id);
                }
            }
        }
        Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(
                    key.as_str(),
                    "handle"
                        | "handleId"
                        | "id"
                        | "fetch"
                        | "webSocket"
                        | "download"
                        | "capabilities"
                ) {
                    collect_handles(value, result);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_handles(value, result);
            }
        }
        _ => {}
    }
}

/// All Runtime instances in one napi Env share one unforgeable Owner::Node.
#[napi]
pub struct Runtime {
    realm: Arc<Realm>,
}

#[napi]
impl Runtime {
    #[napi(constructor)]
    pub fn new(env: Env) -> napi::Result<Self> {
        if let Some(realm) = env.get_instance_data::<Arc<Realm>>()? {
            return Ok(Self {
                realm: realm.clone(),
            });
        }
        let owner = Owner::new_node().map_err(|error| {
            error::throwable(&env, envelope(error)).unwrap_or_else(|error| error)
        })?;
        let realm = Arc::new(Realm {
            owner,
            next_request: AtomicU64::new(1),
            resources: Mutex::default(),
        });
        env.add_env_cleanup_hook(realm.clone(), |realm| realm.cleanup())?;
        env.set_instance_data(realm.clone(), (), |_| {})?;
        Ok(Self { realm })
    }

    /// Nonblocking release for explicit disposal and JS finalization.
    #[napi]
    pub fn drop_handle(&self, id: String) -> WireResult<()> {
        WireResult((|| {
            let id = handle(&id)?;
            let entry = HandleRegistry::global().remove(self.realm.owner, id);
            let signal = {
                let mut resources = self.realm.lock();
                resources.handles.remove(&id.0);
                resources.operations.remove(&id.0)
            };
            if let Some(signal) = signal {
                signal.abort();
            }
            if let Some(Entry::Operation(operation)) = entry {
                operation.consumer_close();
            }
            Ok(())
        })())
    }

    #[napi]
    pub fn manifest_json(&self) -> WireResult<String> {
        WireResult(error::serialize(&aimux_ops::manifest::manifest()))
    }

    #[napi]
    pub fn manifest_digest(&self) -> String {
        aimux_ops::manifest::manifest_digest()
    }

    #[napi(ts_return_type = "Promise<string>")]
    pub async fn call(
        &self,
        id: String,
        op: String,
        params_json: String,
        abort: Option<&Abort>,
    ) -> WireResult<String> {
        WireResult(
            async {
                let id = handle(&id)?;
                let params = error::parse("params", &params_json)?;
                let signal = self.signal(abort)?;
                let request = self.realm.request(signal.clone())?;
                let result = dispatch::call(self.realm.context(&signal), id, &op, params)
                    .await
                    .map_err(envelope)?;
                self.realm.remember(&op, &result, &signal);
                if op == "handle.drop" {
                    let mut resources = self.realm.lock();
                    resources.handles.remove(&id.0);
                    if let Some(signal) = resources.operations.remove(&id.0) {
                        signal.abort();
                    }
                }
                drop(request);
                error::serialize(&result)
            }
            .await,
        )
    }

    /// Copy the caller's buffer on the JS thread before returning the Promise.
    #[napi(ts_return_type = "Promise<void>")]
    pub fn operation_push<'env>(
        &self,
        env: &'env Env,
        id: String,
        data: Uint8Array,
    ) -> napi::Result<PromiseRaw<'env, WireResult<()>>> {
        let prepared = (|| {
            if data.len() > 1024 * 1024 {
                return Err(envelope(OpsError::Protocol(ProtocolError::ResourceLimit(
                    "audio chunk exceeds 1 MiB".into(),
                ))));
            }
            let entry = HandleRegistry::global()
                .get(self.realm.owner, handle(&id)?, &[HandleKind::Operation])
                .map_err(envelope)?;
            match entry {
                Entry::Operation(operation) => Ok((operation, data.to_vec())),
                _ => Err(invalid_state("registry returned a non-operation entry")),
            }
        })();
        env.spawn_future(async move {
            Ok(WireResult(match prepared {
                Ok((operation, bytes)) => {
                    operation.push(&bytes).await.map(|_| ()).map_err(envelope)
                }
                Err(error) => Err(error),
            }))
        })
    }

    #[napi(ts_return_type = "Promise<FrameIterator>")]
    pub async fn stream(
        &self,
        id: String,
        op: String,
        params_json: String,
        abort: Option<&Abort>,
    ) -> WireResult<FrameIterator> {
        WireResult(
            async {
                let id = handle(&id)?;
                let params = error::parse("params", &params_json)?;
                let signal = self.signal(abort)?;
                let request = self.realm.request(signal.clone())?;
                // Validation occurs before an iterator is returned, never as a fabricated first event.
                let stream = dispatch::stream(self.realm.context(&signal), id, &op, params)
                    .map_err(envelope)?;
                Ok(bridge::start(stream, request))
            }
            .await,
        )
    }
}

impl Runtime {
    fn signal(
        &self,
        abort: Option<&Abort>,
    ) -> std::result::Result<Arc<AbortSignal>, ErrorEnvelope> {
        match abort {
            Some(abort) => {
                HandleRegistry::global()
                    .get(
                        self.realm.owner,
                        HandleId(abort.id.load(Ordering::Acquire)),
                        &[HandleKind::Abort],
                    )
                    .map_err(envelope)?;
                Ok(abort.signal.clone())
            }
            None => Ok(Arc::new(AbortSignal::new())),
        }
    }
}

/// An Abort handle; dropping it releases the handle without signalling shared work.
#[napi]
pub struct Abort {
    id: AtomicU64,
    realm: Arc<Realm>,
    signal: Arc<AbortSignal>,
}

#[napi]
impl Abort {
    #[napi(constructor, ts_args_type = "runtime: Runtime, signal?: AbortSignal")]
    pub fn new(env: Env, runtime: &Runtime, signal: Option<Object<'_>>) -> napi::Result<Self> {
        let core = Arc::new(AbortSignal::new());
        if let Some(signal) = signal {
            let already_aborted = signal.get::<bool>("aborted")?.unwrap_or(false);
            // SAFETY: conversion stays on the JS thread; napi owns callback registration.
            let signal = unsafe {
                napi::bindgen_prelude::AbortSignal::from_napi_value(env.raw(), signal.raw())?
            };
            let watcher = core.clone();
            signal.on_abort(move || watcher.abort());
            if already_aborted {
                core.abort();
            }
        }
        let id = HandleRegistry::global()
            .insert(runtime.realm.owner, Entry::Abort(core.clone()))
            .map_err(|error| {
                error::throwable(&env, envelope(error)).unwrap_or_else(|error| error)
            })?;
        runtime.realm.lock().handles.insert(id.0);
        Ok(Self {
            id: AtomicU64::new(id.0),
            realm: runtime.realm.clone(),
            signal: core,
        })
    }

    #[napi(getter)]
    pub fn handle(&self) -> String {
        self.id.load(Ordering::Acquire).to_string()
    }
    #[napi]
    pub fn signal(&self) {
        if self.id.load(Ordering::Acquire) != 0 {
            self.signal.abort();
        }
    }
    #[napi]
    pub fn close(&self) {
        let id = self.id.swap(0, Ordering::AcqRel);
        if id != 0 {
            self.realm.lock().handles.remove(&id);
            drop(HandleRegistry::global().remove(self.realm.owner, HandleId(id)));
        }
    }
    #[napi(getter)]
    pub fn aborted(&self) -> bool {
        self.signal.is_aborted()
    }
}

impl Drop for Abort {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handles_preserve_all_u64_bits_and_reject_noncanonical_ids() {
        assert_eq!(
            handle("18446744073709551615").ok().map(|id| id.0),
            Some(u64::MAX)
        );
        for id in ["", "01", "-1", "+1", "1.0", "18446744073709551616"] {
            assert!(handle(id).is_err(), "{id}");
        }
    }
}
