//! Bidirectional transcription with ordered input admission and a separate terminal slot.

use std::collections::{BTreeSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime};

use aimux_core::AbortSignal;
use aimux_core::error::AiMuxError;
use aimux_core::transcription_model::{
    AudioChunk, InputAudioFormat, TranscriptionModel, TranscriptionStreamOptions,
    TranscriptionStreamPart,
};
use futures::task::AtomicWaker;
use futures::{FutureExt, Stream, StreamExt};
use serde::Serialize;
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinHandle;

use crate::envelope::{BindingError, ErrorEnvelope, OpsError, ProtocolError};

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

use crate::limits::{AUDIO_INPUT_ITEMS as INPUT_ITEMS, ITEM_BYTES, OUTPUT_ITEMS, QUEUE_BYTES};

/// Resource limits may only lower the protocol defaults.
#[derive(Clone, Copy, Debug)]
pub struct OperationLimits {
    pub input_items: usize,
    pub input_bytes: usize,
    pub output_items: usize,
    pub output_bytes: usize,
    pub item_bytes: usize,
}

impl Default for OperationLimits {
    fn default() -> Self {
        Self {
            input_items: INPUT_ITEMS,
            input_bytes: QUEUE_BYTES,
            output_items: OUTPUT_ITEMS,
            output_bytes: QUEUE_BYTES,
            item_bytes: ITEM_BYTES,
        }
    }
}

impl OperationLimits {
    fn validate(self) -> Result<Self, OpsError> {
        for (value, maximum) in [
            (self.input_items, INPUT_ITEMS),
            (self.input_bytes, QUEUE_BYTES),
            (self.output_items, OUTPUT_ITEMS),
            (self.output_bytes, QUEUE_BYTES),
            (self.item_bytes, ITEM_BYTES),
        ] {
            if value == 0 || value > maximum {
                return Err(resource_limit(
                    "operation limits must be positive and cannot exceed defaults",
                ));
            }
        }
        Ok(self)
    }
}

/// Plain options for `transcription.start`; audio comes from subsequent pushes.
#[derive(Default)]
pub struct OperationOptions {
    pub input_audio_format: Option<InputAudioFormat>,
    pub provider_options: Option<aimux_core::shared::SharedProviderOptions>,
    pub headers: Option<aimux_core::shared::SharedHeaders>,
    pub include_raw_chunks: bool,
    pub timeout: Option<aimux_core::options::TimeoutConfiguration>,
    pub limits: OperationLimits,
}

/// Poll timeouts do not terminate an operation.
#[derive(Debug)]
pub enum PollResult {
    Part(TranscriptionStreamPart),
    Ended,
    Timeout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CancelResult {
    First,
    AlreadyRequested,
    AlreadyTerminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationOutcome {
    Success,
    Error,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationState {
    Running,
    InputClosed,
    Terminal,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSummary {
    pub outcome: OperationOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorEnvelope>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationCapabilities {
    pub push: bool,
    pub input_done: bool,
    pub next: bool,
    pub cancel: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationStatus {
    pub subkind: &'static str,
    pub state: OperationState,
    pub capabilities: OperationCapabilities,
    pub consumer_closed: bool,
    pub queued_input_items: usize,
    pub queued_input_bytes: usize,
    pub queued_output_items: usize,
    pub queued_output_bytes: usize,
    /// The close boundary in the same admission sequence as pushes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_done_sequence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup_failure: Option<String>,
}

enum InputState {
    Open,
    Closed { sequence: u64 },
}

enum ConsumerState {
    Open,
    Closed,
}

struct Terminal {
    summary: TerminalSummary,
    // Taken exactly once by next; the independently cloned summary survives.
    error: Option<OpsError>,
}

struct State {
    input: InputState,
    consumer: ConsumerState,
    admission_sequence: u64,
    send_sequence: u64,
    completed_admissions: BTreeSet<u64>,
    input_queue: VecDeque<Vec<u8>>,
    input_bytes: usize,
    output_queue: VecDeque<(TranscriptionStreamPart, usize)>,
    output_bytes: usize,
    terminal: Option<Terminal>,
    cleanup_failure: Option<String>,
}

struct Shared {
    state: Mutex<State>,
    limits: OperationLimits,
    now: fn() -> SystemTime,
    changed: Notify,
    input_waker: AtomicWaker,
    abort: AbortSignal,
    reader: tokio::sync::Mutex<()>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn wake(&self) {
        self.changed.notify_waiters();
        self.input_waker.wake();
    }

    fn finish(&self, outcome: OperationOutcome, error: Option<OpsError>) {
        let error = error.map(|error| error.project_with_clock(self.now));
        {
            let mut state = self.lock();
            if state.terminal.is_some() {
                return;
            }
            let summary = TerminalSummary {
                outcome,
                error: error
                    .as_ref()
                    .map(|error| error.envelope(SystemTime::UNIX_EPOCH)),
            };
            state.terminal = Some(Terminal { summary, error });
            state.input_queue.clear();
            state.input_bytes = 0;
        }
        self.wake();
    }

    fn cancel(&self) -> CancelResult {
        let error = OpsError::AiMux(AiMuxError::Aborted("operation cancelled".into()))
            .project_with_clock(self.now);
        let result = {
            let mut state = self.lock();
            match &state.terminal {
                Some(terminal) if terminal.summary.outcome == OperationOutcome::Cancelled => {
                    return CancelResult::AlreadyRequested;
                }
                Some(_) => return CancelResult::AlreadyTerminal,
                None => {
                    state.terminal = Some(Terminal {
                        summary: TerminalSummary {
                            outcome: OperationOutcome::Cancelled,
                            error: Some(error.envelope(SystemTime::UNIX_EPOCH)),
                        },
                        error: Some(error),
                    });
                    state.input_queue.clear();
                    state.input_bytes = 0;
                    CancelResult::First
                }
            }
        };
        self.abort.abort();
        self.wake();
        result
    }

    fn record_cleanup_failure(&self, message: String) {
        self.lock().cleanup_failure = Some(message);
        self.wake();
    }
}

/// A realtime transcription operation. Dropping the last owner cancels its task.
/// The driver holds only `Shared`, so it cannot keep the operation alive itself.
pub struct Operation {
    shared: Arc<Shared>,
    task: Mutex<Option<JoinHandle<()>>>,
    runtime: tokio::runtime::Handle,
}

impl Operation {
    /// Check streaming capability before accepting; connection failures are terminal results.
    ///
    /// # Errors
    /// Returns invalid limits/runtime state or unsupported streaming capability.
    pub async fn start(
        model: Arc<dyn TranscriptionModel>,
        options: OperationOptions,
        user_abort: Option<AbortSignal>,
    ) -> Result<Arc<Self>, OpsError> {
        Self::start_with_clock(model, options, user_abort, SystemTime::now).await
    }

    /// Start using the clock captured from the dispatch context.
    /// # Errors
    /// Returns preflight errors before operation acceptance.
    pub async fn start_with_clock(
        model: Arc<dyn TranscriptionModel>,
        options: OperationOptions,
        user_abort: Option<AbortSignal>,
        now: fn() -> SystemTime,
    ) -> Result<Arc<Self>, OpsError> {
        let limits = options.limits.validate()?;
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| invalid_state("starting an operation requires a Tokio runtime"))?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                input: InputState::Open,
                consumer: ConsumerState::Open,
                admission_sequence: 0,
                send_sequence: 0,
                completed_admissions: BTreeSet::new(),
                input_queue: VecDeque::new(),
                input_bytes: 0,
                output_queue: VecDeque::new(),
                output_bytes: 0,
                terminal: None,
                cleanup_failure: None,
            }),
            limits,
            now,
            changed: Notify::new(),
            input_waker: AtomicWaker::new(),
            abort: AbortSignal::new(),
            reader: tokio::sync::Mutex::new(()),
        });
        let (accepted_tx, accepted_rx) = oneshot::channel();
        let driver_shared = shared.clone();
        let task = runtime.spawn(async move {
            let driver = run_driver(
                driver_shared.clone(),
                model,
                options,
                user_abort,
                accepted_tx,
            );
            if std::panic::AssertUnwindSafe(driver)
                .catch_unwind()
                .await
                .is_err()
            {
                driver_shared.finish(
                    OperationOutcome::Error,
                    Some(invalid_state("transcription driver panicked")),
                );
            }
        });
        // Hold the operation during admission so cancellation of start also drops/cancels it.
        let operation = Arc::new(Self {
            shared,
            task: Mutex::new(Some(task)),
            runtime,
        });
        accepted_rx
            .await
            .map_err(|_| invalid_state("transcription driver failed before acceptance"))??;
        Ok(operation)
    }

    /// Copy one audio chunk once capacity is available, preserving admission order.
    ///
    /// # Errors
    /// Returns ResourceLimit for oversize chunks, or InvalidState after input closure/termination.
    pub async fn push(&self, bytes: &[u8]) -> Result<(), OpsError> {
        self.check_input_length(bytes.len())?;
        let admission = {
            let mut state = self.shared.lock();
            if !matches!(state.input, InputState::Open) || state.terminal.is_some() {
                return Err(invalid_state("operation input is closed"));
            }
            let sequence = state.admission_sequence;
            state.admission_sequence = sequence
                .checked_add(1)
                .ok_or_else(|| resource_limit("input admission sequence exhausted"))?;
            Admission {
                shared: &self.shared,
                sequence,
            }
        };
        loop {
            let changed = self.shared.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = self.shared.lock();
                if state.terminal.is_some() {
                    return Err(invalid_state("operation has terminated"));
                }
                let byte_count = state
                    .input_bytes
                    .checked_add(bytes.len())
                    .ok_or_else(|| resource_limit("input byte count overflow"))?;
                if state.send_sequence == admission.sequence
                    && state.input_queue.len() < self.shared.limits.input_items
                    && byte_count <= self.shared.limits.input_bytes
                {
                    state.input_queue.push_back(bytes.to_vec());
                    state.input_bytes = byte_count;
                    break;
                }
            }
            changed.await;
        }
        drop(admission);
        self.shared.wake();
        Ok(())
    }

    /// Synchronous binary admission. Full queues reject before copying the buffer.
    /// # Errors
    /// Returns ResourceLimit for full/oversize input and InvalidState after closure.
    pub fn try_push(&self, bytes: &[u8]) -> Result<(), OpsError> {
        self.check_input_length(bytes.len())?;
        {
            let mut state = self.shared.lock();
            if !matches!(state.input, InputState::Open) || state.terminal.is_some() {
                return Err(invalid_state("operation input is closed"));
            }
            let total = state
                .input_bytes
                .checked_add(bytes.len())
                .ok_or_else(|| resource_limit("input byte count overflow"))?;
            if state.admission_sequence != state.send_sequence
                || state.input_queue.len() >= self.shared.limits.input_items
                || total > self.shared.limits.input_bytes
            {
                return Err(resource_limit("operation input queue is full"));
            }
            let sequence = state
                .admission_sequence
                .checked_add(1)
                .ok_or_else(|| resource_limit("input admission sequence exhausted"))?;
            state.input_queue.push_back(bytes.to_vec());
            state.input_bytes = total;
            state.admission_sequence = sequence;
            state.send_sequence = sequence;
        }
        self.shared.wake();
        Ok(())
    }

    fn check_input_length(&self, length: usize) -> Result<(), OpsError> {
        if length > self.shared.limits.item_bytes || length > self.shared.limits.input_bytes {
            return Err(resource_limit("audio chunk exceeds the input budget"));
        }
        Ok(())
    }

    /// Validate a C buffer before constructing its slice, then push it.
    /// NULL with zero length is legal. No allocation occurs before length checks.
    ///
    /// # Safety
    /// For nonzero length, `data` must address readable initialized bytes in one allocation
    /// and remain valid until the returned future completes or is dropped.
    ///
    /// # Errors
    /// Returns NullPointer/ResourceLimit before reading invalid buffer arguments.
    pub unsafe fn push_raw(
        &self,
        data: *const u8,
        length: usize,
    ) -> Result<impl Future<Output = Result<(), OpsError>> + Send + '_, OpsError> {
        if data.is_null() && length != 0 {
            return Err(OpsError::Binding(BindingError::NullPointer {
                argument: "data",
            }));
        }
        self.check_input_length(length)?;
        if data.addr().checked_add(length).is_none() {
            return Err(resource_limit("audio buffer address range overflow"));
        }
        let bytes = if length == 0 {
            &[]
        } else {
            // SAFETY: the caller guarantees validity; size was checked above.
            unsafe { std::slice::from_raw_parts(data, length) }
        };
        Ok(self.push(bytes))
    }

    /// Close admission, leaving every previously admitted push eligible to complete.
    pub fn input_done(&self) {
        {
            let mut state = self.shared.lock();
            if matches!(state.input, InputState::Open) {
                state.input = InputState::Closed {
                    sequence: state.admission_sequence,
                };
            }
        }
        self.shared.wake();
    }

    /// Receive data, the one terminal error, or Ended. A second concurrent reader fails.
    ///
    /// # Errors
    /// Returns ConcurrentConsumer or the saved terminal failure (only once).
    pub async fn next(&self, timeout: Option<Duration>) -> Result<PollResult, OpsError> {
        let _reader = self
            .shared
            .reader
            .try_lock()
            .map_err(|_| OpsError::Protocol(ProtocolError::ConcurrentConsumer))?;
        let receive = async {
            loop {
                let changed = self.shared.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                {
                    let mut state = self.shared.lock();
                    if let Some((part, bytes)) = state.output_queue.pop_front() {
                        state.output_bytes -= bytes;
                        drop(state);
                        self.shared.wake();
                        return Ok(PollResult::Part(part));
                    }
                    if let Some(terminal) = &mut state.terminal {
                        return match terminal.error.take() {
                            Some(error) => Err(error),
                            None => Ok(PollResult::Ended),
                        };
                    }
                }
                changed.await;
            }
        };
        match timeout {
            None => receive.await,
            Some(duration) => tokio::time::timeout(duration, receive)
                .await
                .unwrap_or(Ok(PollResult::Timeout)),
        }
    }

    /// Cancellation wins only if no terminal result has already been stored.
    pub fn cancel(&self) -> CancelResult {
        self.shared.cancel()
    }

    /// Stop consuming, abandon undelivered events, and cancel unfinished work.
    pub fn consumer_close(&self) {
        self.cancel();
        {
            let mut state = self.shared.lock();
            state.consumer = ConsumerState::Closed;
            state.output_queue.clear();
            state.output_bytes = 0;
        }
        self.shared.wake();
    }

    /// Obtain a durable terminal summary, including after its error has been consumed.
    #[must_use]
    pub fn status(&self) -> OperationStatus {
        let state = self.shared.lock();
        OperationStatus {
            subkind: "transcription",
            state: if state.terminal.is_some() {
                OperationState::Terminal
            } else if matches!(state.input, InputState::Closed { .. }) {
                OperationState::InputClosed
            } else {
                OperationState::Running
            },
            capabilities: OperationCapabilities {
                push: true,
                input_done: true,
                next: true,
                cancel: true,
            },
            consumer_closed: matches!(state.consumer, ConsumerState::Closed),
            queued_input_items: state.input_queue.len(),
            queued_input_bytes: state.input_bytes,
            queued_output_items: state.output_queue.len(),
            queued_output_bytes: state.output_bytes,
            input_done_sequence: match state.input {
                InputState::Open => None,
                InputState::Closed { sequence } => Some(sequence.to_string()),
            },
            terminal: state
                .terminal
                .as_ref()
                .map(|terminal| terminal.summary.clone()),
            cleanup_failure: state.cleanup_failure.clone(),
        }
    }

    /// Cancel and join outside all operation locks, with a five-second cleanup cap.
    pub async fn terminate(&self) {
        self.cancel();
        self.input_done();
        let task = self
            .task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(task) = task {
            // A completed driver needs no cleanup runtime or timer. Runtime
            // shutdown also completes it by cancellation.
            if task.is_finished() {
                if let Err(error) = task.await
                    && !error.is_cancelled()
                {
                    self.shared
                        .record_cleanup_failure(format!("operation driver join failed: {error}"));
                }
                return;
            }
            let shared = self.shared.clone();
            // The cleanup task keeps running even if this caller stops awaiting terminate.
            let cleanup = self
                .runtime
                .spawn(async move { join_driver(&shared, task).await });
            // Shutdown can race with spawn; it already disposes the driver.
            if let Err(error) = cleanup.await
                && !error.is_cancelled()
            {
                self.shared
                    .record_cleanup_failure(format!("operation cleanup task failed: {error}"));
            }
        }
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        self.consumer_close();
        let task = self
            .task
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(task) = task
            && !task.is_finished()
        {
            let shared = self.shared.clone();
            self.runtime
                .spawn(async move { join_driver(&shared, task).await });
        }
    }
}

/// Cancelling a waiting push must release its admission sequence as well as capacity.
struct Admission<'a> {
    shared: &'a Shared,
    sequence: u64,
}

impl Drop for Admission<'_> {
    fn drop(&mut self) {
        {
            let mut state = self.shared.lock();
            state.completed_admissions.insert(self.sequence);
            loop {
                let sequence = state.send_sequence;
                if !state.completed_admissions.remove(&sequence) {
                    break;
                }
                // Every admitted sequence has already passed checked_add in push.
                state.send_sequence = sequence.saturating_add(1);
            }
        }
        self.shared.wake();
    }
}

struct AudioStream {
    shared: Arc<Shared>,
}

impl Stream for AudioStream {
    type Item = AudioChunk;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.shared.input_waker.register(cx.waker());
        let mut state = self.shared.lock();
        if state.terminal.is_some() {
            return Poll::Ready(None);
        }
        if let Some(bytes) = state.input_queue.pop_front() {
            state.input_bytes -= bytes.len();
            drop(state);
            self.shared.changed.notify_waiters();
            return Poll::Ready(Some(AudioChunk::Binary(bytes)));
        }
        match state.input {
            InputState::Closed { sequence } if state.send_sequence == sequence => Poll::Ready(None),
            _ => Poll::Pending,
        }
    }
}

async fn run_driver(
    shared: Arc<Shared>,
    model: Arc<dyn TranscriptionModel>,
    options: OperationOptions,
    user_abort: Option<AbortSignal>,
    accepted: oneshot::Sender<Result<(), OpsError>>,
) {
    let options = TranscriptionStreamOptions {
        audio: Box::pin(AudioStream {
            shared: shared.clone(),
        }),
        input_audio_format: options.input_audio_format.unwrap_or(InputAudioFormat {
            format_type: "audio/pcm".into(),
            rate: None,
        }),
        provider_options: options.provider_options,
        abort_signal: Some(shared.abort.clone()),
        headers: options.headers,
        include_raw_chunks: options.include_raw_chunks,
        timeout: options.timeout,
    };
    let Some(connect) = model.do_stream(options) else {
        let _ = accepted.send(Err(OpsError::AiMux(AiMuxError::UnsupportedFunctionality(
            format!(
                "The {} model \"{}\" does not support streaming transcription.",
                model.provider(),
                model.model_id()
            ),
        ))));
        return;
    };
    if accepted.send(Ok(())).is_err() {
        shared.cancel();
        return;
    }
    let external_cancel = async {
        match user_abort {
            Some(signal) => signal.cancelled().await,
            None => std::future::pending::<()>().await,
        }
        // Store cancellation before select drops a potentially slow provider future.
        shared.cancel();
    };
    let forward = async {
        let result = connect.await.map_err(OpsError::AiMux)?;
        let mut stream = result.stream;
        loop {
            wait_output_capacity(&shared, 0).await?;
            let Some(part) = stream.next().await else {
                return Ok::<(), OpsError>(());
            };
            let part = part.map_err(OpsError::AiMux)?;
            let bytes = part_size(
                &part,
                shared.limits.item_bytes.min(shared.limits.output_bytes),
            )?;
            wait_output_capacity(&shared, bytes).await?;
            {
                let mut state = shared.lock();
                if state.terminal.is_some() {
                    return Ok(());
                }
                state.output_bytes = state
                    .output_bytes
                    .checked_add(bytes)
                    .ok_or_else(|| resource_limit("output byte count overflow"))?;
                state.output_queue.push_back((part, bytes));
            }
            shared.wake();
        }
    };
    tokio::select! {
        biased;
        _ = shared.abort.cancelled() => { shared.cancel(); }
        _ = external_cancel => {}
        result = forward => match result {
            Ok(()) => shared.finish(OperationOutcome::Success, None),
            Err(error) => shared.finish(OperationOutcome::Error, Some(error)),
        }
    }
}

async fn wait_output_capacity(shared: &Shared, bytes: usize) -> Result<(), OpsError> {
    loop {
        let changed = shared.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        {
            let state = shared.lock();
            if state.terminal.is_some() {
                return Err(invalid_state("operation has terminated"));
            }
            let count = state
                .output_bytes
                .checked_add(bytes)
                .ok_or_else(|| resource_limit("output byte count overflow"))?;
            if state.output_queue.len() < shared.limits.output_items
                && count <= shared.limits.output_bytes
                && state.output_bytes < shared.limits.output_bytes
            {
                return Ok(());
            }
        }
        changed.await;
    }
}

fn part_size(part: &TranscriptionStreamPart, maximum: usize) -> Result<usize, OpsError> {
    crate::limits::check_wire_item(part, maximum).map_err(|error| match error {
        OpsError::Protocol(ProtocolError::ResourceLimit(_)) => {
            resource_limit("transcription event exceeds the output budget")
        }
        error => error,
    })
}

async fn join_driver(shared: &Shared, mut task: JoinHandle<()>) {
    match tokio::time::timeout(CLEANUP_TIMEOUT, &mut task).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            shared.record_cleanup_failure(format!("operation driver join failed: {error}"))
        }
        Err(_) => {
            task.abort();
            shared.record_cleanup_failure("operation driver cleanup exceeded 5 seconds".into());
        }
    }
}

fn invalid_state(message: &str) -> OpsError {
    OpsError::Protocol(ProtocolError::InvalidState(message.into()))
}

fn resource_limit(message: &str) -> OpsError {
    OpsError::Protocol(ProtocolError::ResourceLimit(message.into()))
}
