use std::sync::Arc;

use aimux_core::session::{SessionSource, SessionStore};
use aimux_ops::dispatch::{Ctx, call, descriptor};
use aimux_ops::envelope::OpsError;
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use aimux_ops::op::{Cancellation, OpMode, RequestBytePolicy, Scope};
use serde_json::{Value, json};

struct TestOwner(Owner);

impl TestOwner {
    fn new() -> Self {
        Self(Owner::new_node().unwrap())
    }

    fn ctx(&self) -> Ctx {
        Ctx {
            owner: self.0,
            abort: None,
        }
    }

    fn insert(&self, store: Arc<SessionStore>) -> HandleId {
        HandleRegistry::global()
            .insert(self.0, Entry::SessionStore(store))
            .unwrap()
    }
}

impl Drop for TestOwner {
    fn drop(&mut self) {
        HandleRegistry::global().release_owner(self.0);
    }
}

fn assert_error(error: OpsError, code: i32, name: &str) -> Value {
    assert_eq!(error.code(), code);
    let envelope = serde_json::to_value(error.envelope()).unwrap();
    assert_eq!(envelope["error"]["name"], name);
    envelope
}

#[tokio::test]
async fn control_params_are_strict_through_dispatch() {
    let owner = TestOwner::new();
    let handle = owner.insert(Arc::new(SessionStore::new()));
    let invalid = [
        ("sessionStore.create", HandleId(0), json!({})),
        ("sessionStore.create", HandleId(0), json!({"infer": 1})),
        ("sessionStore.create", HandleId(0), json!({"infer": null})),
        (
            "sessionStore.create",
            HandleId(0),
            json!({"infer": false, "extra": 1}),
        ),
        ("sessionStore.attach", handle, json!({"extra": 1})),
        ("sessionStore.list", handle, json!({"extra": 1})),
        ("sessionStore.calls", handle, json!({})),
        ("sessionStore.calls", handle, json!({"sessionId": 1})),
        ("sessionStore.calls", handle, json!({"sessionId": null})),
        ("sessionStore.calls", handle, json!({"session_id": "s"})),
        (
            "sessionStore.calls",
            handle,
            json!({"sessionId": "s", "extra": 1}),
        ),
    ];
    for (op, target, params) in invalid {
        assert_error(
            call(owner.ctx(), target, op, params).await.unwrap_err(),
            301,
            "InvalidParams",
        );
    }
    for (op, target) in [
        ("sessionStore.create", HandleId(0)),
        ("sessionStore.attach", handle),
        ("sessionStore.list", handle),
        ("sessionStore.calls", handle),
    ] {
        for params in [Value::Null, json!([]), json!("bad")] {
            assert_error(
                call(owner.ctx(), target, op, params).await.unwrap_err(),
                301,
                "InvalidParams",
            );
        }
    }
}

#[tokio::test]
async fn queries_use_selected_store_and_preserve_core_wire_and_bounds() {
    let owner = TestOwner::new();
    let store = Arc::new(SessionStore::with_capacity(2, 2));
    let handle = owner.insert(store.clone());
    let empty = owner.insert(Arc::new(SessionStore::new()));
    assert_eq!(
        call(owner.ctx(), handle, "sessionStore.list", json!({}))
            .await
            .unwrap(),
        json!([])
    );
    for id in ["unknown", "", "会话😀"] {
        assert_eq!(
            call(
                owner.ctx(),
                handle,
                "sessionStore.calls",
                json!({"sessionId": id})
            )
            .await
            .unwrap(),
            json!([])
        );
    }
    store.append("会话😀", Some("first"), SessionSource::Explicit);
    let second = store.append("会话😀", Some("second"), SessionSource::Explicit);
    let third = store.append("会话😀", Some("third"), SessionSource::Explicit);
    let expected = json!([
        {"callId":"second", "step":1, "recordedAt":second.recorded_at},
        {"callId":"third", "step":2, "recordedAt":third.recorded_at}
    ]);
    let actual = call(
        owner.ctx(),
        handle,
        "sessionStore.calls",
        json!({"sessionId":"会话😀"}),
    )
    .await
    .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        actual,
        serde_json::to_value(store.session_calls("会话😀")).unwrap()
    );
    assert_eq!(
        call(owner.ctx(), handle, "sessionStore.list", json!({}))
            .await
            .unwrap(),
        json!([
            {"sessionId":"会话😀", "source":"Explicit", "calls":expected}
        ])
    );
    assert_eq!(
        call(owner.ctx(), empty, "sessionStore.list", json!({}))
            .await
            .unwrap(),
        json!([])
    );
    store.append("", Some("empty-id"), SessionSource::Inferred);
    assert_eq!(
        call(
            owner.ctx(),
            handle,
            "sessionStore.calls",
            json!({"sessionId":""})
        )
        .await
        .unwrap()[0]["callId"],
        "empty-id"
    );
    store.append("new", Some("new"), SessionSource::Explicit);
    assert_eq!(
        call(
            owner.ctx(),
            handle,
            "sessionStore.calls",
            json!({"sessionId":"会话😀"})
        )
        .await
        .unwrap(),
        json!([])
    );
    let mut actual = call(owner.ctx(), handle, "sessionStore.list", json!({}))
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    actual.sort_by_key(|view| view["sessionId"].as_str().unwrap().to_owned());
    let mut expected = serde_json::to_value(store.list_sessions())
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    expected.sort_by_key(|view| view["sessionId"].as_str().unwrap().to_owned());
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 2);
}

#[tokio::test]
async fn owner_checks_hide_foreign_handles_and_drop_is_idempotent() {
    let owner = TestOwner::new();
    let foreign = TestOwner::new();
    let handle = owner.insert(Arc::new(SessionStore::new()));
    let wrong_kind = HandleRegistry::global()
        .insert(
            owner.0,
            Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
        )
        .unwrap();
    for (op, params) in [
        ("sessionStore.attach", json!({})),
        ("sessionStore.list", json!({})),
        ("sessionStore.calls", json!({"sessionId":"s"})),
    ] {
        let hidden = assert_error(
            call(foreign.ctx(), handle, op, params.clone())
                .await
                .unwrap_err(),
            203,
            "InvalidHandle",
        );
        let unknown = assert_error(
            call(foreign.ctx(), HandleId(u64::MAX), op, params.clone())
                .await
                .unwrap_err(),
            203,
            "InvalidHandle",
        );
        assert_eq!(hidden, unknown);
        assert_eq!(hidden["error"]["actual"], Value::Null);
        let wrong = assert_error(
            call(owner.ctx(), wrong_kind, op, params.clone())
                .await
                .unwrap_err(),
            203,
            "InvalidHandle",
        );
        assert_eq!(wrong["error"]["actual"], "Abort");
        assert_error(
            call(owner.ctx(), HandleId(0), op, params)
                .await
                .unwrap_err(),
            203,
            "InvalidHandle",
        );
    }
    assert_error(
        call(
            owner.ctx(),
            handle,
            "sessionStore.create",
            json!({"infer":false}),
        )
        .await
        .unwrap_err(),
        203,
        "InvalidHandle",
    );
    call(foreign.ctx(), handle, "handle.drop", json!({}))
        .await
        .unwrap();
    assert_eq!(
        call(owner.ctx(), handle, "sessionStore.list", json!({}))
            .await
            .unwrap(),
        json!([])
    );
    for _ in 0..2 {
        assert!(
            call(owner.ctx(), handle, "handle.drop", json!({}))
                .await
                .unwrap()
                .is_null()
        );
    }
    assert_error(
        call(owner.ctx(), handle, "sessionStore.list", json!({}))
            .await
            .unwrap_err(),
        203,
        "InvalidHandle",
    );
}

#[tokio::test]
async fn create_and_attach_preserve_process_global_infer_and_capture_behavior() {
    let owner = TestOwner::new();
    let mut previous = None;
    for infer in [false, true] {
        let result = call(
            owner.ctx(),
            HandleId(0),
            "sessionStore.create",
            json!({"infer":infer}),
        )
        .await
        .unwrap();
        assert_eq!(result.as_object().unwrap().len(), 1);
        let text = result["handle"].as_str().unwrap();
        assert!(text.bytes().all(|byte| byte.is_ascii_digit()));
        let handle: HandleId = serde_json::from_value(result["handle"].clone()).unwrap();
        assert_ne!(handle, HandleId(0));
        if let Some(previous) = previous {
            assert_ne!(handle, previous);
        }
        previous = Some(handle);
        let Entry::SessionStore(store) = HandleRegistry::global()
            .get(owner.0, handle, &[HandleKind::SessionStore])
            .unwrap()
        else {
            unreachable!()
        };
        let inferer = aimux_core::session::session_inferer().unwrap();
        let resolved = inferer.lock().unwrap().resolve(None, &Vec::new());
        assert_eq!(resolved.is_some(), infer);
        assert!(
            !aimux_core::session::session_store()
                .is_some_and(|attached| Arc::ptr_eq(&store, &attached))
        );
        store.append("capture", Some("call"), SessionSource::Explicit);
        assert!(
            call(owner.ctx(), handle, "sessionStore.attach", json!({}))
                .await
                .unwrap()
                .is_null()
        );
        assert!(Arc::ptr_eq(
            &store,
            &aimux_core::session::session_store().unwrap()
        ));
        assert_eq!(
            call(
                owner.ctx(),
                handle,
                "sessionStore.calls",
                json!({"sessionId":"capture"})
            )
            .await
            .unwrap(),
            serde_json::to_value(aimux_core::session::session_calls("capture")).unwrap()
        );
        assert_eq!(
            call(owner.ctx(), handle, "sessionStore.list", json!({}))
                .await
                .unwrap(),
            serde_json::to_value(aimux_core::session::list_sessions()).unwrap()
        );
        call(owner.ctx(), handle, "handle.drop", json!({}))
            .await
            .unwrap();
        assert!(Arc::ptr_eq(
            &store,
            &aimux_core::session::session_store().unwrap()
        ));
    }
}

#[test]
fn descriptors_publish_concrete_types_and_complete_metadata() {
    let manifest = aimux_ops::manifest::manifest();
    for (name, request, result, side_effects) in [
        (
            "sessionStore.create",
            "aimux_ops::ops::session::SessionStoreCreateParams",
            "aimux_ops::ops::session::SessionStoreHandle",
            true,
        ),
        (
            "sessionStore.attach",
            "aimux_ops::ops::operation::EmptyParams",
            "null",
            true,
        ),
        (
            "sessionStore.list",
            "aimux_ops::ops::operation::EmptyParams",
            "[aimux_core::session::SessionView]",
            false,
        ),
        (
            "sessionStore.calls",
            "aimux_ops::ops::session::SessionStoreCallsParams",
            "[aimux_core::session::SessionCall]",
            false,
        ),
    ] {
        let op = descriptor(name).unwrap();
        assert_eq!(op.mode, OpMode::Call);
        assert_eq!((op.request_type.id)().into_owned(), request);
        assert_eq!((op.result_type.id)().into_owned(), result);
        assert_eq!(op.scope, Scope::Current);
        assert_eq!(op.cancellation, Cancellation::None);
        assert_eq!(op.request_byte_policy, RequestBytePolicy::Json);
        assert_eq!(op.side_effects, side_effects);
        assert_eq!(op.result_ownership, "owned");
        assert_eq!(op.retry_owner, "none");
        assert!(op.runtime_handle_fields.is_empty());
        assert!(op.required_capabilities.is_empty());
        assert!(op.event_type.is_none());
        assert!(op.terminal_type.is_none());
        assert!(!op.source_symbols.is_empty());
        assert_eq!(
            op.allowed_handle_kinds,
            if name == "sessionStore.create" {
                &[][..]
            } else {
                &[HandleKind::SessionStore][..]
            }
        );
        for id in [request, result] {
            assert!(
                manifest["types"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|ty| ty["id"] == id)
            );
        }
    }
}
