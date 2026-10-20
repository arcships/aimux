//! Shared test source for the Node and Python native contract builds.
//! No provider/network double replaces aimux-ops. Only the scripted source is
//! injected at its public Dispatcher seam, as in the ops crate's native vectors.
//! Kept out of release builds by the contract-tests feature.
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use aimux_core::error::ApiCallError;
use aimux_core::{AbortSignal, AiMuxError};
use aimux_ops::dispatch::{Ctx, Dispatcher, EventStream, Handler, Target, serialize_events};
use aimux_ops::envelope::{OpsError, ProtocolError};
use aimux_ops::handle::{HandleId, HandleRegistry, Owner};
use aimux_ops::limits::{QueueEvent, QueueLimits, bounded_queue, check_wire_item};
use aimux_ops::op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope, TypeRef};
use futures::{Stream, StreamExt};
use serde::{Serialize, Serializer};
use serde_json::Value;
use tokio::sync::Notify;

#[derive(serde::Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
enum SourceStep {
    Event { event: Value },
    RepeatEvent { count: usize, event: Value },
    Wait { gate: String },
    TerminalError { kind: String, message: String },
    SerializationFailure { message: String },
    Eof,
}

#[derive(serde::Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
enum Control {
    Read { count: usize },
    Drain,
    DrainNative,
    Cancel,
    CloseReader,
    StartProducer,
    WaitForQueueFull,
    WaitForSourceGate { gate: String },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    accepted: bool,
    preflight_error: Option<Value>,
    frames: Vec<Value>,
    consumer_frames: Vec<Value>,
    abort_observed: Option<bool>,
    queue_full_observed: bool,
    consumer_closed: bool,
    source_started: bool,
    source_events_polled: usize,
}

static SOURCE: Mutex<Option<EventStream>> = Mutex::new(None);
static DESCRIPTOR: OpDescriptor = OpDescriptor {
    name: "test.scenario",
    since_ops_version: aimux_ops::op::OPS_VERSION,
    mode: OpMode::Stream,
    allowed_handle_kinds: &[],
    request_type: TypeRef::of::<Value>(),
    result_type: TypeRef::of::<()>(),
    event_type: Some(TypeRef::of::<Value>()),
    scope: Scope::Current,
    terminal_type: Some(TypeRef::of::<aimux_ops::frame::StreamFrame>()),
    runtime_handle_fields: &[],
    required_capabilities: &[],
    cancellation: Cancellation::Abort,
    side_effects: false,
    result_ownership: "owned",
    request_byte_policy: RequestBytePolicy::Json,
    retry_owner: "none",
    source_symbols: &[],
};
fn accept(_: Ctx, _: Target, params: Value) -> Result<EventStream, OpsError> {
    if params
        .get("fault")
        .is_some_and(|fault| fault["stage"] == "preflight")
    {
        return Err(ProtocolError::InvalidParams("mock invalid params".into()).into());
    }
    Ok(SOURCE
        .lock()
        .unwrap()
        .take()
        .expect("installed native source"))
}
#[derive(Default)]
struct Stats {
    started: AtomicBool,
    polled: AtomicUsize,
    gate: Mutex<Option<String>>,
    full: AtomicBool,
    changed: Notify,
}
struct Script {
    steps: VecDeque<SourceStep>,
    stats: Arc<Stats>,
}
enum Event {
    Value(Value),
    Broken(String),
}
impl Serialize for Event {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Value(value) => value.serialize(serializer),
            Self::Broken(message) => Err(serde::ser::Error::custom(message)),
        }
    }
}
impl Stream for Script {
    type Item = Result<Event, OpsError>;
    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.stats.started.store(true, Ordering::SeqCst);
        let Some(step) = this.steps.pop_front() else {
            return Poll::Ready(None);
        };
        let event =
            match step {
                SourceStep::Event { event } => Event::Value(event),
                SourceStep::RepeatEvent { count, event } if count > 0 => {
                    if count > 1 {
                        this.steps.push_front(SourceStep::RepeatEvent {
                            count: count - 1,
                            event: event.clone(),
                        });
                    }
                    Event::Value(event)
                }
                SourceStep::Wait { gate } => {
                    *this.stats.gate.lock().unwrap() = Some(gate.clone());
                    this.steps.push_front(SourceStep::Wait { gate });
                    this.stats.changed.notify_waiters();
                    return Poll::Pending;
                }
                SourceStep::TerminalError { kind, message } => {
                    assert_eq!(kind, "apiCall");
                    return Poll::Ready(Some(Err(AiMuxError::ApiCall(Box::new(
                        ApiCallError::new(message, "https://mock.invalid", serde_json::json!({})),
                    ))
                    .into())));
                }
                SourceStep::SerializationFailure { message } => {
                    return Poll::Ready(Some(Ok(Event::Broken(message))));
                }
                SourceStep::Eof => return Poll::Ready(None),
                SourceStep::RepeatEvent { .. } => panic!("empty repeat"),
            };
        this.stats.polled.fetch_add(1, Ordering::SeqCst);
        Poll::Ready(Some(Ok(event)))
    }
}

async fn barrier(stats: &Stats, predicate: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let changed = stats.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if predicate() {
                break;
            }
            changed.await;
        }
    })
    .await
    .expect("native instrumentation barrier stalled");
}

// The host supplies only inputs. Output comes from the real Dispatcher,
// serialize_events, FrameStream and bounded_queue implementations. Both native
// adapters call this through their ordinary call boundary in a test-only build.
// Serializing calls also protects the one-shot handler's SOURCE installation.
pub(crate) async fn replay(input: Value, owner: Owner) -> Value {
    static RUN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _run = RUN.lock().await;
    serde_json::to_value(observe(input, owner).await).unwrap()
}

async fn observe(mut input: Value, owner: Owner) -> Observation {
    assert!(
        input.as_object_mut().unwrap().remove("expected").is_none(),
        "driver cannot inspect expected output"
    );
    let stats = Arc::new(Stats::default());
    let steps = serde_json::from_value::<Vec<SourceStep>>(input["script"].clone()).unwrap();
    *SOURCE.lock().unwrap() = Some(serialize_events(Script {
        steps: steps.into(),
        stats: stats.clone(),
    }));
    let abort = AbortSignal::new();
    let ctx = Ctx {
        owner,
        abort: Some(abort.clone()),
    };
    let registry = HandleRegistry::default();
    let table = [(&DESCRIPTOR, Handler::Stream(accept))];
    let dispatcher = Dispatcher::new(&registry, &table);
    let result = dispatcher.stream(ctx, HandleId(0), DESCRIPTOR.name, input.clone());
    let mut observed = Observation {
        accepted: result.is_ok(),
        preflight_error: None,
        frames: Vec::new(),
        consumer_frames: Vec::new(),
        abort_observed: Some(false),
        queue_full_observed: false,
        consumer_closed: false,
        source_started: false,
        source_events_polled: 0,
    };
    let mut frames = match result {
        Ok(frames) => frames,
        Err(error) => {
            observed.preflight_error =
                Some(serde_json::to_value(error.envelope()).unwrap());
            SOURCE.lock().unwrap().take();
            return observed;
        }
    };
    let limits = &input["queueLimits"];
    let limits = QueueLimits::output()
        .lowered(
            limits["items"].as_u64().unwrap() as usize,
            limits["bytes"].as_u64().unwrap() as usize,
            limits["maxEventBytes"].as_u64().unwrap() as usize,
        )
        .unwrap();
    let (sender, mut receiver) = bounded_queue::<Box<[u8]>, Value>(limits);
    let native_frames = Arc::new(Mutex::new(Vec::new()));
    let log = native_frames.clone();
    let producer_stats = stats.clone();
    let producer_abort = abort.clone();
    let producer = tokio::spawn(async move {
        while let Some(frame) = frames.next().await {
            let terminal = matches!(frame, aimux_ops::StreamFrame::End { .. });
            let value = serde_json::to_value(&frame).unwrap();
            log.lock().unwrap().push(value.clone());
            if terminal {
                sender.finish(value);
                continue;
            }
            let size = check_wire_item(&frame, limits.max_item_bytes()).unwrap();
            let permit = sender.reserve(size, Some(&producer_abort)).await.unwrap();
            permit
                .send(serde_json::to_vec(&frame).unwrap().into_boxed_slice())
                .unwrap();
            if sender.usage().0 == limits.max_items() {
                producer_stats.full.store(true, Ordering::SeqCst);
                producer_stats.changed.notify_waiters();
            }
        }
    });
    let controls = serde_json::from_value::<Vec<Control>>(input["controls"].clone()).unwrap();
    for control in controls {
        match control {
            Control::Read { count } => {
                for _ in 0..count {
                    observed.consumer_frames.push(
                        receive(&mut receiver)
                            .await
                            .expect("expected consumer frame"),
                    );
                }
            }
            Control::Drain => {
                while let Some(frame) = receive(&mut receiver).await {
                    observed.consumer_frames.push(frame);
                }
            }
            Control::WaitForSourceGate { gate } => {
                barrier(&stats, || {
                    stats.gate.lock().unwrap().as_ref() == Some(&gate)
                })
                .await
            }
            Control::WaitForQueueFull => {
                barrier(&stats, || stats.full.load(Ordering::SeqCst)).await;
                observed.queue_full_observed = true;
            }
            Control::Cancel => abort.abort(),
            Control::CloseReader => {
                observed.consumer_closed = true;
                receiver.close();
                abort.abort();
            }
            Control::DrainNative | Control::StartProducer => {}
        }
    }
    tokio::time::timeout(Duration::from_secs(3), producer)
        .await
        .unwrap()
        .unwrap();
    observed.frames = native_frames.lock().unwrap().clone();
    observed.abort_observed = Some(abort.is_aborted());
    observed.source_started = stats.started.load(Ordering::SeqCst);
    observed.source_events_polled = stats.polled.load(Ordering::SeqCst);
    observed
}
async fn receive(
    receiver: &mut aimux_ops::limits::QueueReceiver<Box<[u8]>, Value>,
) -> Option<Value> {
    match receiver.recv().await? {
        QueueEvent::Data(data) => Some(serde_json::from_slice(&data).unwrap()),
        QueueEvent::Terminal(frame) => Some(frame),
    }
}

// A connecting provider never polls audio, so one admitted chunk deterministically
// fills the lowered input queue. This is a real Operation, not a push double.
struct ConnectingModel;
impl aimux_core::transcription_model::TranscriptionModel for ConnectingModel {
    fn provider(&self) -> &str {
        "contract"
    }
    fn model_id(&self) -> &str {
        "connecting"
    }
    fn do_generate<'a, 'b, 'c>(
        &'a self,
        _: &'b aimux_core::transcription_model::TranscriptionCallOptions,
    ) -> futures::future::BoxFuture<
        'c,
        Result<aimux_core::transcription_model::TranscriptionResult, AiMuxError>,
    >
    where
        'a: 'c,
        'b: 'c,
        Self: 'c,
    {
        Box::pin(async { unreachable!("only streaming is used") })
    }
    fn do_stream(
        &self,
        _: aimux_core::transcription_model::TranscriptionStreamOptions,
    ) -> Option<
        futures::future::BoxFuture<
            '_,
            Result<aimux_core::transcription_model::TranscriptionStreamResult, AiMuxError>,
        >,
    > {
        Some(Box::pin(std::future::pending()))
    }
}

pub(crate) async fn operation(owner: Owner) -> HandleId {
    let operation = aimux_ops::operation::Operation::start(
        Arc::new(ConnectingModel),
        aimux_ops::operation::OperationOptions {
            limits: aimux_ops::operation::OperationLimits {
                input_items: 1,
                ..Default::default()
            },
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap_or_else(|_| panic!("accept connecting operation"));
    HandleRegistry::global()
        .insert(owner, aimux_ops::handle::Entry::Operation(operation))
        .unwrap()
}

pub(crate) fn released(owner: Owner, id: HandleId) -> bool {
    HandleRegistry::global()
        .get(owner, id, &[aimux_ops::handle::HandleKind::Operation])
        .is_err()
}
