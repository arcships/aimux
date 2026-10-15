//! Test-only implementation of the fixed surface consumed by the Python adapter.
//! No provider or production ops behavior is asserted by this double.
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use std::time::SystemTime;

pub mod handle {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct HandleId(pub u64);
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Owner {
        CProcess,
        Node(u64),
        Python(u64),
    }
    impl Owner {
        pub fn new_python() -> Result<Self, super::envelope::OpsError> {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            Ok(Self::Python(
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ))
        }
    }
    pub struct HandleRegistry;
    impl HandleRegistry {
        pub fn global() -> &'static Self {
            &Self
        }
        pub fn release_owner(&self, owner: Owner) -> usize {
            let mut handles = super::handles().lock().unwrap();
            let before = handles.len();
            handles.retain(|_, value| *value != owner);
            before - handles.len()
        }
    }
}
pub mod op {
    pub const OPS_VERSION: &str = "1.0";
    pub const ABI_VERSION: u32 = 1 << 16;
}
pub mod envelope {
    use super::*;
    #[derive(Clone, Debug, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ErrorEnvelope {
        pub kind: String,
        pub code: i32,
        pub message: String,
        pub error: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub retry_after_ms: Option<u64>,
    }
    #[derive(Debug)]
    pub enum BindingError {
        InvalidWireJson { argument: String, message: String },
        ResultSerialization(String),
        CallbackFailure(String),
    }
    #[derive(Debug)]
    pub enum ProtocolError {
        InvalidParams(String),
        InvalidState(String),
        ResourceLimit(String),
        ConcurrentConsumer,
    }
    #[derive(Debug)]
    pub enum OpsError {
        AiMux(aimux_core::AiMuxError),
        Binding(BindingError),
        Protocol(ProtocolError),
        Fixture(ErrorEnvelope),
    }
    impl OpsError {
        pub fn envelope(&self, _now: SystemTime) -> ErrorEnvelope {
            let (kind, code, error) = match self {
                Self::AiMux(error) => ("aimux", 18, serde_json::to_value(error).unwrap()),
                Self::Binding(BindingError::InvalidWireJson { argument, message }) => (
                    "binding",
                    202,
                    json!({"name":"InvalidWireJson", "argument":argument,"message":message}),
                ),
                Self::Binding(BindingError::ResultSerialization(message)) => (
                    "binding",
                    205,
                    json!({"name":"ResultSerialization","message":message}),
                ),
                Self::Binding(BindingError::CallbackFailure(message)) => (
                    "binding",
                    206,
                    json!({"name":"CallbackFailure","message":message}),
                ),
                Self::Protocol(error) => {
                    let (name, code) = match error {
                        ProtocolError::InvalidParams(_) => ("InvalidParams", 301),
                        ProtocolError::InvalidState(_) => ("InvalidState", 304),
                        ProtocolError::ResourceLimit(_) => ("ResourceLimit", 305),
                        ProtocolError::ConcurrentConsumer => ("ConcurrentConsumer", 303),
                    };
                    (
                        "protocol",
                        code,
                        json!({"name":name,"message":format!("{error:?}")}),
                    )
                }
                Self::Fixture(error) => return error.clone(),
            };
            ErrorEnvelope {
                kind: kind.into(),
                code,
                message: error.to_string(),
                error,
                retry_after_ms: None,
            }
        }
    }
}
pub mod frame {
    use super::envelope::ErrorEnvelope;
    #[derive(serde::Serialize)]
    #[serde(tag = "type", rename_all = "camelCase")]
    pub enum StreamFrame {
        Event {
            seq: String,
            event: serde_json::Value,
        },
        End {
            seq: String,
            outcome: Outcome,
            error: Option<ErrorEnvelope>,
        },
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub enum Outcome {
        Success,
        Error,
        Cancelled,
    }
}
pub mod manifest {
    use super::*;
    pub fn manifest() -> Value {
        json!({"abiVersion":op::ABI_VERSION,"opsVersion":op::OPS_VERSION,
            "manifestRevision":"test-double","ops":[],"handleKinds":[],"errorCodes":[]})
    }
    pub fn manifest_digest() -> String {
        "isolated-test-double".into()
    }
}
static NEXT: AtomicU64 = AtomicU64::new(1);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
fn handles() -> &'static Mutex<HashMap<u64, handle::Owner>> {
    static HANDLES: OnceLock<Mutex<HashMap<u64, handle::Owner>>> = OnceLock::new();
    HANDLES.get_or_init(Mutex::default)
}
fn fixture_error(code: i32, kind: &str, error: Value) -> envelope::ErrorEnvelope {
    envelope::ErrorEnvelope {
        kind: kind.into(),
        code,
        message: "fixture failure".into(),
        error,
        retry_after_ms: None,
    }
}
pub mod dispatch {
    pub fn operation_push(
        _ctx: &Ctx,
        _handle: super::handle::HandleId,
        _data: &[u8],
    ) -> Result<(), super::envelope::OpsError> {
        Ok(())
    }

    use super::*;
    use envelope::{OpsError, ProtocolError};
    use frame::{Outcome, StreamFrame};
    pub struct Ctx {
        pub owner: handle::Owner,
        pub abort: Option<aimux_core::AbortSignal>,
        pub now: fn() -> SystemTime,
    }
    pub type FrameStream = std::pin::Pin<Box<dyn futures::Stream<Item = StreamFrame> + Send>>;
    fn validate(ctx: &Ctx, id: handle::HandleId) -> Result<(), OpsError> {
        if id.0 != 0 && handles().lock().unwrap().get(&id.0) != Some(&ctx.owner) {
            return Err(OpsError::Fixture(fixture_error(
                203,
                "binding",
                json!({"name":"InvalidHandle","expected":[],"actual":null}),
            )));
        }
        Ok(())
    }
    pub async fn call(
        ctx: Ctx,
        id: handle::HandleId,
        op: &str,
        params: Value,
    ) -> Result<Value, OpsError> {
        if op == "handle.drop" {
            let mut handles = handles().lock().unwrap();
            if handles.get(&id.0) == Some(&ctx.owner) {
                handles.remove(&id.0);
            }
            return Ok(Value::Null);
        }
        validate(&ctx, id)?;
        if op == "test.stats" {
            return Ok(
                json!({"active":ACTIVE.load(Ordering::SeqCst),"handles":handles().lock().unwrap().len()}),
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(
            params["delayMs"].as_u64().unwrap_or(160),
        ))
        .await;
        if let Some(payload) = params.get("failure") {
            return Err(OpsError::Fixture(fixture_error(
                1,
                "aimux",
                payload.clone(),
            )));
        }
        if op == "provider.create" || op == "provider.model" || op == "recorder.create" {
            let id = NEXT.fetch_add(1, Ordering::SeqCst);
            handles().lock().unwrap().insert(id, ctx.owner);
            return Ok(json!({"handle":id.to_string()}));
        }
        Ok(
            json!({"op":op,"params":params,"owner":match ctx.owner { handle::Owner::Python(id)=>id,_=>u64::MAX }}),
        )
    }
    struct StreamGuard(u64);
    impl Drop for StreamGuard {
        fn drop(&mut self) {
            std::thread::sleep(std::time::Duration::from_millis(self.0));
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    pub fn stream(
        ctx: Ctx,
        id: handle::HandleId,
        _op: &str,
        params: Value,
    ) -> Result<FrameStream, OpsError> {
        validate(&ctx, id)?;
        std::thread::sleep(std::time::Duration::from_millis(
            params["acceptDelayMs"].as_u64().unwrap_or(0),
        ));
        if params["reject"].as_bool() == Some(true) {
            return Err(OpsError::Protocol(ProtocolError::InvalidParams(
                "rejected before acceptance".into(),
            )));
        }
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        let guard = StreamGuard(params["dropDelayMs"].as_u64().unwrap_or(0));
        let count = params["count"].as_u64().unwrap_or(2);
        let delay = params["delayMs"].as_u64().unwrap_or(0);
        let outcome = params["outcome"].as_str().unwrap_or("success").to_owned();
        let size = params["bytes"].as_u64().unwrap_or(0) as usize;
        let recoverable = params["recoverableError"].as_bool().unwrap_or(false);
        Ok(Box::pin(futures::stream::unfold(
            (0, guard, outcome),
            move |(seq, guard, outcome)| async move {
                if seq > count || (seq == count && outcome == "missing") {
                    return None;
                }
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                let frame = if seq < count {
                    StreamFrame::Event {
                        seq: seq.to_string(),
                        event: json!({"type":if recoverable { "error" } else { "text-delta" },"text":"x".repeat(size),"index":seq}),
                    }
                } else {
                    let (outcome, error) = match outcome.as_str() {
                        "error" => (
                            Outcome::Error,
                            Some(fixture_error(
                                1,
                                "aimux",
                                json!({"name":"AI_APICallError","message":"failed","statusCode":429,"isRetryable":true}),
                            )),
                        ),
                        "cancelled" => (Outcome::Cancelled, None),
                        _ => (Outcome::Success, None),
                    };
                    StreamFrame::End {
                        seq: seq.to_string(),
                        outcome,
                        error,
                    }
                };
                Some((frame, (seq + 1, guard, outcome)))
            },
        )))
    }
}
