//! Shared dispatch, validation and error projection for native adapters.
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Duration;

use futures::{Stream, StreamExt};
use serde_json::Value;

use crate::envelope::{OpsError, ProtocolError};
pub use crate::frame::FrameStream;
use crate::frame::TypedEventStream;
use crate::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use crate::op::{Cancellation, OpDescriptor, OpMode};
pub(crate) use crate::request::decode;
pub use crate::request::{Params, WireParams};

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

pub type CallFuture<'a> = Pin<Box<dyn Future<Output = Result<String, OpsError>> + Send + 'a>>;
pub type CallHandler = for<'a> fn(Ctx, Target, Params<'a>) -> CallFuture<'a>;

fn result_serialization(error: serde_json::Error) -> OpsError {
    OpsError::Binding(crate::envelope::BindingError::result_serialization(&error))
}

pub(crate) fn serialize_result(value: impl serde::Serialize) -> Result<String, OpsError> {
    // Unary results can carry attachments and exports. The frame limit is
    // enforced by the bounded writer at the streaming boundary.
    serde_json::to_string(&value).map_err(result_serialization)
}

pub type EventStream = Pin<Box<dyn Stream<Item = Result<Value, OpsError>> + Send>>;

#[derive(Clone, Copy)]
pub enum Handler {
    Call(CallHandler),
    /// Structured stream seam for callers with prebuilt JSON events.
    Stream(fn(Ctx, Target, Value) -> Result<EventStream, OpsError>),
    TypedStream(fn(Ctx, Target, Params<'_>) -> Result<TypedEventStream, OpsError>),
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

type Registration = (&'static OpDescriptor, Handler);

fn lookup(table: &[Registration], name: &str) -> Option<Registration> {
    table
        .binary_search_by(|(descriptor, _)| descriptor.name.cmp(name))
        .ok()
        .map(|index| table[index])
}

#[must_use]
pub fn descriptor(op: &str) -> Option<&'static OpDescriptor> {
    lookup(registrations(), op).map(|(descriptor, _)| descriptor)
}

/// A dispatcher with explicit registry and registration dependencies.
pub struct Dispatcher<'a> {
    registry: &'a dyn ResourceRegistry,
    registrations: Cow<'a, [Registration]>,
}
impl<'a> Dispatcher<'a> {
    #[must_use]
    pub fn new(registry: &'a dyn ResourceRegistry, registrations: &'a [Registration]) -> Self {
        let registrations = if registrations
            .windows(2)
            .all(|pair| pair[0].0.name <= pair[1].0.name)
        {
            Cow::Borrowed(registrations)
        } else {
            let mut sorted = registrations.to_vec();
            sorted.sort_by_key(|(descriptor, _)| descriptor.name);
            Cow::Owned(sorted)
        };
        Self {
            registry,
            registrations,
        }
    }

    fn global() -> Dispatcher<'static> {
        Dispatcher {
            registry: HandleRegistry::global(),
            registrations: Cow::Borrowed(registrations()),
        }
    }

    fn registration(&self, name: &str) -> Result<Registration, OpsError> {
        lookup(&self.registrations, name).ok_or_else(|| invalid_op(name))
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
        let json = self.call_params(ctx, handle, name, params.into()).await?;
        serde_json::from_str(&json).map_err(result_serialization)
    }

    async fn call_params(
        &self,
        mut ctx: Ctx,
        handle: HandleId,
        name: &str,
        params: Params<'_>,
    ) -> Result<String, OpsError> {
        let (descriptor, handler) = self.registration(name)?;
        let Handler::Call(run) = handler else {
            return Err(invalid_op(name));
        };
        if !matches!(descriptor.mode, OpMode::Call | OpMode::Poll) {
            return Err(invalid_op(name));
        }
        params.validate_object()?;
        if descriptor.name == HANDLE_DROP {
            decode::<crate::ops::operation::EmptyParams>(params)?;
            if let Some(Entry::Operation(operation)) = self.registry.remove(ctx.owner, handle) {
                operation.consumer_close();
            }
            return Ok("null".into());
        }
        let target = resolve(self.registry, &ctx, handle, descriptor)?;
        validate_secondary(self.registry, &mut ctx, descriptor, &params)?;
        let abort = ctx
            .abort
            .clone()
            .filter(|_| descriptor.cancellation == Cancellation::Abort);
        let call = async move { run(ctx, target, params).await };
        if let Some(abort) = abort {
            tokio::select! {
                biased;
                () = abort.cancelled() => Err(aborted()),
                result = call => result,
            }
        } else {
            call.await
        }
    }

    /// # Errors
    /// Returns errors before acceptance. Accepted failures become one terminal frame.
    pub fn stream(
        &self,
        ctx: Ctx,
        handle: HandleId,
        name: &str,
        params: Value,
    ) -> Result<FrameStream, OpsError> {
        self.stream_params(ctx, handle, name, params.into())
    }

    fn stream_params(
        &self,
        mut ctx: Ctx,
        handle: HandleId,
        name: &str,
        params: Params<'_>,
    ) -> Result<FrameStream, OpsError> {
        let (descriptor, handler) = self.registration(name)?;
        if descriptor.mode != OpMode::Stream || matches!(handler, Handler::Call(_)) {
            return Err(invalid_op(name));
        }
        params.validate_object()?;
        let target = resolve(self.registry, &ctx, handle, descriptor)?;
        validate_secondary(self.registry, &mut ctx, descriptor, &params)?;
        let abort = ctx
            .abort
            .get_or_insert_with(aimux_core::AbortSignal::new)
            .clone();
        match handler {
            Handler::Stream(run) => Ok(FrameStream::new(
                run(ctx, target, params.into_value()?)?,
                Some(abort),
            )),
            Handler::TypedStream(run) => {
                Ok(FrameStream::typed(run(ctx, target, params)?, Some(abort)))
            }
            Handler::Call(_) => unreachable!("validated stream handler"),
        }
    }
}

fn invalid_op(name: &str) -> OpsError {
    OpsError::Protocol(ProtocolError::InvalidOp(name.into()))
}
fn aborted() -> OpsError {
    OpsError::AiMux(aimux_core::AiMuxError::Aborted("request aborted".into()))
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

fn visit_value(
    value: &Value,
    path: &[&str],
    visit: &mut impl FnMut(&Value) -> Result<(), OpsError>,
) -> Result<(), OpsError> {
    match path.split_first() {
        None if !value.is_null() => visit(value)?,
        Some((&"*", rest)) => match value {
            Value::Array(values) => {
                for value in values {
                    visit_value(value, rest, visit)?;
                }
            }
            Value::Object(values) => {
                for value in values.values() {
                    visit_value(value, rest, visit)?;
                }
            }
            _ => {}
        },
        Some((field, rest)) => {
            if let Some(value) = value.get(*field) {
                visit_value(value, rest, visit)?;
            }
        }
        None => {}
    }
    Ok(())
}

fn validate_secondary(
    registry: &dyn ResourceRegistry,
    ctx: &mut Ctx,
    descriptor: &OpDescriptor,
    params: &Params<'_>,
) -> Result<(), OpsError> {
    let fields = descriptor.runtime_handle_fields;
    if fields.is_empty() {
        return Ok(());
    }
    let mut resolve = |path: &[&str], id, allowed: &[HandleKind]| {
        let entry = registry.get(ctx.owner, id, allowed)?;
        if let (["abort"], Entry::Abort(signal)) = (path, entry) {
            ctx.abort = Some((*signal).clone());
        }
        Ok(())
    };

    match params {
        Params::Wire(wire) => {
            for ((path, allowed), handles) in
                fields.iter().zip(crate::request::probe(*wire, fields)?)
            {
                for raw in handles {
                    resolve(path, crate::request::decode_handle(raw)?, allowed)?;
                }
            }
        }
        Params::Value(value) => {
            for (path, allowed) in fields {
                visit_value(value, path, &mut |value| {
                    resolve(path, decode::<HandleId>(value.clone())?, allowed)
                })?;
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
    Dispatcher::global().call(ctx, handle, name, params).await
}

/// Dispatch a native call and return the handler's encoded JSON.
/// # Errors
/// Returns a projected error for failed call preflight or execution.
pub async fn call_wire(
    ctx: Ctx,
    handle: HandleId,
    name: &str,
    params: WireParams<'_>,
) -> Result<String, OpsError> {
    Dispatcher::global()
        .call_params(ctx, handle, name, Params::Wire(params))
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
    Dispatcher::global().stream(ctx, handle, name, params)
}

/// # Errors
/// Returns a projected error before acceptance.
pub fn stream_wire(
    ctx: Ctx,
    handle: HandleId,
    name: &str,
    params: WireParams<'_>,
) -> Result<FrameStream, OpsError> {
    Dispatcher::global().stream_params(ctx, handle, name, Params::Wire(params))
}

pub use crate::operation::PollResult;

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
    operation.next(Some(timeout)).await
}

/// The wire representation of a direct operation poll.
#[derive(Debug)]
pub enum WirePollResult {
    Part(String),
    Ended,
    Timeout,
}

/// Poll an operation, encoding a returned part directly to JSON.
///
/// # Errors
/// Returns the same operation errors as [operation_next], or ResultSerialization.
pub async fn operation_next_json(
    ctx: &Ctx,
    op: HandleId,
    timeout: Duration,
) -> Result<WirePollResult, OpsError> {
    match operation_next(ctx, op, timeout).await? {
        PollResult::Part(part) => serialize_result(part).map(WirePollResult::Part),
        PollResult::Ended => Ok(WirePollResult::Ended),
        PollResult::Timeout => Ok(WirePollResult::Timeout),
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
        serde_json::to_value(event?)
            .map_err(|error| crate::envelope::BindingError::result_serialization(&error).into())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::HandleKind::Abort;
    use std::sync::Arc;

    fn ctx() -> Ctx {
        Ctx {
            owner: Owner::CProcess,
            abort: None,
        }
    }

    fn wire(bytes: &[u8]) -> WireParams<'_> {
        WireParams::new(bytes, "params_json")
    }

    #[tokio::test]
    async fn native_call_decodes_direct_json_and_keeps_input_errors() {
        let request = br#" { "kind": "fetch", "proxy": {"no_proxy":"localhost"} } "#;
        let output = call_wire(ctx(), HandleId(0), "transport.create", wire(request))
            .await
            .unwrap();
        let output: Value = serde_json::from_str(&output).unwrap();
        let id: HandleId = decode(output["handle"].clone()).unwrap();
        assert!(
            HandleRegistry::global()
                .get(Owner::CProcess, id, &[HandleKind::Fetch])
                .is_ok()
        );
        assert!(
            HandleRegistry::global()
                .remove(Owner::CProcess, id)
                .is_some()
        );

        for (input, code) in [
            (&b"{} {}"[..], 202),
            (b"{", 202),
            (b"null", 301),
            (br#"{"unknown":true}"#, 301),
        ] {
            let error = call_wire(ctx(), HandleId(0), "runtime.describe", wire(input))
                .await
                .unwrap_err();
            assert_eq!(error.code(), code);
        }
    }

    #[test]
    fn probe_skips_unrelated_fields_and_validates_nested_handles_in_descriptor_order() {
        static DESCRIPTOR: OpDescriptor = OpDescriptor {
            runtime_handle_fields: &[
                (&["settings", "groups", "*", "handle"], &[Abort]),
                (&["settings", "another"], &[Abort]),
                (&["abort"], &[Abort]),
            ],
            ..crate::ops::runtime::OPS[0]
        };
        let registry = HandleRegistry::default();
        let signal = aimux_core::AbortSignal::new();
        let local = registry
            .insert(Owner::CProcess, Entry::Abort(Arc::new(signal.clone())))
            .unwrap();
        let foreign = registry
            .insert(
                Owner::new_node().unwrap(),
                Entry::Abort(Arc::new(signal.clone())),
            )
            .unwrap();

        let input = format!(
            r#"{{
            "settings": {{
                "groups": [
                    {{"handle": 99, "handle": "{}", "unused":{{"handle": 7}}}},
                    {{"handle":null}}, {{}}, null, false
                ],
                "another":"{}"
            }},
            "unused": ["not a handle",{{"abort":0}}],
            "abort": "{}", "abort":"{}"
        }}"#,
            local.0, local.0, foreign.0, local.0
        );
        let mut context = ctx();
        validate_secondary(
            &registry,
            &mut context,
            &DESCRIPTOR,
            &Params::Wire(wire(input.as_bytes())),
        )
        .unwrap();
        signal.abort();
        assert!(context.abort.unwrap().is_aborted());

        for input in [
            r#"{}"#,
            r#"{"settings":{"groups":{"a":{"handle":42},"a":{}}}}"#,
            r#"{"settings":{"groups":[{"handle":42}]},"settings":null}"#,
        ] {
            validate_secondary(
                &registry,
                &mut ctx(),
                &DESCRIPTOR,
                &Params::Wire(wire(input.as_bytes())),
            )
            .unwrap();
        }

        for (input, code) in [
            (r#"{"settings":{"groups":[{"handle":42}]}}"#.to_owned(), 301),
            (
                format!(
                    r#"{{"settings":{{"groups":[{{"handle":"{}"}}]}}}}"#,
                    foreign.0
                ),
                203,
            ),
            (
                format!(r#"{{"abort":3,"settings":{{"another":"{}"}}}}"#, foreign.0),
                203,
            ),
            (r#"{"unused":[1,]}"#.to_owned(), 202),
        ] {
            let error = validate_secondary(
                &registry,
                &mut ctx(),
                &DESCRIPTOR,
                &Params::Wire(wire(input.as_bytes())),
            )
            .unwrap_err();
            assert_eq!(error.code(), code, "{input}");
        }
    }
}
