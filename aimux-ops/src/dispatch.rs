//! Shared dispatch, validation and error projection for native adapters.
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Duration;

use futures::{Stream, StreamExt};
use serde_json::Value;

use crate::envelope::{OpsError, ProtocolError};
pub use crate::frame::FrameStream;
use crate::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use crate::op::{Cancellation, OpDescriptor, OpMode};

#[derive(Clone)]
pub struct Ctx {
    pub owner: Owner,
    pub abort: Option<aimux_core::AbortSignal>,
}

#[derive(Clone)]
pub enum Target {
    Global,
    Handle(Entry),
}

pub type CallFuture = Pin<Box<dyn Future<Output = Result<Value, OpsError>> + Send>>;
pub type EventStream = Pin<Box<dyn Stream<Item = Result<Value, OpsError>> + Send>>;

#[derive(Clone, Copy)]
pub enum Handler {
    Call(fn(Ctx, Target, Value) -> CallFuture),
    Stream(fn(Ctx, Target, Value) -> Result<EventStream, OpsError>),
}

/// Owner-aware resolution seam. Production uses HandleRegistry; gates use an isolated registry.
pub trait ResourceRegistry: Sync {
    /// # Errors
    /// Returns InvalidHandle for missing, foreign or incorrectly typed handles.
    fn get(&self, owner: Owner, id: HandleId, allowed: &[HandleKind]) -> Result<Entry, OpsError>;
    fn remove(&self, owner: Owner, id: HandleId) -> Option<Entry>;
}
impl ResourceRegistry for HandleRegistry {
    fn get(&self, owner: Owner, id: HandleId, allowed: &[HandleKind]) -> Result<Entry, OpsError> {
        self.get(owner, id, allowed)
    }
    fn remove(&self, owner: Owner, id: HandleId) -> Option<Entry> {
        self.remove(owner, id)
    }
}

pub(crate) fn registrations() -> &'static [(&'static OpDescriptor, Handler)] {
    static OPS: OnceLock<Vec<(&'static OpDescriptor, Handler)>> = OnceLock::new();
    OPS.get_or_init(|| {
        let mut r = Vec::new();
        crate::ops::runtime::register(&mut r);
        crate::ops::provider::register(&mut r);
        crate::ops::registry::register(&mut r);
        crate::ops::composite::register(&mut r);
        crate::ops::text::register(&mut r);
        crate::ops::modalities::register(&mut r);
        crate::ops::files::register(&mut r);
        crate::ops::handles::register(&mut r);
        crate::ops::trace::register(&mut r);
        crate::ops::recording::register(&mut r);
        crate::ops::session::register(&mut r);
        crate::ops::replay::register(&mut r);
        crate::ops::misc::register(&mut r);
        crate::ops::decision::register(&mut r);
        crate::ops::operation::register(&mut r);
        r.sort_unstable_by_key(|(op, _)| op.name);
        r
    })
}

/// Dispatch routes this op itself, before ordinary handle lookup.
pub const HANDLE_DROP: &str = "handle.drop";

#[must_use]
pub fn descriptor(op: &str) -> Option<&'static OpDescriptor> {
    registrations()
        .iter()
        .find(|(descriptor, _)| descriptor.name == op)
        .map(|(descriptor, _)| *descriptor)
}

/// A dispatcher with explicit registry and registration dependencies.
pub struct Dispatcher<'a> {
    registry: &'a dyn ResourceRegistry,
    registrations: &'a [(&'static OpDescriptor, Handler)],
}
impl<'a> Dispatcher<'a> {
    #[must_use]
    pub fn new(
        registry: &'a dyn ResourceRegistry,
        registrations: &'a [(&'static OpDescriptor, Handler)],
    ) -> Self {
        Self {
            registry,
            registrations,
        }
    }

    fn registration(&self, name: &str) -> Result<(&'static OpDescriptor, Handler), OpsError> {
        self.registrations
            .iter()
            .find(|(op, _)| op.name == name)
            .copied()
            .ok_or_else(|| invalid_op(name))
    }

    /// # Errors
    /// Returns the error for validation, ownership, mode or runtime failure.
    pub async fn call(
        &self,
        ctx: Ctx,
        handle: HandleId,
        name: &str,
        params: Value,
    ) -> Result<Value, OpsError> {
        let (descriptor, handler) = self.registration(name)?;
        self.call_registered(ctx, handle, descriptor, handler, params)
            .await
    }

    async fn call_registered(
        &self,
        mut ctx: Ctx,
        handle: HandleId,
        descriptor: &OpDescriptor,
        handler: Handler,
        params: Value,
    ) -> Result<Value, OpsError> {
        if !matches!(descriptor.mode, OpMode::Call | OpMode::Poll) {
            return Err(invalid_op(descriptor.name));
        }
        let Handler::Call(handler) = handler else {
            return Err(invalid_op(descriptor.name));
        };
        validate_object(&params)?;
        if descriptor.name == HANDLE_DROP {
            decode::<crate::ops::operation::EmptyParams>(params)?;
            if let Some(Entry::Operation(operation)) = self.registry.remove(ctx.owner, handle) {
                operation.consumer_close();
            }
            return Ok(Value::Null);
        }
        let target = resolve(self.registry, &ctx, handle, descriptor)?;
        validate_secondary(self.registry, &mut ctx, descriptor, &params)?;
        let abort = ctx
            .abort
            .clone()
            .filter(|_| descriptor.cancellation == Cancellation::Abort);
        if let Some(abort) = abort {
            tokio::select! {
                biased;
                () = abort.cancelled() => Err(aborted()),
                result = handler(ctx, target, params) => result,
            }
        } else {
            handler(ctx, target, params).await
        }
    }

    /// # Errors
    /// Returns errors before acceptance. Accepted failures become one terminal frame.
    pub fn stream(
        &self,
        mut ctx: Ctx,
        handle: HandleId,
        name: &str,
        params: Value,
    ) -> Result<FrameStream, OpsError> {
        (|| {
            let (descriptor, handler) = self.registration(name)?;
            if descriptor.mode != OpMode::Stream {
                return Err(invalid_op(name));
            }
            let Handler::Stream(handler) = handler else {
                return Err(invalid_op(name));
            };
            validate_object(&params)?;
            let target = resolve(self.registry, &ctx, handle, descriptor)?;
            validate_secondary(self.registry, &mut ctx, descriptor, &params)?;
            let abort = ctx
                .abort
                .get_or_insert_with(aimux_core::AbortSignal::new)
                .clone();
            let events = handler(ctx, target, params)?;
            Ok(FrameStream::new(events, Some(abort)))
        })()
    }
}

fn invalid_op(name: &str) -> OpsError {
    OpsError::Protocol(ProtocolError::InvalidOp(name.into()))
}
fn aborted() -> OpsError {
    OpsError::AiMux(aimux_core::AiMuxError::Aborted("request aborted".into()))
}
fn validate_object(params: &Value) -> Result<(), OpsError> {
    if params.is_object() {
        Ok(())
    } else {
        Err(OpsError::Protocol(ProtocolError::InvalidParams(
            "operation params must be an object".into(),
        )))
    }
}

pub(crate) fn decode<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, OpsError> {
    serde_json::from_value(params)
        .map_err(|error| OpsError::Protocol(ProtocolError::InvalidParams(error.to_string())))
}

fn resolve(
    registry: &dyn ResourceRegistry,
    ctx: &Ctx,
    handle: HandleId,
    descriptor: &OpDescriptor,
) -> Result<Target, OpsError> {
    if handle.0 == 0 && descriptor.allowed_handle_kinds.is_empty() {
        return Ok(Target::Global);
    }
    registry
        .get(ctx.owner, handle, descriptor.allowed_handle_kinds)
        .map(Target::Handle)
}

/// Resolve a secondary handle with exactly the same owner and kind policy as the primary.
/// # Errors
/// Returns InvalidParams for invalid wire IDs or InvalidHandle for resolution failure.
pub fn resolve_secondary(
    registry: &dyn ResourceRegistry,
    ctx: &Ctx,
    value: &Value,
    allowed: &[HandleKind],
) -> Result<Entry, OpsError> {
    let handle: HandleId = decode(value.clone())?;
    registry.get(ctx.owner, handle, allowed)
}

fn field_values<'a>(value: &'a Value, path: &[&str], result: &mut Vec<&'a Value>) {
    if path.is_empty() {
        if !value.is_null() {
            result.push(value);
        }
        return;
    }
    if path[0] == "*" {
        match value {
            Value::Array(values) => {
                for value in values {
                    field_values(value, &path[1..], result);
                }
            }
            Value::Object(values) => {
                for value in values.values() {
                    field_values(value, &path[1..], result);
                }
            }
            _ => {}
        }
    } else if let Some(value) = value.get(path[0]) {
        field_values(value, &path[1..], result);
    }
}

fn validate_secondary(
    registry: &dyn ResourceRegistry,
    ctx: &mut Ctx,
    descriptor: &OpDescriptor,
    params: &Value,
) -> Result<(), OpsError> {
    for (path, allowed) in descriptor.runtime_handle_fields {
        let mut values = Vec::new();
        field_values(params, &path.split('.').collect::<Vec<_>>(), &mut values);
        for value in values {
            let entry = resolve_secondary(registry, ctx, value, allowed)?;
            if let ("abort", Entry::Abort(signal)) = (*path, entry) {
                ctx.abort = Some((*signal).clone());
            }
        }
    }
    Ok(())
}

/// # Errors
/// Returns a projected error for failed call preflight or execution.
pub async fn call(
    ctx: Ctx,
    handle: HandleId,
    name: &str,
    params: Value,
) -> Result<Value, OpsError> {
    Dispatcher::new(HandleRegistry::global(), registrations())
        .call(ctx, handle, name, params)
        .await
}
/// # Errors
/// Returns a projected error before acceptance.
pub fn stream(
    ctx: Ctx,
    handle: HandleId,
    name: &str,
    params: Value,
) -> Result<FrameStream, OpsError> {
    Dispatcher::new(HandleRegistry::global(), registrations()).stream(ctx, handle, name, params)
}

#[derive(Debug)]
pub enum PollResult {
    Part(Value),
    Ended,
    Timeout,
}

/// Admit raw input without a JSON/base64 round trip or an allocation on rejection.
/// # Errors
/// Returns InvalidHandle, InvalidState or ResourceLimit.
pub fn operation_push(ctx: &Ctx, op: HandleId, data: &[u8]) -> Result<(), OpsError> {
    let Entry::Operation(operation) =
        HandleRegistry::global().get(ctx.owner, op, &[HandleKind::Operation])?
    else {
        return Err(invalid_op("operation.push"));
    };
    operation.try_push(data)
}

/// Poll the same service used by JSON operation.next.
/// # Errors
/// Returns InvalidHandle, ConcurrentConsumer or the saved terminal error.
pub async fn operation_next(
    ctx: &Ctx,
    op: HandleId,
    timeout: Duration,
) -> Result<PollResult, OpsError> {
    let Entry::Operation(operation) =
        HandleRegistry::global().get(ctx.owner, op, &[HandleKind::Operation])?
    else {
        return Err(invalid_op("operation.next"));
    };
    next_operation(&operation, timeout).await
}
pub(crate) async fn next_operation(
    operation: &crate::operation::Operation,
    timeout: Duration,
) -> Result<PollResult, OpsError> {
    match operation.next(Some(timeout)).await? {
        crate::operation::PollResult::Part(part) => serde_json::to_value(part)
            .map(PollResult::Part)
            .map_err(|error| {
                crate::envelope::BindingError::ResultSerialization(error.to_string()).into()
            }),
        crate::operation::PollResult::Ended => Ok(PollResult::Ended),
        crate::operation::PollResult::Timeout => Ok(PollResult::Timeout),
    }
}

/// Lift typed serializable events at the native framing boundary.
/// Serializer failures become terminal ResultSerialization errors.
pub fn serialize_events<S, T>(source: S) -> EventStream
where
    S: Stream<Item = Result<T, OpsError>> + Send + 'static,
    T: serde::Serialize + Send + 'static,
{
    Box::pin(source.map(|event| {
        serde_json::to_value(event?).map_err(|error| {
            crate::envelope::BindingError::ResultSerialization(error.to_string()).into()
        })
    }))
}
