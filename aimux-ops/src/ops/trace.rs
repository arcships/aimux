//! Trace storage and queries. Session identifiers and filters are ordinary data.

use std::sync::Arc;

use aimux_core::error::{AiMuxError, SourceError};
use aimux_core::trace::store::{DEFAULT_RING_CAPACITY, DEFAULT_SCOPE_CAP, TraceAuditMode};
use aimux_core::trace::{RingTraceStore, TraceFilter};
use serde::Deserialize;

use crate::dispatch::serialize_result as result;
use crate::dispatch::{CallFuture, Ctx, Handler, Params, Target, decode};
use crate::envelope::{BindingError, OpsError, ProtocolError};
use crate::handle::{Entry, HandleKind, HandleRegistry};
use crate::op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope, TypeRef};
use crate::ops::operation::EmptyParams;

pub type TraceAggregateResult = Vec<aimux_core::trace::TraceStats>;
pub type TraceTrajectoryResult = Vec<aimux_core::trace::SessionStepStat>;

#[derive(Clone, Copy, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(rename = "TraceAuditMode")]
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
    #[schemars(range(min = 1, max = 1_048_576))]
    capacity: usize,
    #[serde(default = "default_scope_capacity")]
    #[schemars(range(min = 1))]
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
    #[schemars(with = "TraceAggregateFilter")]
    filter: TraceFilter,
}

// The schema uses the canonical filter type, with a closed object.
#[derive(schemars::JsonSchema)]
#[schemars(schema_with = "trace_filter_schema")]
struct TraceAggregateFilter;

fn trace_filter_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let mut schema = <TraceFilter as schemars::JsonSchema>::json_schema(generator);
    schema.insert("additionalProperties".into(), false.into());
    schema
}

fn deserialize_filter<'de, D: serde::Deserializer<'de>>(d: D) -> Result<TraceFilter, D::Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Filter {
        provider: Option<String>,
        model: Option<String>,
        session_id: Option<String>,
        since_unix_ms: Option<i64>,
    }
    let filter: Filter = crate::request::deserialize_object(d)?;
    Ok(TraceFilter {
        provider: filter.provider,
        model: filter.model,
        session_id: filter.session_id,
        since_unix_ms: filter.since_unix_ms,
    })
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TraceSessionParams {
    session_id: String,
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

const MAX_TRACE_CAPACITY: usize = 1 << 20;

fn create(ctx: Ctx, _target: Target, value: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        let p: TraceCreateParams = decode(value)?;
        if p.capacity == 0 || p.capacity > MAX_TRACE_CAPACITY {
            return Err(OpsError::Protocol(ProtocolError::invalid_params(
                "trace capacity must be between 1 and 1048576",
            )));
        }
        if p.per_scope_capacity == 0 {
            return Err(OpsError::Protocol(ProtocolError::invalid_params(
                "trace perScopeCapacity must be greater than zero",
            )));
        }
        let reservation = HandleRegistry::global().reserve()?;
        let mode = match p.audit_mode {
            AuditMode::Off => TraceAuditMode::Off,
            AuditMode::Shared => TraceAuditMode::Shared,
            AuditMode::Strict => TraceAuditMode::Strict,
        };
        let trace = RingTraceStore::try_with_capacity(p.capacity, p.per_scope_capacity)
            .map_err(|msg| OpsError::Protocol(ProtocolError::invalid_params(msg)))?
            .with_audit_mode(mode);
        let id = HandleRegistry::global().commit(
            ctx.owner,
            reservation,
            Entry::TraceStore(Arc::new(trace)),
        );
        result(id)
    })
}

fn aggregate(_ctx: Ctx, target: Target, value: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        let p: TraceAggregateParams = decode(value)?;
        let stats: TraceAggregateResult = store(target)?.aggregate(&p.filter);
        result(stats)
    })
}

fn session_chain(_ctx: Ctx, target: Target, value: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        let p: TraceSessionParams = decode(value)?;
        let view = store(target)?
            .session_chain(&p.session_id)
            .ok_or_else(|| OpsError::AiMux(AiMuxError::invalid_argument("unknown session")))?;
        result(view)
    })
}

fn session_trajectory(_ctx: Ctx, target: Target, value: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        let p: TraceSessionParams = decode(value)?;
        let steps: TraceTrajectoryResult = store(target)?.session_cache_trajectory(&p.session_id);
        result(steps)
    })
}

fn export(_ctx: Ctx, target: Target, value: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        let _: EmptyParams = decode(value)?;
        let mut bytes = Vec::new();
        store(target)?.export_jsonl(&mut bytes).map_err(|error| {
            OpsError::Binding(BindingError::ResultSerialization {
                message: format!("export: {error}"),
                cause: Some(SourceError::new(&error)),
            })
        })?;
        let jsonl = String::from_utf8(bytes).map_err(|error| {
            OpsError::Binding(BindingError::ResultSerialization {
                message: format!("utf8: {error}"),
                cause: Some(SourceError::new(&error)),
            })
        })?;
        result(jsonl)
    })
}

fn clear(_ctx: Ctx, target: Target, value: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        let _: EmptyParams = decode(value)?;
        store(target)?.clear();
        result(())
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
