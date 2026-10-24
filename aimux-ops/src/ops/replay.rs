//! Model-level replay using existing core recordings and caller-owned live targets.
//! Transport replay is Future (G-c); recording schema migration remains G-b.

use std::sync::Arc;

use aimux_core::recording::{Recording, RecordingOperation};
use aimux_core::replay::{MockReplayModel, replay_decision_with_model, replay_with_model};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dispatch::{CallFuture, Ctx, Handler, Target, decode, resolve_secondary};
use crate::envelope::{BindingError, OpsError, ProtocolError};
use crate::handle::{Entry, HandleId, HandleKind, HandleRegistry};
use crate::op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope, TypeRef};
use crate::ops::operation::EmptyParams;

pub(crate) static OPS: &[OpDescriptor] = &[
    OpDescriptor {
        name: "replay.create",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: TypeRef::of::<ReplayCreateParams>(),
        result_type: TypeRef::of::<ReplaySessionHandle>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        side_effects: true,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        retry_owner: "none",
        source_symbols: &["aimux_mock_replay_new"],
    },
    OpDescriptor {
        name: "replay.capabilities",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[HandleKind::ReplaySession],
        request_type: TypeRef::of::<crate::ops::operation::EmptyParams>(),
        result_type: TypeRef::of::<()>(),
        event_type: None,
        scope: Scope::Future,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        side_effects: false,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        retry_owner: "none",
        source_symbols: &["aimux_provider_utils::fetch::Fetch"],
    },
    OpDescriptor {
        name: "replay.inspect",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[HandleKind::ReplaySession],
        request_type: TypeRef::of::<crate::ops::operation::EmptyParams>(),
        result_type: TypeRef::of::<ReplayInspection>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        side_effects: false,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        retry_owner: "none",
        source_symbols: &["aimux_core::replay::MockReplayModel::recordings"],
    },
    OpDescriptor {
        name: "replay.dryRun",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[HandleKind::ReplaySession],
        request_type: TypeRef::of::<crate::ops::operation::EmptyParams>(),
        result_type: TypeRef::of::<ReplayPlan>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        side_effects: false,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        retry_owner: "none",
        source_symbols: &["aimux_core::recording::Recording"],
    },
    OpDescriptor {
        name: "replay.live",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[HandleKind::ReplaySession],
        request_type: TypeRef::of::<ReplayLiveParams>(),
        result_type: TypeRef::of::<ReplayLiveResult>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[
            (
                "target.handle",
                &[
                    HandleKind::Language,
                    HandleKind::Decision,
                    HandleKind::Registry,
                ],
            ),
            ("abort", &[HandleKind::Abort]),
        ],
        required_capabilities: &[],
        cancellation: Cancellation::Abort,
        side_effects: true,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        retry_owner: "core",
        source_symbols: &["aimux_core::replay::replay_with_model"],
    },
];
pub(crate) fn register(r: &mut Vec<(&'static OpDescriptor, Handler)>) {
    for (op, handler) in OPS
        .iter()
        .zip([create, capabilities, inspect, dry_run, live]
            as [fn(Ctx, Target, Value) -> CallFuture; 5])
    {
        r.push((op, Handler::Call(handler)));
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayCreateParams {
    recordings_jsonl: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ReplaySessionHandle {
    handle: HandleId,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayLiveParams {
    call_id: String,
    target: LiveTarget,
    // Dispatch resolves this into Ctx before invoking the handler.
    #[serde(rename = "abort")]
    _abort: Option<HandleId>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum LiveTarget {
    Model { handle: HandleId },
    Registry { handle: HandleId },
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ReplayInspection {
    recordings: Vec<ReplayRecordingInfo>,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ReplayRecordingInfo {
    call_id: String,
    provider: aimux_core::recording::ProviderRecord,
    operation: RecordingOperation,
    complete: bool,
    transport_closed: bool,
    exchange_count: usize,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ReplayPlan {
    calls: Vec<ReplayPlannedCall>,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ReplayPlannedCall {
    call_id: String,
    provider: aimux_core::recording::ProviderRecord,
    operation: RecordingOperation,
    live_supported: bool,
    requires_target: bool,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(untagged)]
#[schemars(extend("x-aimux-codec" = "ordered-untagged"))]
pub enum ReplayLiveResult {
    Language(aimux_core::generate::GenerateTextResult),
    Decision(aimux_core::decision_model::DecideResult),
}

fn invalid(message: impl Into<String>) -> OpsError {
    ProtocolError::invalid_params(message).into()
}

fn serialize(value: impl Serialize) -> Result<Value, OpsError> {
    serde_json::to_value(value).map_err(|error| BindingError::result_serialization(&error).into())
}

fn session(target: Target) -> Result<Arc<MockReplayModel>, OpsError> {
    match target {
        Target::Handle(Entry::ReplaySession(session)) => Ok(session),
        _ => Err(ProtocolError::InvalidState("expected replay session target".into()).into()),
    }
}

fn from_jsonl(jsonl: &str) -> Result<MockReplayModel, OpsError> {
    let mut recordings: Vec<Recording> = Vec::new();
    for (index, line) in jsonl.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        // Use the same canonical reader as aimux_mock_replay_new. G-b leaves
        // the existing schema/alias handling in core until RFC-0041.
        let recording: Recording = serde_json::from_str(line)
            .map_err(|error| invalid(format!("recordingsJsonl line {}: {error}", index + 1)))?;
        if recording.call_id.is_empty() || recordings.iter().any(|r| r.call_id == recording.call_id)
        {
            return Err(invalid("recording callId must be nonempty and unique"));
        }
        recordings.push(recording);
    }
    let first = recordings
        .first()
        .ok_or_else(|| invalid("recordingsJsonl: no recordings"))?;
    Ok(MockReplayModel::new(
        first.provider.provider.clone(),
        first.provider.model_id.clone(),
        recordings,
    ))
}

fn create(ctx: Ctx, _target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let params: ReplayCreateParams = decode(params)?;
        let session = from_jsonl(&params.recordings_jsonl)?;
        let handle =
            HandleRegistry::global().insert(ctx.owner, Entry::ReplaySession(Arc::new(session)))?;
        serialize(ReplaySessionHandle { handle })
    })
}

fn capabilities(_ctx: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let _: EmptyParams = decode(params)?;
        let _session = session(target)?;
        Err(aimux_core::AiMuxError::UnsupportedFunctionality(
            "replay transport capabilities (Fetch/WebSocket/Download) are Future (G-c)".into(),
        )
        .into())
    })
}

fn inspect(_ctx: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let _: EmptyParams = decode(params)?;
        let session = session(target)?;
        serialize(ReplayInspection {
            recordings: session
                .recordings()
                .iter()
                .map(|r| ReplayRecordingInfo {
                    call_id: r.call_id.clone(),
                    provider: r.provider.clone(),
                    operation: r.input.operation,
                    complete: r.complete,
                    transport_closed: r.transport_closed,
                    exchange_count: r.exchanges.len(),
                })
                .collect(),
        })
    })
}

fn dry_run(_ctx: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let _: EmptyParams = decode(params)?;
        let session = session(target)?;
        serialize(ReplayPlan {
            calls: session
                .recordings()
                .iter()
                .map(|r| ReplayPlannedCall {
                    call_id: r.call_id.clone(),
                    provider: r.provider.clone(),
                    operation: r.input.operation,
                    live_supported: true,
                    requires_target: true,
                })
                .collect(),
        })
    })
}

fn live(ctx: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let params: ReplayLiveParams = decode(params)?;
        let session = session(target)?;
        let recording = session
            .recordings()
            .iter()
            .find(|r| r.call_id == params.call_id)
            .ok_or_else(|| invalid("callId is not in this replay session"))?;
        let model_kind = match recording.input.operation {
            RecordingOperation::LanguageModel => HandleKind::Language,
            RecordingOperation::Decision => HandleKind::Decision,
        };
        let (handle, kind) = match params.target {
            LiveTarget::Model { handle } => (handle, model_kind),
            LiveTarget::Registry { handle } => (handle, HandleKind::Registry),
        };
        let entry =
            resolve_secondary(HandleRegistry::global(), &ctx, &serialize(handle)?, &[kind])?;
        let result = match entry {
            Entry::Language(model) => ReplayLiveResult::Language(
                replay_with_model(recording, model.as_ref(), None).await?,
            ),
            Entry::Decision(model) => ReplayLiveResult::Decision(
                replay_decision_with_model(recording, model.as_ref()).await?,
            ),
            Entry::Registry(registry) => match recording.input.operation {
                RecordingOperation::LanguageModel => {
                    let model =
                        aimux_providers::replay::rebuild_provider(&recording.provider, &registry)?;
                    ReplayLiveResult::Language(
                        replay_with_model(recording, model.as_ref(), None).await?,
                    )
                }
                RecordingOperation::Decision => {
                    let model = aimux_providers::replay::rebuild_decision_provider(
                        &recording.provider,
                        &registry,
                    )?;
                    ReplayLiveResult::Decision(
                        replay_decision_with_model(recording, model.as_ref()).await?,
                    )
                }
            },
            _ => {
                return Err(
                    ProtocolError::InvalidState("unexpected replay target kind".into()).into(),
                );
            }
        };
        serialize(result)
    })
}
