//! Table-driven gates over the real manifest and real dispatcher.
//! An isolated registry provides typed mock resources; counting handlers replace
//! business I/O while retaining every production descriptor and validation step.
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use aimux_ops::dispatch::{
    self, CallFuture, Ctx, Dispatcher, EventStream, Handler, ResourceRegistry, Target,
};
use aimux_ops::envelope::{BindingError, OpsError, ProtocolError};
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use aimux_ops::op::OpMode;
use futures::StreamExt;
use serde_json::{Value, json};

#[derive(Default)]
struct Registry {
    entries: Mutex<HashMap<u64, (Owner, HandleKind)>>,
    next: AtomicUsize,
}
impl Registry {
    fn insert(&self, owner: Owner, kind: HandleKind) -> HandleId {
        let id = self.next.fetch_add(1, Ordering::Relaxed) as u64 + 1;
        self.entries.lock().unwrap().insert(id, (owner, kind));
        HandleId(id)
    }
}
impl ResourceRegistry for Registry {
    fn get(&self, owner: Owner, id: HandleId, allowed: &[HandleKind]) -> Result<Entry, OpsError> {
        let entries = self.entries.lock().unwrap();
        let actual = entries
            .get(&id.0)
            .filter(|(found, _)| *found == owner)
            .map(|(_, kind)| *kind);
        if actual.is_some_and(|kind| allowed.contains(&kind)) {
            // Handlers only count admissions. No fake model/provider behaviour.
            Ok(Entry::Abort(Arc::new(aimux_core::AbortSignal::new())))
        } else {
            Err(BindingError::InvalidHandle {
                expected: allowed.to_vec(),
                actual,
            }
            .into())
        }
    }
    fn remove(&self, owner: Owner, id: HandleId) -> Option<Entry> {
        let mut entries = self.entries.lock().unwrap();
        if entries.get(&id.0).is_some_and(|(found, _)| *found == owner) {
            entries.remove(&id.0);
            Some(Entry::Abort(Arc::new(aimux_core::AbortSignal::new())))
        } else {
            None
        }
    }
}
static CALLS: AtomicUsize = AtomicUsize::new(0);
fn probe(_: Ctx, _: Target, _: Value) -> CallFuture {
    CALLS.fetch_add(1, Ordering::SeqCst);
    Box::pin(async { Ok(Value::Null) })
}
fn stream_probe(_: Ctx, _: Target, _: Value) -> Result<EventStream, OpsError> {
    CALLS.fetch_add(1, Ordering::SeqCst);
    Ok(Box::pin(futures::stream::iter([Ok(
        json!({"probe": true}),
    )])))
}
fn ctx(owner: Owner) -> Ctx {
    Ctx { owner, abort: None }
}
fn invalid(
    result: Result<(), OpsError>,
    expected: &[HandleKind],
    actual: Option<HandleKind>,
    label: &str,
) {
    let error = result.expect_err(label);
    match &error {
        OpsError::Binding(BindingError::InvalidHandle {
            expected: found,
            actual: kind,
        }) => {
            assert_eq!(found, expected, "{label}");
            assert_eq!(*kind, actual, "{label}");
        }
        error => panic!("{label}: expected InvalidHandle, got {error}"),
    }
}
async fn invoke(
    dispatcher: &Dispatcher<'_>,
    registry: &Registry,
    owner: Owner,
    id: HandleId,
    name: &str,
    mode: OpMode,
) -> Result<(), OpsError> {
    match mode {
        OpMode::Stream => {
            let frames = dispatcher
                .stream(ctx(owner), id, name, json!({}))?
                .collect::<Vec<_>>()
                .await;
            assert_eq!(frames.len(), 2);
            Ok(())
        }
        OpMode::BinaryPush => {
            // Generic JSON dispatch rejects BinaryPush for every target.
            let error = dispatcher
                .call(ctx(owner), id, name, json!({}))
                .await
                .unwrap_err();
            assert!(matches!(
                &error,
                OpsError::Protocol(ProtocolError::InvalidOp(_))
            ));
            // Native binary admission uses this identical owner-aware typed resolver.
            registry
                .get(owner, id, &[HandleKind::Operation])
                .map(|_| ())
        }
        _ => dispatcher
            .call(ctx(owner), id, name, json!({}))
            .await
            .map(|_| ()),
    }
}

#[tokio::test]
async fn every_manifest_op_crosses_every_kind_and_invalid_handle_state() {
    let manifest = aimux_ops::manifest::manifest();
    let operations = manifest["ops"].as_array().unwrap();
    assert_eq!(
        operations.len(),
        72,
        "RFC inventory plus Decision operations"
    );
    let table: Vec<_> = operations
        .iter()
        .map(|op| {
            let descriptor = dispatch::descriptor(op["name"].as_str().unwrap()).unwrap();
            let handler = if descriptor.mode == OpMode::Stream {
                Handler::Stream(stream_probe)
            } else {
                Handler::Call(probe)
            };
            (descriptor, handler)
        })
        .collect();
    let registry = Registry::default();
    let dispatcher = Dispatcher::new(&registry, &table);
    let owner = Owner::new_node().unwrap();
    let other = Owner::new_python().unwrap();
    let kinds = HandleKind::ALL;
    assert_eq!(
        manifest["handleKinds"],
        serde_json::to_value(kinds).unwrap()
    );
    let mut checked = 0;
    for (descriptor, _) in &table {
        let name = descriptor.name;
        let expected = descriptor.allowed_handle_kinds;
        for &kind in kinds {
            let id = registry.insert(owner, kind);
            let before = CALLS.load(Ordering::SeqCst);
            let result = invoke(&dispatcher, &registry, owner, id, name, descriptor.mode).await;
            let label = format!("{name} x {kind:?}");
            if name == "handle.drop" || expected.contains(&kind) {
                assert!(result.is_ok(), "{label}");
                assert_eq!(
                    CALLS.load(Ordering::SeqCst) - before,
                    usize::from(name != "handle.drop" && descriptor.mode != OpMode::BinaryPush),
                    "{label}"
                );
            } else {
                invalid(result, expected, Some(kind), &label);
                assert_eq!(CALLS.load(Ordering::SeqCst), before, "{label}");
            }
            if name == "handle.drop" {
                assert!(registry.get(owner, id, &[kind]).is_err());
            }
            drop(registry.remove(owner, id));
            checked += 1;
        }
        let dropped = registry.insert(owner, kinds[0]);
        drop(registry.remove(owner, dropped));
        let foreign = registry.insert(other, kinds[0]);
        for (label, id) in [
            ("global", HandleId(0)),
            ("unknown", HandleId(u64::MAX)),
            ("dropped", dropped),
            ("foreign", foreign),
        ] {
            let before = CALLS.load(Ordering::SeqCst);
            let result = invoke(&dispatcher, &registry, owner, id, name, descriptor.mode).await;
            if name == "handle.drop" || (id.0 == 0 && expected.is_empty()) {
                assert!(result.is_ok(), "{name} x {label}");
                assert_eq!(
                    CALLS.load(Ordering::SeqCst) - before,
                    usize::from(name != "handle.drop")
                );
            } else {
                invalid(result, expected, None, &format!("{name} x {label}"));
                assert_eq!(CALLS.load(Ordering::SeqCst), before);
            }
            checked += 1;
        }
        assert!(registry.get(other, foreign, &[kinds[0]]).is_ok());
        drop(registry.remove(other, foreign));
    }
    assert_eq!(checked, operations.len() * (kinds.len() + 4));
    eprintln!("gate matrix: {checked} op/handle combinations");
}

#[test]
fn production_registry_uses_descriptor_allowlists_and_hides_foreign_handles() {
    let registry = HandleRegistry::default();
    let owner = Owner::new_node().unwrap();
    let other = Owner::new_python().unwrap();
    let id = registry
        .insert(
            owner,
            Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
        )
        .unwrap();
    let dropped = registry
        .insert(
            owner,
            Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
        )
        .unwrap();
    drop(registry.remove(owner, dropped));
    for op in aimux_ops::manifest::manifest()["ops"].as_array().unwrap() {
        let name = op["name"].as_str().unwrap();
        let expected = dispatch::descriptor(name).unwrap().allowed_handle_kinds;
        let result = registry.get(owner, id, expected).map(|_| ());
        if expected.contains(&HandleKind::Abort) {
            assert!(result.is_ok());
        } else {
            invalid(result, expected, Some(HandleKind::Abort), name);
        }
        for (caller, id) in [
            (owner, HandleId(0)),
            (owner, HandleId(u64::MAX)),
            (owner, dropped),
            (other, id),
        ] {
            invalid(
                registry.get(caller, id, expected).map(|_| ()),
                expected,
                None,
                name,
            );
        }
    }
}

#[tokio::test]
async fn production_dispatch_rejects_invalid_handles_before_acceptance() {
    let registry = HandleRegistry::global();
    let owner = Owner::new_node().unwrap();
    let other = Owner::new_node().unwrap();
    let id = registry
        .insert(
            owner,
            Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
        )
        .unwrap();
    let dropped = registry
        .insert(
            owner,
            Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
        )
        .unwrap();
    drop(registry.remove(owner, dropped));
    let foreign = registry
        .insert(
            other,
            Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
        )
        .unwrap();
    for op in aimux_ops::manifest::manifest()["ops"].as_array().unwrap() {
        let name = op["name"].as_str().unwrap();
        if name == "handle.drop" {
            continue;
        }
        let descriptor = dispatch::descriptor(name).unwrap();
        let expected = descriptor.allowed_handle_kinds;
        let mut cases = vec![(HandleId(u64::MAX), None), (dropped, None), (foreign, None)];
        if !expected.is_empty() {
            cases.push((HandleId(0), None));
        }
        if !expected.contains(&HandleKind::Abort) {
            cases.push((id, Some(HandleKind::Abort)));
        }
        for (handle, actual) in cases {
            let result = if descriptor.mode == OpMode::Stream {
                dispatch::stream(ctx(owner), handle, name, json!({})).map(|_| ())
            } else if descriptor.mode == OpMode::BinaryPush {
                dispatch::operation_push(&ctx(owner), handle, &[])
            } else {
                dispatch::call(ctx(owner), handle, name, json!({}))
                    .await
                    .map(|_| ())
            };
            invalid(result, expected, actual, name);
        }
    }
    registry.release_owner(owner);
    registry.release_owner(other);
}
