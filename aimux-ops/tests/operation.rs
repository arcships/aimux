use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use aimux_core::AbortSignal;
use aimux_core::error::AiMuxError;
use aimux_core::transcription_model::{
    AudioChunk, TranscriptionCallOptions, TranscriptionModel, TranscriptionResult,
    TranscriptionStreamOptions, TranscriptionStreamPart, TranscriptionStreamResult,
};
use aimux_ops::envelope::{BindingError, OpsError, ProtocolError};
use aimux_ops::operation::{
    CancelResult, Operation, OperationLimits, OperationOptions, OperationOutcome, OperationState,
    PollResult,
};
use futures::future::BoxFuture;
use futures::{Stream, StreamExt, poll};

const WAIT: Duration = Duration::from_secs(2);

enum Mode {
    Unsupported,
    ConnectFailure,
    Pending,
    Echo(AbortSignal),
    Events(Mutex<Vec<Result<TranscriptionStreamPart, AiMuxError>>>),
    BlockingDrop(Arc<DropGate>),
}

struct MockModel {
    mode: Mode,
    received: Arc<Mutex<Vec<Vec<u8>>>>,
    pulled: Arc<AtomicUsize>,
    provider_abort: Arc<Mutex<Option<AbortSignal>>>,
}

impl MockModel {
    fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            received: Arc::default(),
            pulled: Arc::default(),
            provider_abort: Arc::default(),
        })
    }
}

impl TranscriptionModel for MockModel {
    fn provider(&self) -> &str {
        "mock"
    }
    fn model_id(&self) -> &str {
        "realtime"
    }

    fn do_generate<'a, 'b, 'c>(
        &'a self,
        _: &'b TranscriptionCallOptions,
    ) -> BoxFuture<'c, Result<TranscriptionResult, AiMuxError>>
    where
        'a: 'c,
        'b: 'c,
        Self: 'c,
    {
        Box::pin(async { Err(AiMuxError::other("batch generation is unused")) })
    }

    fn do_stream(
        &self,
        options: TranscriptionStreamOptions,
    ) -> Option<BoxFuture<'_, Result<TranscriptionStreamResult, AiMuxError>>> {
        *self.provider_abort.lock().unwrap() = options.abort_signal;
        if matches!(self.mode, Mode::Unsupported) {
            return None;
        }
        Some(Box::pin(async move {
            let stream: Pin<
                Box<dyn Stream<Item = Result<TranscriptionStreamPart, AiMuxError>> + Send>,
            > = match &self.mode {
                Mode::Unsupported => return Err(AiMuxError::other("unused")),
                Mode::ConnectFailure => return Err(AiMuxError::other("connect failed")),
                Mode::Pending => return std::future::pending().await,
                Mode::Echo(gate) => {
                    gate.cancelled().await;
                    Box::pin(EchoStream {
                        audio: options.audio,
                        received: self.received.clone(),
                    })
                }
                Mode::Events(events) => {
                    let events = std::mem::take(&mut *events.lock().unwrap());
                    let pulled = self.pulled.clone();
                    Box::pin(futures::stream::iter(events).inspect(move |_| {
                        pulled.fetch_add(1, Ordering::SeqCst);
                    }))
                }
                Mode::BlockingDrop(gate) => Box::pin(BlockingDropStream(gate.clone())),
            };
            Ok(TranscriptionStreamResult {
                stream,
                request: None,
                response: None,
            })
        }))
    }
}

struct EchoStream {
    audio: Pin<Box<dyn Stream<Item = AudioChunk> + Send>>,
    received: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Stream for EchoStream {
    type Item = Result<TranscriptionStreamPart, AiMuxError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.audio.as_mut().poll_next(cx) {
            Poll::Ready(Some(AudioChunk::Binary(bytes))) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                self.received.lock().unwrap().push(bytes);
                Poll::Ready(Some(Ok(delta(&text))))
            }
            Poll::Ready(Some(AudioChunk::Base64(_))) => panic!("operation must pass binary audio"),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Default)]
struct DropGate {
    polled: AbortSignal,
    entered: AbortSignal,
    released: Mutex<bool>,
    changed: Condvar,
}

struct BlockingDropStream(Arc<DropGate>);

impl Stream for BlockingDropStream {
    type Item = Result<TranscriptionStreamPart, AiMuxError>;
    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.polled.abort();
        Poll::Pending
    }
}

impl Drop for BlockingDropStream {
    fn drop(&mut self) {
        self.0.entered.abort();
        let mut released = self.0.released.lock().unwrap();
        while !*released {
            released = self.0.changed.wait(released).unwrap();
        }
    }
}

struct ReleaseDropGate(Arc<DropGate>);

impl Drop for ReleaseDropGate {
    fn drop(&mut self) {
        *self.0.released.lock().unwrap() = true;
        self.0.changed.notify_all();
    }
}

fn delta(text: &str) -> TranscriptionStreamPart {
    TranscriptionStreamPart::TranscriptDelta {
        id: None,
        delta: text.into(),
        provider_metadata: None,
    }
}

fn events(parts: Vec<Result<TranscriptionStreamPart, AiMuxError>>) -> Arc<MockModel> {
    MockModel::new(Mode::Events(Mutex::new(parts)))
}

fn open_gate() -> AbortSignal {
    let gate = AbortSignal::new();
    gate.abort();
    gate
}

async fn start(model: Arc<MockModel>, limits: OperationLimits) -> Arc<Operation> {
    Operation::start(
        model,
        OperationOptions {
            limits,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap_or_else(|_| panic!("mock operation failed admission"))
}

#[test]
fn drop_after_runtime_shutdown_is_safe() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let model = MockModel::new(Mode::Pending);
    let operation = runtime.block_on(start(model.clone(), OperationLimits::default()));
    drop(runtime);
    drop(operation);
    assert!(
        model
            .provider_abort
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_aborted()
    );
}

#[test]
fn terminate_after_runtime_shutdown_is_safe() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let operation = runtime.block_on(start(
        MockModel::new(Mode::Pending),
        OperationLimits::default(),
    ));
    drop(runtime);
    futures::executor::block_on(operation.terminate());
    assert!(operation.status().cleanup_failure.is_none());
    assert!(matches!(operation.status().state, OperationState::Terminal));
}

async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(WAIT, async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("state did not progress");
}

async fn next(operation: &Operation) -> Result<PollResult, OpsError> {
    tokio::time::timeout(WAIT, operation.next(None))
        .await
        .expect("next hung")
}

fn assert_part(result: Result<PollResult, OpsError>, text: &str) {
    match result {
        Ok(PollResult::Part(TranscriptionStreamPart::TranscriptDelta { delta, .. })) => {
            assert_eq!(delta, text);
        }
        _ => panic!("expected transcript delta"),
    }
}

#[tokio::test]
async fn capability_failure_precedes_acceptance() {
    let result = Operation::start(
        MockModel::new(Mode::Unsupported),
        OperationOptions::default(),
        None,
    )
    .await;
    assert!(matches!(
        result,
        Err(OpsError::AiMux(AiMuxError::UnsupportedFunctionality(_)))
    ));
}

#[tokio::test]
async fn connection_failure_is_saved_and_delivered_once() {
    let operation = start(
        MockModel::new(Mode::ConnectFailure),
        OperationLimits::default(),
    )
    .await;
    assert!(
        matches!(next(&operation).await, Err(OpsError::AiMux(AiMuxError::Other { message, .. })) if message == "connect failed")
    );
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
    let terminal = operation.status().terminal.unwrap();
    assert_eq!(terminal.outcome, OperationOutcome::Error);
    assert!(terminal.error.is_some());
}

#[tokio::test]
async fn pushes_copy_data_and_input_done_is_idempotent() {
    let model = MockModel::new(Mode::Echo(AbortSignal::new()));
    let operation = start(model.clone(), OperationLimits::default()).await;
    let mut buffer = b"original".to_vec();
    // SAFETY: the buffer remains readable and unchanged until push completes.
    unsafe { operation.push_raw(buffer.as_ptr(), buffer.len()) }
        .unwrap_or_else(|_| panic!("raw input rejected"))
        .await
        .unwrap_or_else(|_| panic!("push failed"));
    buffer.fill(b'x');
    operation.input_done();
    let sequence = operation.status().input_done_sequence;
    operation.input_done();
    assert_eq!(operation.status().input_done_sequence, sequence);
    assert_eq!(operation.status().state, OperationState::InputClosed);
    assert!(matches!(
        operation.push(b"late").await,
        Err(OpsError::Protocol(ProtocolError::InvalidState(_)))
    ));
    if let Mode::Echo(gate) = &model.mode {
        gate.abort();
    }
    assert_part(next(&operation).await, "original");
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
    assert_eq!(*model.received.lock().unwrap(), vec![b"original".to_vec()]);
    assert_eq!(operation.cancel(), CancelResult::AlreadyTerminal);
    assert_eq!(
        operation.status().terminal.unwrap().outcome,
        OperationOutcome::Success
    );
}

#[tokio::test]
async fn close_preserves_a_push_admitted_while_full() {
    let gate = AbortSignal::new();
    let model = MockModel::new(Mode::Echo(gate.clone()));
    let operation = start(
        model.clone(),
        OperationLimits {
            input_items: 1,
            ..Default::default()
        },
    )
    .await;
    operation
        .push(b"first")
        .await
        .unwrap_or_else(|_| panic!("push failed"));
    let mut second = Box::pin(operation.push(b"second"));
    assert!(poll!(second.as_mut()).is_pending());
    operation.input_done();
    assert_eq!(operation.status().input_done_sequence.as_deref(), Some("2"));
    assert!(operation.push(b"late").await.is_err());
    gate.abort();
    tokio::time::timeout(WAIT, second)
        .await
        .unwrap()
        .unwrap_or_else(|_| panic!("admitted push rejected"));
    assert_part(next(&operation).await, "first");
    assert_part(next(&operation).await, "second");
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
    assert_eq!(
        *model.received.lock().unwrap(),
        vec![b"first".to_vec(), b"second".to_vec()]
    );
}

#[tokio::test]
async fn dropped_waiting_push_does_not_leave_a_sequence_gap() {
    let gate = AbortSignal::new();
    let operation = start(
        MockModel::new(Mode::Echo(gate.clone())),
        OperationLimits {
            input_items: 1,
            ..Default::default()
        },
    )
    .await;
    operation
        .push(b"first")
        .await
        .unwrap_or_else(|_| panic!("push failed"));
    let mut discarded = Box::pin(operation.push(b"discarded"));
    assert!(poll!(discarded.as_mut()).is_pending());
    let mut third = Box::pin(operation.push(b"third"));
    assert!(poll!(third.as_mut()).is_pending());
    drop(discarded);
    operation.input_done();
    gate.abort();
    third
        .await
        .unwrap_or_else(|_| panic!("sequence gap stalled push"));
    assert_part(next(&operation).await, "first");
    assert_part(next(&operation).await, "third");
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
}

#[tokio::test]
async fn input_byte_budget_backpressures_before_item_limit() {
    let gate = AbortSignal::new();
    let operation = start(
        MockModel::new(Mode::Echo(gate.clone())),
        OperationLimits {
            input_bytes: 8,
            ..Default::default()
        },
    )
    .await;
    operation
        .push(b"12345678")
        .await
        .unwrap_or_else(|_| panic!("push failed"));
    let mut waiting = Box::pin(operation.push(b"x"));
    assert!(poll!(waiting.as_mut()).is_pending());
    assert_eq!(operation.status().queued_input_items, 1);
    assert_eq!(operation.status().queued_input_bytes, 8);
    gate.abort();
    waiting
        .await
        .unwrap_or_else(|_| panic!("capacity not released"));
    operation.input_done();
    assert_part(next(&operation).await, "12345678");
    assert_part(next(&operation).await, "x");
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
}

#[tokio::test]
async fn input_checks_raw_pointer_and_lengths_before_reading() {
    let operation = start(
        MockModel::new(Mode::Echo(open_gate())),
        OperationLimits::default(),
    )
    .await;
    // SAFETY: zero-length input needs no readable pointer.
    unsafe { operation.push_raw(std::ptr::null(), 0) }
        .unwrap_or_else(|_| panic!("NULL+0 rejected"))
        .await
        .unwrap_or_else(|_| panic!("empty push failed"));
    // SAFETY: invalid arguments must be rejected before any read.
    let null = unsafe { operation.push_raw(std::ptr::null(), 1) };
    assert!(matches!(
        null,
        Err(OpsError::Binding(BindingError::NullPointer { .. }))
    ));
    // SAFETY: an impossible length is rejected without reading this pointer.
    let oversized =
        unsafe { operation.push_raw(std::ptr::NonNull::<u8>::dangling().as_ptr(), usize::MAX) };
    assert!(matches!(
        oversized,
        Err(OpsError::Protocol(ProtocolError::ResourceLimit(_)))
    ));
    // SAFETY: the overflowing address range must be rejected without reading it.
    let wrapped = unsafe { operation.push_raw(usize::MAX as *const u8, 1) };
    assert!(matches!(
        wrapped,
        Err(OpsError::Protocol(ProtocolError::ResourceLimit(_)))
    ));
    let big = vec![0; 1024 * 1024 + 1];
    assert!(matches!(
        operation.push(&big).await,
        Err(OpsError::Protocol(ProtocolError::ResourceLimit(_)))
    ));
    operation.input_done();
    assert_part(next(&operation).await, "");
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
}

#[tokio::test]
async fn default_input_item_and_byte_limits_are_both_enforced() {
    let operation = start(MockModel::new(Mode::Pending), OperationLimits::default()).await;
    for _ in 0..64 {
        operation
            .push(&[])
            .await
            .unwrap_or_else(|_| panic!("push failed"));
    }
    let mut extra = Box::pin(operation.push(&[]));
    assert!(poll!(extra.as_mut()).is_pending());
    assert_eq!(operation.status().queued_input_items, 64);
    operation.cancel();
    assert!(extra.await.is_err());
    let operation = start(MockModel::new(Mode::Pending), OperationLimits::default()).await;
    let chunk = vec![0; 1024 * 1024];
    for _ in 0..8 {
        operation
            .push(&chunk)
            .await
            .unwrap_or_else(|_| panic!("push failed"));
    }
    let mut extra = Box::pin(operation.push(b"x"));
    assert!(poll!(extra.as_mut()).is_pending());
    assert_eq!(operation.status().queued_input_bytes, 8 * 1024 * 1024);
    operation.cancel();
    assert!(extra.await.is_err());
}

#[tokio::test]
async fn a_concurrent_reader_fails_and_timeout_keeps_operation_running() {
    let operation = start(MockModel::new(Mode::Pending), OperationLimits::default()).await;
    let mut first = Box::pin(operation.next(None));
    assert!(poll!(first.as_mut()).is_pending());
    assert!(matches!(
        operation.next(Some(Duration::ZERO)).await,
        Err(OpsError::Protocol(ProtocolError::ConcurrentConsumer))
    ));
    drop(first);
    assert!(matches!(
        operation.next(Some(Duration::ZERO)).await,
        Ok(PollResult::Timeout)
    ));
    assert_eq!(operation.status().state, OperationState::Running);
    assert_eq!(operation.cancel(), CancelResult::First);
    assert_eq!(operation.cancel(), CancelResult::AlreadyRequested);
    assert!(matches!(
        next(&operation).await,
        Err(OpsError::AiMux(AiMuxError::Aborted(_)))
    ));
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
}

#[tokio::test]
async fn cancelling_a_pending_next_releases_the_reader() {
    let operation = start(MockModel::new(Mode::Pending), OperationLimits::default()).await;
    let mut reader = Box::pin(operation.next(None));
    assert!(poll!(reader.as_mut()).is_pending());
    operation.cancel();
    assert!(matches!(
        reader.await,
        Err(OpsError::AiMux(AiMuxError::Aborted(_)))
    ));
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
}

#[tokio::test]
async fn full_output_queue_never_loses_cancellation_terminal() {
    let model = events((0..300).map(|_| Ok(delta("data"))).collect());
    let operation = start(model.clone(), OperationLimits::default()).await;
    until(|| operation.status().queued_output_items == 256).await;
    assert_eq!(model.pulled.load(Ordering::SeqCst), 256);
    assert_eq!(operation.cancel(), CancelResult::First);
    operation.terminate().await;
    for _ in 0..256 {
        assert_part(next(&operation).await, "data");
    }
    assert!(matches!(
        next(&operation).await,
        Err(OpsError::AiMux(AiMuxError::Aborted(_)))
    ));
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
    let status = operation.status();
    assert_eq!(
        status.terminal.unwrap().outcome,
        OperationOutcome::Cancelled
    );
    assert!(status.cleanup_failure.is_none());
}

#[tokio::test]
async fn output_bytes_are_bounded_and_events_remain_lossless() {
    let text = "a".repeat(80);
    let size = serde_json::to_vec(&delta(&text)).unwrap().len();
    let model = events((0..4).map(|_| Ok(delta(&text))).collect());
    let operation = start(
        model,
        OperationLimits {
            output_bytes: size * 2,
            ..Default::default()
        },
    )
    .await;
    until(|| operation.status().queued_output_items == 2).await;
    assert_eq!(operation.status().queued_output_bytes, size * 2);
    for _ in 0..4 {
        assert_part(next(&operation).await, &text);
    }
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
    assert_eq!(operation.status().queued_output_bytes, 0);
}

#[tokio::test]
async fn provider_failure_after_a_full_queue_is_not_a_successful_eof() {
    let mut parts: Vec<_> = (0..256).map(|_| Ok(delta("data"))).collect();
    parts.push(Err(AiMuxError::Timeout("provider timeout".into())));
    let operation = start(events(parts), OperationLimits::default()).await;
    until(|| operation.status().queued_output_items == 256).await;
    for _ in 0..256 {
        assert_part(next(&operation).await, "data");
    }
    assert!(matches!(
        next(&operation).await,
        Err(OpsError::AiMux(AiMuxError::Timeout(_)))
    ));
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
    assert_eq!(
        operation.status().terminal.unwrap().outcome,
        OperationOutcome::Error
    );
}

#[tokio::test]
async fn v4_error_and_finish_are_data_parts() {
    let error = TranscriptionStreamPart::Error {
        error: serde_json::json!({"recoverable": true}),
    };
    let finish = TranscriptionStreamPart::Finish {
        text: "done".into(),
        segments: vec![],
        language: None,
        duration_in_seconds: None,
        provider_metadata: None,
    };
    let operation = start(
        events(vec![Ok(error), Ok(delta("continued")), Ok(finish)]),
        OperationLimits::default(),
    )
    .await;
    assert!(matches!(
        next(&operation).await,
        Ok(PollResult::Part(TranscriptionStreamPart::Error { .. }))
    ));
    assert_part(next(&operation).await, "continued");
    assert!(matches!(
        next(&operation).await,
        Ok(PollResult::Part(TranscriptionStreamPart::Finish { .. }))
    ));
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
    assert_eq!(
        operation.status().terminal.unwrap().outcome,
        OperationOutcome::Success
    );
}

#[tokio::test]
async fn oversized_serialized_event_is_a_terminal_resource_limit() {
    let operation = start(
        events(vec![Ok(delta(&"\n".repeat(600_000)))]),
        OperationLimits::default(),
    )
    .await;
    assert!(matches!(
        next(&operation).await,
        Err(OpsError::Protocol(ProtocolError::ResourceLimit(_)))
    ));
    assert_eq!(operation.status().queued_output_items, 0);
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
}

#[tokio::test]
async fn invalid_or_unlimited_budgets_are_rejected_before_start() {
    for limits in [
        OperationLimits {
            input_items: 0,
            ..Default::default()
        },
        OperationLimits {
            output_bytes: usize::MAX,
            ..Default::default()
        },
        OperationLimits {
            item_bytes: 1024 * 1024 + 1,
            ..Default::default()
        },
    ] {
        let result = Operation::start(
            MockModel::new(Mode::Pending),
            OperationOptions {
                limits,
                ..Default::default()
            },
            None,
        )
        .await;
        assert!(matches!(
            result,
            Err(OpsError::Protocol(ProtocolError::ResourceLimit(_)))
        ));
    }
}

#[tokio::test]
async fn shared_abort_cancels_connection_and_unblocks_a_push() {
    let abort = AbortSignal::new();
    let model = MockModel::new(Mode::Pending);
    let operation = Operation::start(
        model.clone(),
        OperationOptions {
            limits: OperationLimits {
                input_items: 1,
                ..Default::default()
            },
            ..Default::default()
        },
        Some(abort.clone()),
    )
    .await
    .unwrap_or_else(|_| panic!("start failed"));
    operation
        .push(b"full")
        .await
        .unwrap_or_else(|_| panic!("push failed"));
    let mut waiting = Box::pin(operation.push(b"waiting"));
    assert!(poll!(waiting.as_mut()).is_pending());
    abort.abort();
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap()
        .expect_err("push must fail");
    assert!(
        model
            .provider_abort
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_aborted()
    );
    assert!(matches!(
        next(&operation).await,
        Err(OpsError::AiMux(AiMuxError::Aborted(_)))
    ));
}

#[tokio::test]
async fn consumer_close_abandons_queued_parts_and_marks_cancellation() {
    let operation = start(
        events((0..300).map(|_| Ok(delta("data"))).collect()),
        OperationLimits::default(),
    )
    .await;
    until(|| operation.status().queued_output_items == 256).await;
    operation.consumer_close();
    assert!(operation.status().consumer_closed);
    assert_eq!(operation.status().queued_output_items, 0);
    assert!(matches!(
        next(&operation).await,
        Err(OpsError::AiMux(AiMuxError::Aborted(_)))
    ));
    assert!(matches!(next(&operation).await, Ok(PollResult::Ended)));
}

#[tokio::test]
async fn last_owner_drop_aborts_the_provider_without_a_reference_cycle() {
    let model = MockModel::new(Mode::Pending);
    let operation = start(model.clone(), OperationLimits::default()).await;
    let abort = model.provider_abort.lock().unwrap().clone().unwrap();
    let second_owner = operation.clone();
    drop(operation);
    assert!(!abort.is_aborted());
    std::thread::spawn(move || drop(second_owner))
        .join()
        .unwrap();
    assert!(abort.is_aborted());
    until(|| Arc::strong_count(&model) == 1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cleanup_is_capped_at_five_seconds_and_failure_is_recorded() {
    let gate = Arc::new(DropGate::default());
    let release = ReleaseDropGate(gate.clone());
    let operation = start(
        MockModel::new(Mode::BlockingDrop(gate.clone())),
        OperationLimits::default(),
    )
    .await;
    tokio::time::timeout(WAIT, gate.polled.cancelled())
        .await
        .expect("stream was not polled");
    operation.cancel();
    tokio::time::timeout(WAIT, gate.entered.cancelled())
        .await
        .expect("stream was not dropped");
    let started = std::time::Instant::now();
    let mut cleanup = Box::pin(operation.terminate());
    assert!(poll!(cleanup.as_mut()).is_pending());
    drop(cleanup);
    let joined = tokio::time::timeout(Duration::from_secs(7), async {
        while operation.status().cleanup_failure.is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let elapsed = started.elapsed();
    let failure = operation.status().cleanup_failure;
    // Always release the deliberately blocked destructor before assertions.
    drop(release);
    assert!(joined.is_ok());
    assert!(elapsed >= Duration::from_secs(5));
    assert!(elapsed < Duration::from_secs(7));
    assert!(
        failure
            .as_deref()
            .is_some_and(|message| message.contains("exceeded 5 seconds"))
    );
    assert_eq!(
        operation.status().terminal.unwrap().outcome,
        OperationOutcome::Cancelled
    );
}

fn dispatch_context(owner: aimux_ops::Owner) -> aimux_ops::Ctx {
    aimux_ops::Ctx { owner, abort: None }
}

#[tokio::test]
async fn native_binary_and_json_poll_share_the_registered_operation_service() {
    use aimux_ops::dispatch;
    use aimux_ops::{Entry, HandleId, HandleRegistry, Owner};
    let owner = Owner::new_node().unwrap();
    let registry = HandleRegistry::global();
    let gate = AbortSignal::new();
    let model = MockModel::new(Mode::Echo(gate.clone()));
    let handle = registry
        .insert(owner, Entry::Transcription(model.clone()))
        .unwrap();
    let value = dispatch::call(
        dispatch_context(owner),
        handle,
        "transcription.start",
        serde_json::json!({}),
    )
    .await
    .unwrap();
    let op: HandleId = serde_json::from_value(value).unwrap();
    assert_eq!(
        dispatch::call(
            dispatch_context(owner),
            op,
            "operation.push",
            serde_json::json!({})
        )
        .await
        .unwrap_err()
        .code(),
        300
    );
    dispatch::operation_push(&dispatch_context(owner), op, &[0, 128, 255]).unwrap();
    for _ in 0..2 {
        dispatch::call(
            dispatch_context(owner),
            op,
            "operation.inputDone",
            serde_json::json!({}),
        )
        .await
        .unwrap();
    }
    gate.abort();
    assert!(matches!(
        dispatch::operation_next(&dispatch_context(owner), op, WAIT)
            .await
            .unwrap(),
        dispatch::PollResult::Part(_)
    ));
    assert_eq!(
        dispatch::call(
            dispatch_context(owner),
            op,
            "operation.next",
            serde_json::json!({"timeoutMs":2_000})
        )
        .await
        .unwrap(),
        serde_json::json!({"type":"ended"})
    );
    assert_eq!(*model.received.lock().unwrap(), vec![vec![0, 128, 255]]);
    let status = dispatch::call(
        dispatch_context(owner),
        op,
        "operation.status",
        serde_json::json!({}),
    )
    .await
    .unwrap();
    assert_eq!(status["terminal"]["outcome"], "success");
    assert_eq!(
        dispatch::call(
            dispatch_context(owner),
            op,
            "operation.cancel",
            serde_json::json!({})
        )
        .await
        .unwrap(),
        "alreadyTerminal"
    );
    registry.release_owner(owner);
}

#[tokio::test]
async fn binary_admission_checks_limits_and_owner_teardown_signals_retained_operations() {
    use aimux_ops::dispatch;
    use aimux_ops::{Entry, HandleKind, HandleRegistry, Owner};
    let owner = Owner::new_python().unwrap();
    let registry = HandleRegistry::global();
    let handle = registry
        .insert(owner, Entry::Transcription(MockModel::new(Mode::Pending)))
        .unwrap();
    let op = serde_json::from_value(
        dispatch::call(
            dispatch_context(owner),
            handle,
            "transcription.start",
            serde_json::json!({}),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    let Entry::Operation(operation) = registry.get(owner, op, &[HandleKind::Operation]).unwrap()
    else {
        panic!("operation");
    };
    let foreign = Owner::new_node().unwrap();
    assert_eq!(
        dispatch::operation_push(&dispatch_context(foreign), op, &[])
            .unwrap_err()
            .code(),
        203
    );
    let too_large = vec![0; aimux_ops::limits::ITEM_BYTES + 1];
    assert_eq!(
        dispatch::operation_push(&dispatch_context(owner), op, &too_large)
            .unwrap_err()
            .code(),
        305
    );
    for _ in 0..aimux_ops::limits::AUDIO_INPUT_ITEMS {
        dispatch::operation_push(&dispatch_context(owner), op, &[]).unwrap();
    }
    assert_eq!(
        dispatch::operation_push(&dispatch_context(owner), op, &[])
            .unwrap_err()
            .code(),
        305
    );
    assert_eq!(
        operation.status().queued_input_items,
        aimux_ops::limits::AUDIO_INPUT_ITEMS
    );
    let context = dispatch_context(owner);
    let mut reader = Box::pin(dispatch::operation_next(&context, op, WAIT));
    assert!(poll!(reader.as_mut()).is_pending());
    assert_eq!(
        dispatch::call(
            dispatch_context(owner),
            op,
            "operation.next",
            serde_json::json!({})
        )
        .await
        .unwrap_err()
        .code(),
        303
    );
    assert_eq!(registry.release_owner(owner), 2);
    assert_eq!(
        operation.status().terminal.unwrap().outcome,
        OperationOutcome::Cancelled
    );
    assert_eq!(reader.await.unwrap_err().code(), 13);
    operation.terminate().await;
}

#[tokio::test]
async fn terminal_status_and_native_error_share_one_envelope() {
    use aimux_ops::dispatch;
    use aimux_ops::{Entry, HandleRegistry, Owner};
    let mut api = aimux_core::error::ApiCallError::new(
        "terminal",
        "https://mock.invalid",
        serde_json::json!({}),
    );
    api.response_headers = Some(std::collections::HashMap::from([(
        "Retry-After".into(),
        "Tue, 14 Nov 2023 22:13:22 GMT".into(),
    )]));
    let model = MockModel::new(Mode::Events(Mutex::new(vec![Err(AiMuxError::ApiCall(
        Box::new(api),
    ))])));
    let operation = Operation::start(model, OperationOptions::default(), None)
        .await
        .unwrap();
    let owner = Owner::new_node().unwrap();
    let op = HandleRegistry::global()
        .insert(owner, Entry::Operation(operation.clone()))
        .unwrap();
    let error = dispatch::operation_next(&dispatch_context(owner), op, WAIT)
        .await
        .unwrap_err();
    let saved = operation.status().terminal.unwrap().error.unwrap();
    assert_eq!(error.envelope(), saved);
    assert_eq!(
        saved.error["responseHeaders"]["Retry-After"],
        "Tue, 14 Nov 2023 22:13:22 GMT"
    );
    assert!(matches!(
        dispatch::operation_next(&dispatch_context(owner), op, WAIT)
            .await
            .unwrap(),
        dispatch::PollResult::Ended
    ));
    assert_eq!(operation.status().terminal.unwrap().error.unwrap(), saved);
    HandleRegistry::global().release_owner(owner);
}

#[tokio::test]
async fn registered_controls_reject_unknown_fields_before_starting_provider_work() {
    use aimux_ops::dispatch;
    use aimux_ops::{Entry, HandleRegistry, Owner};
    let owner = Owner::new_node().unwrap();
    let model = MockModel::new(Mode::Pending);
    let handle = HandleRegistry::global()
        .insert(owner, Entry::Transcription(model.clone()))
        .unwrap();
    for params in [
        serde_json::json!({"unknown":true}),
        serde_json::json!({"inputAudioFormat":{"formatType":"audio/pcm", "unknown":true}}),
        serde_json::json!({"timeout":{"unknown":1}}),
    ] {
        assert_eq!(
            dispatch::call(
                dispatch_context(owner),
                handle,
                "transcription.start",
                params
            )
            .await
            .unwrap_err()
            .code(),
            301
        );
    }
    assert!(model.provider_abort.lock().unwrap().is_none());
    HandleRegistry::global().release_owner(owner);
}
