//! The six realtime operations share the native Operation service.
use crate::dispatch::{CallFuture, Ctx, Handler, PollResult, Target, decode, next_operation};
use crate::envelope::{BindingError, OpsError, ProtocolError};
use crate::handle::{Entry, HandleKind, HandleRegistry};
use crate::op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope, TypeRef};
use crate::operation::{Operation, OperationOptions};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmptyParams {}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationNextParams {
    #[serde(default)]
    pub timeout_ms: u64,
}

#[derive(Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TranscriptionStartParams {
    pub input_audio_format: Option<aimux_core::transcription_model::InputAudioFormat>,
    pub provider_options: Option<aimux_core::shared::SharedProviderOptions>,
    pub headers: Option<aimux_core::shared::SharedHeaders>,
    #[serde(default)]
    pub include_raw_chunks: bool,
    pub timeout: Option<aimux_core::options::TimeoutConfiguration>,
    #[serde(rename = "abort")]
    _abort: Option<crate::HandleId>,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
enum OperationPollResult {
    Part {
        #[schemars(with = "aimux_core::transcription_model::TranscriptionStreamPart")]
        part: Value,
    },
    Ended,
    Timeout,
}

const fn descriptor(
    name: &'static str,
    mode: OpMode,
    kinds: &'static [HandleKind],
    request_type: TypeRef,
    result_type: TypeRef,
    cancellation: Cancellation,
    side_effects: bool,
) -> OpDescriptor {
    OpDescriptor {
        name,
        since_ops_version: "1.0",
        mode,
        allowed_handle_kinds: kinds,
        request_type,
        result_type,
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: if matches!(cancellation, Cancellation::Abort) {
            &[("abort", &[HandleKind::Abort])]
        } else {
            &[]
        },
        required_capabilities: if matches!(cancellation, Cancellation::Abort) {
            &["doStream"]
        } else {
            &[]
        },
        cancellation,
        side_effects,
        result_ownership: "owned",
        request_byte_policy: if matches!(mode, OpMode::BinaryPush) {
            RequestBytePolicy::Binary
        } else {
            RequestBytePolicy::Json
        },
        retry_owner: "none",
        source_symbols: &["aimux_ops::operation::Operation"],
    }
}
pub(crate) static OPS: &[OpDescriptor] = &[
    descriptor(
        "transcription.start",
        OpMode::Call,
        &[HandleKind::Transcription],
        TypeRef::of::<TranscriptionStartParams>(),
        TypeRef::of::<crate::handle::HandleId>(),
        Cancellation::Abort,
        true,
    ),
    descriptor(
        "operation.push",
        OpMode::BinaryPush,
        &[HandleKind::Operation],
        TypeRef::of::<Vec<u8>>(),
        TypeRef::of::<()>(),
        Cancellation::Operation,
        true,
    ),
    descriptor(
        "operation.inputDone",
        OpMode::Call,
        &[HandleKind::Operation],
        TypeRef::of::<EmptyParams>(),
        TypeRef::of::<()>(),
        Cancellation::Operation,
        true,
    ),
    descriptor(
        "operation.next",
        OpMode::Call,
        &[HandleKind::Operation],
        TypeRef::of::<OperationNextParams>(),
        TypeRef::of::<OperationPollResult>(),
        Cancellation::Operation,
        true,
    ),
    descriptor(
        "operation.status",
        OpMode::Call,
        &[HandleKind::Operation],
        TypeRef::of::<EmptyParams>(),
        TypeRef::of::<crate::operation::OperationStatus>(),
        Cancellation::None,
        false,
    ),
    descriptor(
        "operation.cancel",
        OpMode::Call,
        &[HandleKind::Operation],
        TypeRef::of::<EmptyParams>(),
        TypeRef::of::<crate::operation::CancelResult>(),
        Cancellation::Operation,
        true,
    ),
];
pub(crate) fn register(r: &mut Vec<(&'static OpDescriptor, Handler)>) {
    for (descriptor, handler) in
        OPS.iter()
            .zip([start, binary_only, input_done, next, status, cancel]
                as [fn(Ctx, Target, Value) -> CallFuture; 6])
    {
        r.push((descriptor, Handler::Call(handler)));
    }
}
fn operation(target: Target) -> Result<std::sync::Arc<Operation>, OpsError> {
    match target {
        Target::Handle(Entry::Operation(operation)) => Ok(operation),
        _ => Err(OpsError::Protocol(ProtocolError::InvalidState(
            "expected operation target".into(),
        ))),
    }
}
fn serialize(value: impl serde::Serialize) -> Result<Value, OpsError> {
    serde_json::to_value(value)
        .map_err(|error| BindingError::ResultSerialization(error.to_string()).into())
}
fn start(ctx: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        if params
            .get("inputAudioFormat")
            .and_then(Value::as_object)
            .is_some_and(|format| {
                format
                    .keys()
                    .any(|key| key != "formatType" && key != "rate")
            })
        {
            return Err(
                ProtocolError::InvalidParams("unknown inputAudioFormat field".into()).into(),
            );
        }
        if params
            .get("timeout")
            .and_then(Value::as_object)
            .is_some_and(|timeout| {
                timeout.keys().any(|key| {
                    !["totalMs", "stepMs", "firstChunkMs", "chunkMs"].contains(&key.as_str())
                })
            })
        {
            return Err(ProtocolError::InvalidParams("unknown timeout field".into()).into());
        }
        let request: TranscriptionStartParams = decode(params)?;
        let Target::Handle(Entry::Transcription(model)) = target else {
            return Err(ProtocolError::InvalidState("expected transcription target".into()).into());
        };
        let options = OperationOptions {
            input_audio_format: request.input_audio_format,
            provider_options: request.provider_options,
            headers: request.headers,
            include_raw_chunks: request.include_raw_chunks,
            timeout: request.timeout,
            ..Default::default()
        };
        let operation = Operation::start_with_clock(model, options, ctx.abort, ctx.now).await?;
        let id = HandleRegistry::global().insert(ctx.owner, Entry::Operation(operation))?;
        serialize(id)
    })
}
fn binary_only(_: Ctx, _: Target, _: Value) -> CallFuture {
    Box::pin(async {
        Err(ProtocolError::InvalidOp("operation.push requires binary dispatch".into()).into())
    })
}
fn input_done(_: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        decode::<EmptyParams>(params)?;
        operation(target)?.input_done();
        Ok(Value::Null)
    })
}
fn next(_: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        let request: OperationNextParams = decode(params)?;
        let operation = operation(target)?;
        match next_operation(&operation, Duration::from_millis(request.timeout_ms)).await? {
            PollResult::Part(part) => serialize(OperationPollResult::Part { part }),
            PollResult::Ended => serialize(OperationPollResult::Ended),
            PollResult::Timeout => serialize(OperationPollResult::Timeout),
        }
    })
}
fn status(_: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        decode::<EmptyParams>(params)?;
        serialize(operation(target)?.status())
    })
}
fn cancel(_: Ctx, target: Target, params: Value) -> CallFuture {
    Box::pin(async move {
        decode::<EmptyParams>(params)?;
        serialize(operation(target)?.cancel())
    })
}
