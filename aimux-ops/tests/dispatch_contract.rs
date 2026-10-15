//! Dispatch, ownership and error projection invariants, with no provider I/O.
use aimux_core::error::ApiCallError;
use aimux_core::{AbortSignal, AiMuxError};
use aimux_ops::dispatch::{CallFuture, Ctx, Dispatcher, EventStream, Handler, Target};
use aimux_ops::envelope::{BindingError, OpsError, ProtocolError};
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use aimux_ops::op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope};
use futures::StreamExt;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, SystemTime};

const fn descriptor(
    name: &'static str,
    mode: OpMode,
    kinds: &'static [HandleKind],
) -> OpDescriptor {
    OpDescriptor {
        name,
        since_ops_version: "1.0",
        mode,
        allowed_handle_kinds: kinds,
        request_type: "Value",
        result_type: "Value",
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::Abort,
        side_effects: false,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        retry_owner: "none",
        source_symbols: &[],
    }
}
fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}
fn ctx(owner: Owner) -> Ctx {
    Ctx {
        owner,
        abort: None,
        now,
    }
}
fn unknown(error: &OpsError) {
    assert!(matches!(
        error.unprojected(),
        OpsError::Binding(BindingError::InvalidHandle { actual: None, .. })
    ));
}
fn success(_: Ctx, target: Target, _: Value) -> CallFuture {
    Box::pin(async move {
        assert!(matches!(target, Target::Global));
        Ok(Value::Null)
    })
}

#[tokio::test]
async fn global_target_and_mode_preflight_are_explicit_and_projected() {
    static GLOBAL: OpDescriptor = descriptor("test.global", OpMode::Call, &[]);
    static BINARY: OpDescriptor =
        descriptor("test.binary", OpMode::BinaryPush, &[HandleKind::Operation]);
    let registry = HandleRegistry::default();
    let owner = Owner::new_node().unwrap();
    let id = registry
        .insert(owner, Entry::Abort(Arc::new(AbortSignal::new())))
        .unwrap();
    let registrations = [
        (&GLOBAL, Handler::Call(success)),
        (&BINARY, Handler::Call(success)),
    ];
    let dispatcher = Dispatcher::new(&registry, &registrations);
    assert!(
        dispatcher
            .call(ctx(owner), HandleId(0), GLOBAL.name, json!({}))
            .await
            .is_ok()
    );
    let error = dispatcher
        .call(ctx(owner), id, GLOBAL.name, json!({}))
        .await
        .unwrap_err();
    assert!(matches!(&error, OpsError::Projected(_)));
    assert!(matches!(
        error.unprojected(),
        OpsError::Binding(BindingError::InvalidHandle {
            actual: Some(HandleKind::Abort),
            ..
        })
    ));
    for id in [HandleId(0), id, HandleId(u64::MAX)] {
        assert_eq!(
            dispatcher
                .call(ctx(owner), id, BINARY.name, json!({}))
                .await
                .unwrap_err()
                .code(),
            300
        );
    }
    assert!(
        dispatcher
            .stream(ctx(owner), HandleId(0), GLOBAL.name, json!({}))
            .is_err()
    );
}

static SECONDARY_CALLS: AtomicUsize = AtomicUsize::new(0);
fn with_abort(ctx: Ctx, _: Target, _: Value) -> CallFuture {
    SECONDARY_CALLS.fetch_add(1, Ordering::SeqCst);
    Box::pin(async move {
        assert!(ctx.abort.is_some());
        Ok(Value::Null)
    })
}
#[tokio::test]
async fn secondary_handles_use_the_same_owner_resolver_before_handler_entry() {
    static SECONDARY: OpDescriptor = OpDescriptor {
        runtime_handle_fields: &[("abort", &[HandleKind::Abort])],
        ..descriptor("test.secondary", OpMode::Call, &[])
    };
    let registry = HandleRegistry::default();
    let owner = Owner::new_node().unwrap();
    let foreign = Owner::new_python().unwrap();
    let own = registry
        .insert(owner, Entry::Abort(Arc::new(AbortSignal::new())))
        .unwrap();
    let other = registry
        .insert(foreign, Entry::Abort(Arc::new(AbortSignal::new())))
        .unwrap();
    let registrations = [(&SECONDARY, Handler::Call(with_abort))];
    let dispatcher = Dispatcher::new(&registry, &registrations);
    let foreign_error = dispatcher
        .call(
            ctx(owner),
            HandleId(0),
            SECONDARY.name,
            json!({"abort":other}),
        )
        .await
        .unwrap_err();
    let unknown_error = dispatcher
        .call(
            ctx(owner),
            HandleId(0),
            SECONDARY.name,
            json!({"abort":"18446744073709551615"}),
        )
        .await
        .unwrap_err();
    unknown(&foreign_error);
    assert_eq!(foreign_error.envelope(now()), unknown_error.envelope(now()));
    assert_eq!(SECONDARY_CALLS.load(Ordering::SeqCst), 0);
    assert!(
        dispatcher
            .call(
                ctx(owner),
                HandleId(0),
                SECONDARY.name,
                json!({"abort":own})
            )
            .await
            .is_ok()
    );
    assert_eq!(SECONDARY_CALLS.load(Ordering::SeqCst), 1);
}

fn api_failure(_: Ctx, _: Target, _: Value) -> CallFuture {
    Box::pin(async {
        let mut api = ApiCallError::new("retry later", "https://mock.invalid", json!({}));
        api.response_headers = Some(std::collections::HashMap::from([(
            "Retry-After".into(),
            "Tue, 14 Nov 2023 22:13:22 GMT".into(),
        )]));
        Err(AiMuxError::ApiCall(Box::new(api)).into())
    })
}
#[tokio::test]
async fn the_dispatch_projection_keeps_the_original_clock_and_cause() {
    static FAIL: OpDescriptor = descriptor("test.failure", OpMode::Call, &[]);
    let registry = HandleRegistry::default();
    let registrations = [(&FAIL, Handler::Call(api_failure))];
    let dispatcher = Dispatcher::new(&registry, &registrations);
    let error = dispatcher
        .call(ctx(Owner::CProcess), HandleId(0), FAIL.name, json!({}))
        .await
        .unwrap_err();
    assert!(matches!(
        error.unprojected(),
        OpsError::AiMux(AiMuxError::ApiCall(_))
    ));
    assert_eq!(error.envelope(now()).retry_after_ms, Some(2000));
    assert_eq!(
        error.envelope(now() + Duration::from_secs(100)),
        error.envelope(now())
    );
    assert_eq!(
        error
            .project(now() + Duration::from_secs(100))
            .envelope(now())
            .retry_after_ms,
        Some(2000)
    );
}

fn events(_: Ctx, _: Target, params: Value) -> Result<EventStream, OpsError> {
    match params["case"].as_str() {
        Some("reject") => {
            Err(ProtocolError::InvalidParams("reject before acceptance".into()).into())
        }
        Some("pending") => Ok(Box::pin(futures::stream::pending())),
        Some("large") => Ok(Box::pin(futures::stream::iter([Ok(json!(
            "x".repeat(aimux_ops::limits::ITEM_BYTES)
        ))]))),
        _ => Ok(Box::pin(futures::stream::iter([
            Ok(json!({"type":"error"})),
            Ok(json!({"type":"finish"})),
            Err(AiMuxError::Other("terminal".into()).into()),
            Ok(Value::Null),
        ]))),
    }
}
#[tokio::test]
async fn streams_preserve_business_errors_and_commit_exactly_one_terminal() {
    static STREAM: OpDescriptor = descriptor("test.stream", OpMode::Stream, &[]);
    let registry = HandleRegistry::default();
    let table = [(&STREAM, Handler::Stream(events))];
    let dispatcher = Dispatcher::new(&registry, &table);
    assert_eq!(
        dispatcher
            .stream(
                ctx(Owner::CProcess),
                HandleId(0),
                STREAM.name,
                json!({"case":"reject"})
            )
            .err()
            .unwrap()
            .code(),
        301
    );
    let frames = dispatcher
        .stream(ctx(Owner::CProcess), HandleId(0), STREAM.name, json!({}))
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert_eq!(frames.len(), 3);
    assert_eq!(
        serde_json::to_value(&frames[0]).unwrap()["event"]["type"],
        "error"
    );
    assert_eq!(
        serde_json::to_value(&frames[2]).unwrap()["error"]["code"],
        1
    );
    let frames = dispatcher
        .stream(
            ctx(Owner::CProcess),
            HandleId(0),
            STREAM.name,
            json!({"case":"large"}),
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert_eq!(frames.len(), 1);
    assert_eq!(
        serde_json::to_value(&frames[0]).unwrap()["error"]["code"],
        305
    );
    let signal = AbortSignal::new();
    let mut context = ctx(Owner::CProcess);
    context.abort = Some(signal.clone());
    let stream = dispatcher
        .stream(context, HandleId(0), STREAM.name, json!({"case":"pending"}))
        .unwrap();
    drop(stream);
    assert!(signal.is_aborted());
}

struct ReentrantDrop {
    registry: &'static HandleRegistry,
    owner: Owner,
}
// An Arc-backed built-in closure is only a canonical resource payload here, not
// an ops configuration callback; its captured guard probes registry-lock release.
impl Drop for ReentrantDrop {
    fn drop(&mut self) {
        let id = self
            .registry
            .insert(self.owner, Entry::Abort(Arc::new(AbortSignal::new())))
            .unwrap();
        drop(self.registry.remove(self.owner, id));
    }
}
#[test]
fn owner_allocations_are_unique_and_teardown_releases_locks_before_destruction() {
    let mut owners = std::collections::HashSet::new();
    for _ in 0..100 {
        assert!(owners.insert(Owner::new_node().unwrap()));
        assert!(owners.insert(Owner::new_python().unwrap()));
    }
    let registry = HandleRegistry::global();
    let owner = Owner::new_node().unwrap();
    let other = Owner::new_python().unwrap();
    let guard = ReentrantDrop { registry, owner };
    let resolver = aimux_provider_utils::Resolvable::from_fn(move || {
        let _ = &guard;
        Ok("test".to_owned())
    });
    let id = registry
        .insert(owner, Entry::CredentialResolver(Arc::new(resolver)))
        .unwrap();
    let retained = registry
        .insert(other, Entry::Abort(Arc::new(AbortSignal::new())))
        .unwrap();
    assert_eq!(registry.release_owner(owner), 1);
    assert!(
        registry
            .get(owner, id, &[HandleKind::CredentialResolver])
            .is_err()
    );
    assert!(registry.get(other, retained, &[HandleKind::Abort]).is_ok());
    registry.release_owner(other);
}
