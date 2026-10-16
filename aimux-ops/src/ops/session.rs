//! Session grouping and queries backed by the core session index.

use std::sync::Arc;

use aimux_core::session::{SessionStore, init_session_infer, init_session_store};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dispatch::{CallFuture, Ctx, Handler, Target, decode};
use crate::envelope::{BindingError, OpsError};
use crate::handle::{Entry, HandleId, HandleKind, HandleRegistry};
use crate::op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope};
use crate::ops::operation::EmptyParams;

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionStoreCreateParams {
    infer: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionStoreCallsParams {
    session_id: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct SessionStoreHandle {
    handle: HandleId,
}

pub type SessionViews = Vec<aimux_core::session::SessionView>;
pub type SessionCalls = Vec<aimux_core::session::SessionCall>;

pub(crate) static OPS: &[OpDescriptor] = &[
    OpDescriptor {
        name: "sessionStore.create",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: "aimux_ops::ops::session::SessionStoreCreateParams",
        result_type: "aimux_ops::ops::session::SessionStoreHandle",
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects: true,
        retry_owner: "none",
        source_symbols: &["aimux_session_store_init", "aimux_session_infer_init"],
    },
    OpDescriptor {
        name: "sessionStore.attach",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[HandleKind::SessionStore],
        request_type: "EmptyParams",
        result_type: "()",
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects: true,
        retry_owner: "none",
        source_symbols: &["aimux_core::session::init_session_store"],
    },
    OpDescriptor {
        name: "sessionStore.list",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[HandleKind::SessionStore],
        request_type: "EmptyParams",
        result_type: "aimux_ops::ops::session::SessionViews",
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects: false,
        retry_owner: "none",
        source_symbols: &["aimux_list_sessions"],
    },
    OpDescriptor {
        name: "sessionStore.calls",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[HandleKind::SessionStore],
        request_type: "aimux_ops::ops::session::SessionStoreCallsParams",
        result_type: "aimux_ops::ops::session::SessionCalls",
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects: false,
        retry_owner: "none",
        source_symbols: &["aimux_session_calls"],
    },
];

pub(crate) fn register(r: &mut Vec<(&'static OpDescriptor, Handler)>) {
    r.push((&OPS[0], Handler::Call(create)));
    r.push((&OPS[1], Handler::Call(attach)));
    r.push((&OPS[2], Handler::Call(list)));
    r.push((&OPS[3], Handler::Call(calls)));
}

fn json(value: impl Serialize) -> Result<Value, OpsError> {
    serde_json::to_value(value)
        .map_err(|error| OpsError::Binding(BindingError::ResultSerialization(error.to_string())))
}

fn store(target: Target) -> Result<Arc<SessionStore>, OpsError> {
    match target {
        Target::Handle(Entry::SessionStore(store)) => Ok(store),
        other => Err(OpsError::Binding(BindingError::InvalidHandle {
            expected: vec![HandleKind::SessionStore],
            actual: match other {
                Target::Global => None,
                Target::Handle(entry) => Some(entry.kind()),
            },
        })),
    }
}

fn create(ctx: Ctx, _target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let request: SessionStoreCreateParams = decode(value)?;
        let handle = HandleRegistry::global().insert(
            ctx.owner,
            Entry::SessionStore(Arc::new(SessionStore::new())),
        )?;
        // Core inference is process-wide, as in aimux_session_infer_init.
        init_session_infer(request.infer);
        json(SessionStoreHandle { handle })
    })
}

fn attach(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let _: EmptyParams = decode(value)?;
        init_session_store(store(target)?);
        Ok(Value::Null)
    })
}

fn list(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let _: EmptyParams = decode(value)?;
        let sessions: SessionViews = store(target)?.list_sessions();
        json(sessions)
    })
}

fn calls(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let request: SessionStoreCallsParams = decode(value)?;
        let calls: SessionCalls = store(target)?.session_calls(&request.session_id);
        json(calls)
    })
}
