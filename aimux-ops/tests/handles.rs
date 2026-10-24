//! Built-in handle capabilities exercised through shared dispatch.
use std::sync::Arc;
use std::time::Duration;

use aimux_core::download::PlannedDownload;
use aimux_core::trace::RingTraceStore;
use aimux_core::{AbortSignal, AiMuxError};
use aimux_ops::dispatch::{self, Ctx};
use aimux_ops::envelope::{BindingError, OpsError, ProtocolError};
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use aimux_ops::op::{Cancellation, OpMode, RequestBytePolicy, Scope};
use serde_json::{Value, json};

struct TestOwner(Owner);
impl TestOwner {
    fn new() -> Self {
        Self(Owner::new_node().unwrap())
    }

    async fn call(&self, id: HandleId, op: &str, params: Value) -> Result<Value, OpsError> {
        dispatch::call(ctx(self.0), id, op, params).await
    }

    async fn create(&self, op: &str, params: Value) -> Value {
        self.call(HandleId(0), op, params).await.unwrap()
    }

    fn get(&self, result: &Value, kind: HandleKind) -> Entry {
        HandleRegistry::global()
            .get(self.0, id(result), &[kind])
            .unwrap()
    }
}
impl Drop for TestOwner {
    fn drop(&mut self) {
        HandleRegistry::global().release_owner(self.0);
    }
}

fn ctx(owner: Owner) -> Ctx {
    Ctx { owner, abort: None }
}
fn id(value: &Value) -> HandleId {
    serde_json::from_value(value["handle"].clone()).unwrap()
}
fn invalid_params(result: Result<Value, OpsError>) {
    let error = result.unwrap_err();
    assert!(matches!(
        &error,
        OpsError::Protocol(ProtocolError::InvalidParams { .. })
    ));
}
fn invalid_handle(result: Result<Value, OpsError>, actual: Option<HandleKind>) -> Value {
    let error = result.unwrap_err();
    assert!(
        matches!(&error, OpsError::Binding(BindingError::InvalidHandle { actual: found, .. }) if *found == actual)
    );
    serde_json::to_value(error.envelope()).unwrap()
}

#[tokio::test]
async fn abort_is_idempotent_and_signals_another_thread() {
    let owner = TestOwner::new();
    let result = owner.create("abort.create", json!({})).await;
    assert_eq!(result["kind"], "Abort");
    let Entry::Abort(signal) = owner.get(&result, HandleKind::Abort) else {
        panic!("wrong kind")
    };
    let waiter = tokio::spawn(signal.cancelled());
    let handle = id(&result);
    let token = owner.0;
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(dispatch::call(
                ctx(token),
                handle,
                "abort.signal",
                json!({}),
            ))
            .unwrap();
    })
    .join()
    .unwrap();
    assert_eq!(
        owner.call(handle, "abort.signal", json!({})).await.unwrap(),
        Value::Null
    );
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .unwrap()
        .unwrap();
    assert!(signal.is_aborted());
}

#[tokio::test]
async fn dropping_an_abort_preserves_cloned_capabilities_without_signalling() {
    let owner = TestOwner::new();
    let result = owner.create("abort.create", json!({})).await;
    let Entry::Abort(signal) = owner.get(&result, HandleKind::Abort) else {
        panic!("wrong kind")
    };
    invalid_params(
        owner
            .call(id(&result), "handle.drop", json!({"force":true}))
            .await,
    );
    assert_eq!(
        owner.get(&result, HandleKind::Abort).kind(),
        HandleKind::Abort
    );
    for handle in [id(&result), id(&result), HandleId(0), HandleId(u64::MAX)] {
        assert_eq!(
            owner.call(handle, "handle.drop", json!({})).await.unwrap(),
            Value::Null
        );
    }
    invalid_handle(
        owner.call(id(&result), "abort.signal", json!({})).await,
        None,
    );
    assert!(!signal.is_aborted());
    signal.abort();
    assert!(signal.is_aborted());
}

#[tokio::test]
async fn foreign_owner_cannot_signal_or_drop_a_capability() {
    let owner = TestOwner::new();
    let other = TestOwner::new();
    let result = owner.create("abort.create", json!({})).await;
    let foreign = invalid_handle(
        other.call(id(&result), "abort.signal", json!({})).await,
        None,
    );
    let unknown = invalid_handle(
        other
            .call(HandleId(u64::MAX), "abort.signal", json!({}))
            .await,
        None,
    );
    assert_eq!(foreign, unknown);
    assert_eq!(
        other
            .call(id(&result), "handle.drop", json!({}))
            .await
            .unwrap(),
        Value::Null
    );
    let Entry::Abort(signal) = owner.get(&result, HandleKind::Abort) else {
        panic!("wrong kind")
    };
    assert!(!signal.is_aborted());
}

#[tokio::test]
async fn transports_have_precise_kinds_and_decimal_ids() {
    let owner = TestOwner::new();
    for (kind, expected) in [
        ("fetch", HandleKind::Fetch),
        ("ws", HandleKind::WebSocket),
        ("download", HandleKind::Download),
    ] {
        let result = owner.create("transport.create", json!({"kind":kind})).await;
        let handle = result["handle"].as_str().unwrap();
        assert!(handle.bytes().all(|b| b.is_ascii_digit()));
        assert_ne!(id(&result), HandleId(0));
        assert_eq!(result["kind"], serde_json::to_value(expected).unwrap());
        assert_eq!(owner.get(&result, expected).kind(), expected);
    }
}

#[tokio::test]
async fn proxy_settings_reuse_existing_transport_configuration() {
    let owner = TestOwner::new();
    for kind in ["fetch", "ws"] {
        let result = owner
            .create(
                "transport.create",
                json!({"kind":kind,"proxy":{
                    "http_url":"http://proxy.invalid:80", "https_url":"http://proxy.invalid:81",
                    "all_url":null, "no_proxy":"localhost"
                }}),
            )
            .await;
        assert!(id(&result).0 > 0);
    }
}

#[tokio::test]
async fn builtin_download_keeps_the_guard_and_model_supported_url_policy() {
    let owner = TestOwner::new();
    let result = owner
        .create("transport.create", json!({"kind":"download","maxBytes":7}))
        .await;
    let Entry::Download(download) = owner.get(&result, HandleKind::Download) else {
        panic!("wrong kind")
    };
    let url = "http://127.0.0.1:9/private".to_owned();
    assert!(matches!(
        download
            .download(vec![PlannedDownload {
                url: url.clone(),
                is_url_supported_by_model: false
            }])
            .await,
        Err(AiMuxError::InvalidArgument { .. })
    ));
    assert_eq!(
        download
            .download(vec![PlannedDownload {
                url,
                is_url_supported_by_model: true
            }])
            .await
            .unwrap(),
        [None]
    );
    assert!(download.download(Vec::new()).await.unwrap().is_empty());
}

#[tokio::test]
async fn builtin_download_retains_the_resolved_abort_signal() {
    let owner = TestOwner::new();
    let signal = AbortSignal::new();
    signal.abort();
    let mut context = ctx(owner.0);
    context.abort = Some(signal);
    let result = dispatch::call(
        context,
        HandleId(0),
        "transport.create",
        json!({"kind":"download"}),
    )
    .await
    .unwrap();
    let Entry::Download(download) = owner.get(&result, HandleKind::Download) else {
        panic!("wrong kind")
    };
    assert!(matches!(
        download
            .download(vec![PlannedDownload {
                url: "https://example.invalid/file".into(),
                is_url_supported_by_model: false
            }])
            .await,
        Err(AiMuxError::Aborted(_))
    ));
}

#[tokio::test]
async fn control_objects_reject_unknown_fields_and_callbacks() {
    let owner = TestOwner::new();
    invalid_params(
        owner
            .call(HandleId(0), "abort.create", json!({"callback":true}))
            .await,
    );
    let abort = owner.create("abort.create", json!({})).await;
    invalid_params(owner.call(id(&abort), "abort.signal", Value::Null).await);
    invalid_params(
        owner
            .call(id(&abort), "abort.signal", json!({"extra":true}))
            .await,
    );
    for value in [
        json!({"kind":"fetch", "callback":{}}),
        json!({"kind":"fetch", "proxy":{"callback":true}}),
        json!({"kind":"fetch", "proxy":null}),
        json!({"kind":"fetch", "proxy":{"httpUrl":"http://proxy.invalid"}}),
        json!({"kind":"ws", "maxBytes":5}),
        json!({"kind":"download", "proxy":{}}),
        json!({"kind":"download", "maxBytes":-1}),
        json!({"kind":"download", "maxBytes":1.5}),
        json!({"kind":"FETCH"}),
        json!({}),
    ] {
        invalid_params(owner.call(HandleId(0), "transport.create", value).await);
    }
}

#[tokio::test]
async fn credential_creation_is_future_and_never_exports_supplied_material() {
    let owner = TestOwner::new();
    let error = owner
        .call(
            HandleId(0),
            "credential.create",
            json!({"kind":"secret-sentinel"}),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        OpsError::AiMux(AiMuxError::UnsupportedFunctionality(_))
    ));
    assert_eq!(error.code(), 8);
    assert!(
        !serde_json::to_string(&error.envelope())
            .unwrap()
            .contains("secret-sentinel")
    );
    let error = owner
        .call(
            HandleId(0),
            "credential.create",
            json!({"kind":"static","secret":"secret-sentinel"}),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), 301);
    assert!(
        !serde_json::to_string(&error.envelope())
            .unwrap()
            .contains("secret-sentinel")
    );
    assert_eq!(
        dispatch::descriptor("credential.create").unwrap().scope,
        Scope::Future
    );
}

#[tokio::test]
async fn trace_store_resolution_hides_foreign_kinds_and_rejects_wrong_kinds() {
    let owner = TestOwner::new();
    let other = TestOwner::new();
    let store = HandleRegistry::global()
        .insert(owner.0, Entry::TraceStore(Arc::new(RingTraceStore::new())))
        .unwrap();
    let request = json!({"kind":"trace","traceStore":store});
    let foreign = invalid_handle(
        other.call(HandleId(0), "middleware.create", request).await,
        None,
    );
    let unknown = invalid_handle(
        other
            .call(
                HandleId(0),
                "middleware.create",
                json!({"kind":"trace","traceStore":u64::MAX.to_string()}),
            )
            .await,
        None,
    );
    assert_eq!(foreign, unknown);
    let abort = owner.create("abort.create", json!({})).await;
    invalid_handle(
        owner
            .call(
                HandleId(0),
                "middleware.create",
                json!({"kind":"trace","traceStore":abort["handle"]}),
            )
            .await,
        Some(HandleKind::Abort),
    );
}

#[tokio::test]
async fn middleware_validates_control_fields_and_handle_wire_format() {
    let owner = TestOwner::new();
    for value in [
        json!({"kind":"trace"}),
        json!({"kind":"trace","traceStore":1}),
        json!({"kind":"trace","traceStore":"+1"}),
        json!({"kind":"trace","traceStore":""}),
        json!({"kind":"trace","traceStore":"18446744073709551616"}),
        json!({"kind":"trace","callback":true}),
        json!({"kind":"trace","auditMode":"invalid"}),
    ] {
        invalid_params(owner.call(HandleId(0), "middleware.create", value).await);
    }
    let error = owner
        .call(
            HandleId(0),
            "middleware.create",
            json!({"kind":"hostCallback"}),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        OpsError::AiMux(AiMuxError::UnsupportedFunctionality(_))
    ));
}

#[tokio::test]
async fn trace_middleware_creation_retains_store_and_audit_configuration() {
    let owner = TestOwner::new();
    let store = Arc::new(RingTraceStore::new());
    let handle = HandleRegistry::global()
        .insert(owner.0, Entry::TraceStore(store.clone()))
        .unwrap();
    let references = Arc::strong_count(&store);
    for audit_mode in ["disabled", "shared", "strict"] {
        let result = owner.create("middleware.create", json!({"kind":"trace","traceStore":handle,"scopeName":"test","auditMode":audit_mode})).await;
        assert_eq!(result["kind"], "Middleware");
        assert_eq!(Arc::strong_count(&store), references + 1);
        assert!(
            HandleRegistry::global()
                .remove(owner.0, id(&result))
                .is_some()
        );
        assert_eq!(Arc::strong_count(&store), references);
    }
    assert_eq!(store.len(), 0);
}

#[test]
fn registrations_publish_complete_metadata() {
    let names = [
        "abort.create",
        "abort.signal",
        "transport.create",
        "credential.create",
        "middleware.create",
        "handle.drop",
    ];
    for name in names {
        let op = dispatch::descriptor(name).unwrap();
        assert_eq!(op.mode, OpMode::Call);
        assert_eq!(op.cancellation, Cancellation::None);
        assert_eq!(op.request_byte_policy, RequestBytePolicy::Json);
        assert!(op.terminal_type.is_none());
        assert!(op.side_effects);
        assert!(op.required_capabilities.is_empty());
        assert_ne!((op.request_type.id)().into_owned(), "AnyValue");
        assert_ne!((op.result_type.id)().into_owned(), "AnyValue");
    }
    assert_eq!(
        dispatch::descriptor("abort.signal")
            .unwrap()
            .allowed_handle_kinds,
        [HandleKind::Abort]
    );
    assert_eq!(
        dispatch::descriptor("handle.drop")
            .unwrap()
            .allowed_handle_kinds
            .len(),
        HandleKind::ALL.len()
    );
    assert_eq!(
        dispatch::descriptor("middleware.create")
            .unwrap()
            .runtime_handle_fields,
        [("traceStore", &[HandleKind::TraceStore][..])]
    );
}
