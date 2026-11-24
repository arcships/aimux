//! Decision calls reuse the existing core operation and its model properties.

use aimux_core::decision_model::{DecideOptions, decide, decision_model_properties};
use serde::Deserialize;

use crate::{
    dispatch::{CallFuture, Ctx, Handler, Params, Target, decode, serialize_result},
    envelope::{OpsError, ProtocolError},
    handle::{Entry, HandleId, HandleKind},
    op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope, TypeRef},
    ops::operation::EmptyParams,
};

/// Runtime controls surround the canonical business options; only the control
/// object rejects unknown fields. Dispatch resolves `abort` with the caller's
/// owner before entering the handler and supplies its signal in `Ctx`.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionDecideRequest {
    pub options: DecideOptions,
    pub abort: Option<HandleId>,
}

pub(crate) static OPS: &[OpDescriptor] = &[
    OpDescriptor {
        name: "decision.decide",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[HandleKind::Decision],
        request_type: TypeRef::of::<DecisionDecideRequest>(),
        result_type: TypeRef::of::<aimux_core::decision_model::DecideResult>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[(&["abort"], &[HandleKind::Abort])],
        required_capabilities: &["doDecide"],
        cancellation: Cancellation::Abort,
        side_effects: true,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        retry_owner: "core",
        source_symbols: &[
            "aimux_decide",
            "aimux_decide_with_abort",
            "aimux_core::decision_model::decide",
        ],
    },
    OpDescriptor {
        name: "decision.capabilities",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[HandleKind::Decision],
        request_type: TypeRef::of::<crate::ops::operation::EmptyParams>(),
        // The canonical property helper already returns this JSON value.
        result_type: TypeRef::of::<serde_json::Value>(),
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
        source_symbols: &[
            "aimux_decision_capabilities",
            "aimux_core::decision_model::decision_model_properties",
        ],
    },
];

pub(crate) fn register(r: &mut Vec<(&'static OpDescriptor, Handler)>) {
    for (descriptor, handler) in OPS
        .iter()
        .zip([decision_decide, decision_capabilities] as [crate::dispatch::CallHandler; 2])
    {
        r.push((descriptor, Handler::Call(handler)));
    }
}

fn model(
    target: Target,
) -> Result<std::sync::Arc<dyn aimux_core::decision_model::DecisionModel>, OpsError> {
    let Target::Handle(Entry::Decision(model)) = target else {
        return Err(ProtocolError::InvalidState("expected decision target".into()).into());
    };
    Ok(model)
}

fn decision_capabilities(_: Ctx, target: Target, params: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        decode::<EmptyParams>(params)?;
        serialize_result(decision_model_properties(model(target)?.as_ref()))
    })
}

fn decision_decide(ctx: Ctx, target: Target, params: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        let mut request: DecisionDecideRequest = decode(params)?;
        request.options.abort_signal = ctx.abort;
        let result = decide(model(target)?.as_ref(), request.options).await?;
        serialize_result(result)
    })
}
