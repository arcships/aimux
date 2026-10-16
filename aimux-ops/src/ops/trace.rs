//! Trace storage and queries. Session identifiers and filters are ordinary data.

use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::trace::store::{DEFAULT_RING_CAPACITY, DEFAULT_SCOPE_CAP, TraceAuditMode};
use aimux_core::trace::{RingTraceStore, TraceFilter};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dispatch::{CallFuture, Ctx, Handler, Target, decode};
use crate::envelope::{BindingError, OpsError, ProtocolError};
use crate::handle::{Entry, HandleKind, HandleRegistry};
use crate::op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope, TypeRef};
use crate::ops::operation::EmptyParams;

pub type TraceAggregateResult = Vec<aimux_core::trace::TraceStats>;
pub type TraceTrajectoryResult = Vec<aimux_core::trace::SessionStepStat>;

#[derive(Clone, Copy, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AuditMode {
    #[default]
    Off,
    Shared,
    Strict,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TraceCreateParams {
    #[serde(default = "default_capacity")]
    capacity: usize,
    #[serde(default = "default_scope_capacity")]
    per_scope_capacity: usize,
    #[serde(default)]
    audit_mode: AuditMode,
}

fn default_capacity() -> usize {
    DEFAULT_RING_CAPACITY
}

fn default_scope_capacity() -> usize {
    DEFAULT_SCOPE_CAP
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceAggregateParams {
    #[serde(default, deserialize_with = "deserialize_filter")]
    #[schemars(schema_with = "trace_filter_schema")]
    filter: TraceFilter,
}

// TraceFilter is the canonical core DTO, whose open deserializer is also used
// elsewhere. Validate its control keys here without duplicating its fields.
fn trace_filter_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let mut schema = <TraceFilter as schemars::JsonSchema>::json_schema(generator);
    schema.insert("additionalProperties".into(), false.into());
    schema
}

fn deserialize_filter<'de, D: serde::Deserializer<'de>>(d: D) -> Result<TraceFilter, D::Error> {
    let value = Value::deserialize(d)?;
    let object = value
        .as_object()
        .ok_or_else(|| serde::de::Error::custom("trace filter must be an object"))?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "provider" | "model" | "sessionId" | "sinceUnixMs"
        ) {
            return Err(serde::de::Error::custom(format!(
                "unknown filter field `{key}`"
            )));
        }
    }
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TraceSessionParams {
    session_id: String,
}

fn result(value: impl Serialize) -> Result<Value, OpsError> {
    serde_json::to_value(value)
        .map_err(|error| OpsError::Binding(BindingError::ResultSerialization(error.to_string())))
}

fn store(target: Target) -> Result<Arc<RingTraceStore>, OpsError> {
    match target {
        Target::Handle(Entry::TraceStore(store)) => Ok(store),
        other => Err(OpsError::Binding(BindingError::InvalidHandle {
            expected: vec![HandleKind::TraceStore],
            actual: match other {
                Target::Global => None,
                Target::Handle(entry) => Some(entry.kind()),
            },
        })),
    }
}

fn create(ctx: Ctx, _target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let p: TraceCreateParams = decode(value)?;
        if p.capacity == 0 || p.per_scope_capacity == 0 {
            return Err(OpsError::Protocol(ProtocolError::InvalidParams(
                "trace capacities must be greater than zero".into(),
            )));
        }
        let mode = match p.audit_mode {
            AuditMode::Off => TraceAuditMode::Off,
            AuditMode::Shared => TraceAuditMode::Shared,
            AuditMode::Strict => TraceAuditMode::Strict,
        };
        let trace = Arc::new(
            RingTraceStore::with_capacity(p.capacity, p.per_scope_capacity).with_audit_mode(mode),
        );
        let id = HandleRegistry::global().insert(ctx.owner, Entry::TraceStore(trace))?;
        result(id)
    })
}

fn aggregate(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let p: TraceAggregateParams = decode(value)?;
        let stats: TraceAggregateResult = store(target)?.aggregate(&p.filter);
        result(stats)
    })
}

fn session_chain(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let p: TraceSessionParams = decode(value)?;
        let view = store(target)?.session_chain(&p.session_id).ok_or_else(|| {
            OpsError::AiMux(AiMuxError::InvalidArgument("unknown session".into()))
        })?;
        result(view)
    })
}

fn session_trajectory(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let p: TraceSessionParams = decode(value)?;
        let steps: TraceTrajectoryResult = store(target)?.session_cache_trajectory(&p.session_id);
        result(steps)
    })
}

fn export(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let _: EmptyParams = decode(value)?;
        let mut bytes = Vec::new();
        store(target)?.export_jsonl(&mut bytes).map_err(|error| {
            OpsError::Binding(BindingError::ResultSerialization(format!(
                "export: {error}"
            )))
        })?;
        let jsonl = String::from_utf8(bytes).map_err(|error| {
            OpsError::Binding(BindingError::ResultSerialization(format!("utf8: {error}")))
        })?;
        Ok(Value::String(jsonl))
    })
}

fn clear(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let _: EmptyParams = decode(value)?;
        store(target)?.clear();
        Ok(Value::Null)
    })
}

const fn descriptor(
    name: &'static str,
    allowed_handle_kinds: &'static [HandleKind],
    request_type: TypeRef,
    result_type: TypeRef,
    side_effects: bool,
    source_symbols: &'static [&'static str],
) -> OpDescriptor {
    OpDescriptor {
        name,
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds,
        request_type,
        result_type,
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects,
        retry_owner: "none",
        source_symbols,
    }
}

pub(crate) static OPS: &[OpDescriptor] = &[
    descriptor(
        "trace.create",
        &[],
        TypeRef::of::<TraceCreateParams>(),
        TypeRef::of::<crate::handle::HandleId>(),
        true,
        &["aimux_core::trace::RingTraceStore::with_capacity"],
    ),
    descriptor(
        "trace.aggregate",
        &[HandleKind::TraceStore],
        TypeRef::of::<TraceAggregateParams>(),
        TypeRef::of::<TraceAggregateResult>(),
        false,
        &[
            "aimux_trace_aggregate",
            "aimux_core::trace::RingTraceStore::aggregate",
        ],
    ),
    descriptor(
        "trace.sessionChain",
        &[HandleKind::TraceStore],
        TypeRef::of::<TraceSessionParams>(),
        TypeRef::of::<aimux_core::trace::SessionChainView>(),
        false,
        &[
            "aimux_trace_session_chain",
            "aimux_core::trace::RingTraceStore::session_chain",
        ],
    ),
    descriptor(
        "trace.sessionTrajectory",
        &[HandleKind::TraceStore],
        TypeRef::of::<TraceSessionParams>(),
        TypeRef::of::<TraceTrajectoryResult>(),
        false,
        &[
            "aimux_trace_session_trajectory",
            "aimux_core::trace::RingTraceStore::session_cache_trajectory",
        ],
    ),
    descriptor(
        "trace.export",
        &[HandleKind::TraceStore],
        TypeRef::of::<crate::ops::operation::EmptyParams>(),
        TypeRef::of::<String>(),
        false,
        &[
            "aimux_trace_export_jsonl",
            "aimux_core::trace::RingTraceStore::export_jsonl",
        ],
    ),
    descriptor(
        "trace.clear",
        &[HandleKind::TraceStore],
        TypeRef::of::<crate::ops::operation::EmptyParams>(),
        TypeRef::of::<()>(),
        true,
        &[
            "aimux_trace_clear",
            "aimux_core::trace::RingTraceStore::clear",
        ],
    ),
];

pub(crate) fn register(r: &mut Vec<(&'static OpDescriptor, Handler)>) {
    let handlers = [
        create,
        aggregate,
        session_chain,
        session_trajectory,
        export,
        clear,
    ];
    r.extend(
        OPS.iter()
            .zip(handlers)
            .map(|(op, handler)| (op, Handler::Call(handler))),
    );
}
