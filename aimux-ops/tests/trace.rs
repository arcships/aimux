//! Family gates use real dispatch, owner resolution and canonical core queries.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use aimux_core::trace::{RingTraceStore, TraceFilter, TraceRecord, TraceSink};
use aimux_ops::dispatch::{self, Ctx};
use aimux_ops::envelope::{BindingError, OpsError, ProtocolError};
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use aimux_ops::op::{Cancellation, OpMode, RequestBytePolicy, Scope};
use serde_json::{Value, json};

fn ctx(owner: Owner) -> Ctx {
    Ctx {
        owner,
        abort: None,
        now: SystemTime::now,
    }
}

struct Trace {
    owner: Owner,
    id: HandleId,
    store: Arc<RingTraceStore>,
}

impl Trace {
    async fn new(params: Value) -> Self {
        let owner = Owner::new_node().unwrap();
        let value = dispatch::call(ctx(owner), HandleId(0), "trace.create", params)
            .await
            .unwrap();
        assert!(value.is_string(), "owned handles use decimal-string JSON");
        let id: HandleId = serde_json::from_value(value).unwrap();
        let Entry::TraceStore(store) = HandleRegistry::global()
            .get(owner, id, &[HandleKind::TraceStore])
            .unwrap()
        else {
            panic!("trace.create must retain the canonical store");
        };
        Self { owner, id, store }
    }

    async fn call(&self, op: &str, params: Value) -> Result<Value, OpsError> {
        dispatch::call(ctx(self.owner), self.id, op, params).await
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        HandleRegistry::global().release_owner(self.owner);
    }
}

fn record(id: &str, session: &str, provider: &str, sent: i64) -> TraceRecord {
    serde_json::from_value(json!({
        "provider": provider, "model": "model", "callId": id,
        "sessionId": session, "sentAtUnixMs": sent,
        "fingerprint": {"bodyHash": "abc", "lenBytes": 16,
            "blockSize": 16, "blockHashes": ["01"], "tokenEstimate": 4},
        "usage": {"inputTotal": 100, "cacheRead": 25}
    }))
    .unwrap()
}

#[tokio::test]
async fn create_uses_real_store_capacity_defaults_and_owned_handle() {
    let trace = Trace::new(json!({"capacity":2,"perScopeCapacity":1,"auditMode":"off"})).await;
    assert_ne!(trace.id, HandleId(0));
    for n in 0..3 {
        trace.store.record(record(&n.to_string(), "s", "p", n));
    }
    assert_eq!(trace.store.len(), 2);
    assert_eq!(
        trace
            .call("trace.sessionTrajectory", json!({"sessionId":"s"}))
            .await
            .unwrap()[0]["callId"],
        "1"
    );
    let defaults = Trace::new(json!({})).await;
    assert_eq!(
        defaults.call("trace.aggregate", json!({})).await.unwrap(),
        json!([])
    );
    assert_eq!(defaults.call("trace.export", json!({})).await.unwrap(), "");
}

#[tokio::test]
async fn aggregate_matches_six_canonical_filter_queries() {
    let trace = Trace::new(json!({"capacity":4})).await;
    trace.store.record(record("a", "session-①", "provider", 10));
    trace.store.record(record("b", "other", "provider", 20));
    trace.store.record(record("c", "session-①", "other", 30));
    for filter in [
        json!({}),
        json!({"provider":"provider"}),
        json!({"model":"missing"}),
        json!({"sessionId":"session-①"}),
        json!({"sinceUnixMs":20}),
        json!({"provider":"provider","sessionId":"session-①"}),
    ] {
        let canonical: TraceFilter = serde_json::from_value(filter.clone()).unwrap();
        assert_eq!(
            trace
                .call("trace.aggregate", json!({"filter":filter}))
                .await
                .unwrap(),
            serde_json::to_value(trace.store.aggregate(&canonical)).unwrap()
        );
    }
    assert_eq!(
        trace
            .call("trace.aggregate", json!({}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn sessions_export_and_idempotent_clear_preserve_ffi_query_semantics() {
    let trace = Trace::new(json!({"capacity":4})).await;
    trace.store.record(record("a", "203", "provider", 10));
    trace.store.record(record("b", "203", "provider", 20));
    assert_eq!(
        trace
            .call("trace.sessionChain", json!({"sessionId":"203"}))
            .await
            .unwrap(),
        serde_json::to_value(trace.store.session_chain("203").unwrap()).unwrap()
    );
    let trajectory = trace
        .call("trace.sessionTrajectory", json!({"sessionId":"203"}))
        .await
        .unwrap();
    assert_eq!(
        trajectory,
        serde_json::to_value(trace.store.session_cache_trajectory("203")).unwrap()
    );
    assert_eq!(trajectory[0]["hitRate"], json!(0.25));
    assert_eq!(trajectory[1]["callId"], "b");
    assert_eq!(
        trace
            .call("trace.sessionTrajectory", json!({"sessionId":"missing"}))
            .await
            .unwrap(),
        json!([])
    );
    let error = trace
        .call("trace.sessionChain", json!({"sessionId":"missing"}))
        .await
        .unwrap_err();
    assert!(
        matches!(error.unprojected(), OpsError::AiMux(aimux_core::AiMuxError::InvalidArgument(message)) if message == "unknown session")
    );
    let envelope = serde_json::to_value(error.envelope(SystemTime::now())).unwrap();
    assert_eq!(envelope["kind"], "aimux");
    assert_eq!(envelope["code"], 5);
    assert_eq!(envelope["error"]["name"], "AI_InvalidArgumentError");

    let exported = trace.call("trace.export", json!({})).await.unwrap();
    let mut canonical = Vec::new();
    trace.store.export_jsonl(&mut canonical).unwrap();
    assert_eq!(exported, String::from_utf8(canonical).unwrap());
    let lines: Vec<Value> = exported
        .as_str()
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["callId"], "a");
    assert!(lines[0].get("scopeKey").is_none());
    for _ in 0..2 {
        assert_eq!(
            trace.call("trace.clear", json!({})).await.unwrap(),
            Value::Null
        );
    }
    assert!(trace.store.is_empty());
    assert_eq!(trace.call("trace.export", json!({})).await.unwrap(), "");
    assert_eq!(
        trace.call("trace.aggregate", json!({})).await.unwrap(),
        json!([])
    );
    assert_eq!(
        trace
            .call("trace.sessionTrajectory", json!({"sessionId":"203"}))
            .await
            .unwrap(),
        json!([])
    );
}

#[tokio::test]
async fn invalid_controls_fail_before_mutation() {
    let trace = Trace::new(json!({"capacity":1})).await;
    trace.store.record(record("a", "s", "p", 1));
    for (op, params) in [
        ("trace.create", json!({"capacity":0})),
        ("trace.create", json!({"perScopeCapacity":0})),
        ("trace.create", json!({"capacity":-1})),
        ("trace.create", json!({"capacity":1.5})),
        ("trace.create", json!({"auditMode":"unknown"})),
        ("trace.create", json!({"audit_mode":"strict"})),
        ("trace.create", json!({"extra":true})),
        ("trace.aggregate", json!({"filter":{"extra":true}})),
        ("trace.aggregate", json!({"filter":{"session_id":"s"}})),
        ("trace.aggregate", json!({"filter":{"sinceUnixMs":"20"}})),
        ("trace.aggregate", json!({"filter":null})),
        ("trace.aggregate", json!({"filter":[]})),
        ("trace.aggregate", json!({"extra":true})),
        ("trace.sessionChain", json!({"sessionId":203})),
        (
            "trace.sessionTrajectory",
            json!({"sessionId":"s","extra":0}),
        ),
        ("trace.sessionChain", json!({})),
        ("trace.export", json!({"format":"json"})),
        ("trace.clear", json!({"sessionId":"s"})),
        ("trace.clear", Value::Null),
    ] {
        let handle = if op == "trace.create" {
            HandleId(0)
        } else {
            trace.id
        };
        let error = dispatch::call(ctx(trace.owner), handle, op, params)
            .await
            .unwrap_err();
        assert!(
            matches!(
                error.unprojected(),
                OpsError::Protocol(ProtocolError::InvalidParams(_))
            ),
            "{op}: {error}"
        );
        assert_eq!(error.code(), 301);
        assert!(matches!(error, OpsError::Projected(_)));
        assert_eq!(trace.store.len(), 1, "{op} mutated data on rejection");
    }
}

#[tokio::test]
async fn trace_store_retains_audit_policy_for_middleware() {
    use aimux_core::trace::store::TraceAuditMode;
    let owner = Owner::new_node().unwrap();
    for (mode, expected) in [
        ("shared", TraceAuditMode::Shared),
        ("strict", TraceAuditMode::Strict),
    ] {
        let result = dispatch::call(
            ctx(owner),
            HandleId(0),
            "trace.create",
            json!({"auditMode":mode}),
        )
        .await
        .unwrap();
        let handle = serde_json::from_value(result).unwrap();
        let Entry::TraceStore(store) = HandleRegistry::global()
            .get(owner, handle, &[HandleKind::TraceStore])
            .unwrap()
        else {
            panic!("trace store")
        };
        assert_eq!(store.audit_mode(), expected);
    }
    assert_eq!(HandleRegistry::global().release_owner(owner), 2);
}

#[tokio::test]
async fn trace_ownership_and_drop_use_real_dispatch_without_disclosure() {
    let trace = Trace::new(json!({"capacity":2})).await;
    trace.store.record(record("a", "203", "p", 1));
    let foreign = Owner::new_python().unwrap();
    let wrong = HandleRegistry::global()
        .insert(
            trace.owner,
            Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
        )
        .unwrap();
    for (op, params) in [
        ("trace.aggregate", json!({})),
        ("trace.sessionChain", json!({"sessionId":"203"})),
        ("trace.sessionTrajectory", json!({"sessionId":"203"})),
        ("trace.export", json!({})),
        ("trace.clear", json!({})),
    ] {
        let foreign_error = dispatch::call(ctx(foreign), trace.id, op, params.clone())
            .await
            .unwrap_err();
        let unknown_error = dispatch::call(ctx(foreign), HandleId(u64::MAX), op, params.clone())
            .await
            .unwrap_err();
        assert_eq!(
            serde_json::to_value(foreign_error.envelope(SystemTime::now())).unwrap(),
            serde_json::to_value(unknown_error.envelope(SystemTime::now())).unwrap()
        );
        for id in [HandleId(0), wrong] {
            let error = dispatch::call(ctx(trace.owner), id, op, params.clone())
                .await
                .unwrap_err();
            assert!(matches!(
                error.unprojected(),
                OpsError::Binding(BindingError::InvalidHandle { .. })
            ));
        }
    }
    assert_eq!(trace.store.len(), 1);
    assert_eq!(
        dispatch::call(ctx(foreign), trace.id, "handle.drop", json!({}))
            .await
            .unwrap(),
        Value::Null
    );
    assert!(
        HandleRegistry::global()
            .get(trace.owner, trace.id, &[HandleKind::TraceStore])
            .is_ok()
    );
    for _ in 0..2 {
        assert_eq!(
            trace.call("handle.drop", json!({})).await.unwrap(),
            Value::Null
        );
    }
    let error = trace.call("trace.export", json!({})).await.unwrap_err();
    assert_eq!(error.code(), 203);
    // The retained Arc survives handle removal; no registry lock is needed.
    assert_eq!(trace.store.len(), 1);
    HandleRegistry::global().release_owner(foreign);
}

static CLOCK_CALLS: AtomicUsize = AtomicUsize::new(0);
fn counted_clock() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(CLOCK_CALLS.fetch_add(1, Ordering::SeqCst) as u64)
}

#[tokio::test]
async fn trace_errors_are_projected_once_by_dispatch() {
    let trace = Trace::new(json!({})).await;
    let mut context = ctx(trace.owner);
    context.now = counted_clock;
    CLOCK_CALLS.store(0, Ordering::SeqCst);
    let error = dispatch::call(
        context,
        trace.id,
        "trace.sessionChain",
        json!({"sessionId":"missing"}),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, OpsError::Projected(_)));
    assert_eq!(CLOCK_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(
        serde_json::to_value(error.envelope(SystemTime::UNIX_EPOCH)).unwrap(),
        serde_json::to_value(error.envelope(SystemTime::now())).unwrap()
    );
    assert_eq!(CLOCK_CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn six_trace_descriptors_publish_concrete_types_and_a5_fields() {
    let manifest = aimux_ops::manifest::manifest();
    let names = [
        "trace.create",
        "trace.aggregate",
        "trace.sessionChain",
        "trace.sessionTrajectory",
        "trace.export",
        "trace.clear",
    ];
    assert_eq!(
        manifest["ops"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|op| op["name"].as_str().unwrap().starts_with("trace."))
            .count(),
        names.len()
    );
    for (index, name) in names.into_iter().enumerate() {
        let descriptor = dispatch::descriptor(name).unwrap();
        assert_eq!(descriptor.mode, OpMode::Call);
        assert_eq!(descriptor.scope, Scope::Current);
        assert_eq!(descriptor.cancellation, Cancellation::None);
        assert_eq!(descriptor.request_byte_policy, RequestBytePolicy::Json);
        assert_eq!(
            descriptor.allowed_handle_kinds,
            if index == 0 {
                &[][..]
            } else {
                &[HandleKind::TraceStore][..]
            }
        );
        assert!(descriptor.runtime_handle_fields.is_empty());
        assert!(descriptor.required_capabilities.is_empty());
        assert!(descriptor.terminal_type.is_none());
        assert!(descriptor.event_type.is_none());
        assert_eq!(
            descriptor.side_effects,
            matches!(name, "trace.create" | "trace.clear")
        );
        assert!(!descriptor.source_symbols.is_empty());
        for ty in [descriptor.request_type, descriptor.result_type] {
            assert_ne!(ty, "serde_json::Value");
            assert!(
                manifest["types"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entry| entry["id"] == ty)
            );
        }
    }
}
