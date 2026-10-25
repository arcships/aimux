//! Bounded, cancellation-aware bridge. Worker tasks never acquire the GIL.
use std::future::Future;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, mpsc};
use std::time::{Duration, Instant};

use aimux_core::AbortSignal;
use aimux_ops::envelope::{ErrorEnvelope, OpsError, ProtocolError};
use aimux_ops::frame::{Outcome, StreamFrame};
use aimux_ops::handle::HandleId;
use futures::StreamExt;
use pyo3::prelude::*;
use serde_json::Value;
use tokio::sync::{Semaphore, mpsc as channel};

use crate::{InterpreterOwner, check_owner, context, error, invalid_state, runtime, serialize};

const WAIT_SLICE: Duration = Duration::from_millis(20);
const QUEUE_ITEMS: usize = 64;
const QUEUE_BYTES: usize = 8 * 1024 * 1024;
const MAX_FRAME_BYTES: usize = 1024 * 1024;

type Terminal = Result<(), ErrorEnvelope>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn aborted() -> OpsError {
    OpsError::AiMux(aimux_core::AiMuxError::Aborted(
        "Python operation cancelled".into(),
    ))
}

fn project(error: OpsError) -> ErrorEnvelope {
    error.envelope()
}

#[derive(Default)]
struct Completion {
    done: Mutex<bool>,
    changed: Condvar,
}

impl Completion {
    fn wait_slice(&self) -> bool {
        let done = lock(&self.done);
        if *done {
            return true;
        }
        let (done, _) = self
            .changed
            .wait_timeout(done, WAIT_SLICE)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *done
    }
}

struct CompletionGuard(Arc<Completion>);

impl Drop for CompletionGuard {
    fn drop(&mut self) {
        *lock(&self.0.done) = true;
        self.0.changed.notify_all();
    }
}

struct Task {
    signal: AbortSignal,
    handle: tokio::task::JoinHandle<()>,
    completion: Arc<Completion>,
}

impl Task {
    fn spawn(
        py: Python<'_>,
        signal: AbortSignal,
        future: impl Future<Output = ()> + Send + 'static,
    ) -> PyResult<Self> {
        let rt = py
            .allow_threads(runtime)
            .map_err(|e| error::ops_error(py, e))?;
        let completion = Arc::new(Completion::default());
        let guard = CompletionGuard(completion.clone());
        let handle = rt.spawn(async move {
            let _guard = guard;
            future.await;
        });
        Ok(Self {
            signal,
            handle,
            completion,
        })
    }

    fn cancel(&self) {
        self.signal.abort();
        // Also interrupts a producer waiting for channel or byte capacity.
        self.handle.abort();
    }

    fn join(&self, py: Python<'_>) -> PyResult<()> {
        self.wait_for_completion(py, None)
    }

    fn wait_for_completion(&self, py: Python<'_>, deadline: Option<Instant>) -> PyResult<()> {
        loop {
            let done = py.allow_threads(|| self.completion.wait_slice());
            if let Err(error) = py.check_signals() {
                self.cancel();
                return Err(error);
            }
            if done {
                return Ok(());
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(error::ops_error(
                    py,
                    invalid_state("Python stream cleanup exceeded 5s"),
                ));
            }
        }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        if !*lock(&self.completion.done) {
            self.cancel();
        }
    }
}

fn receive<T: Send>(py: Python<'_>, rx: &Mutex<mpsc::Receiver<T>>, task: &Task) -> PyResult<T> {
    loop {
        let result = py.allow_threads(|| lock(rx).recv_timeout(WAIT_SLICE));
        if let Err(error) = py.check_signals() {
            task.cancel();
            return Err(error);
        }
        match result {
            Ok(value) => return Ok(value),
            Err(mpsc::RecvTimeoutError::Timeout) if task.signal.is_aborted() => {
                task.cancel();
                return Err(error::ops_error(py, aborted()));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(error::ops_error(
                    py,
                    invalid_state("worker ended without a result"),
                ));
            }
        }
    }
}

pub(crate) fn wait_call(
    py: Python<'_>,
    signal: AbortSignal,
    future: impl Future<Output = Result<String, OpsError>> + Send + 'static,
) -> PyResult<String> {
    let (tx, rx) = mpsc::sync_channel(1);
    let cancellation = signal.clone();
    let task = Task::spawn(py, signal, async move {
        let result = tokio::select! {
            biased;
            result = future => result,
            _ = cancellation.cancelled() => Err(aborted()),
        };
        let _ = tx.send(result.map_err(project));
    })?;
    let result = receive(py, &Mutex::new(rx), &task)?;
    task.join(py)?;
    result.map_err(|e| error::envelope_error(py, &e))
}

struct Delivery {
    json: String,
    // Queue bytes remain reserved until Python removes the event.
    _bytes: tokio::sync::OwnedSemaphorePermit,
}

struct Queue {
    receiver: Mutex<channel::Receiver<Delivery>>,
    // Independent of the bounded data queue: terminal delivery cannot block
    // behind data, and EOF cannot conceal a missing terminal frame.
    terminal: Mutex<Option<Terminal>>,
}

async fn pump(
    mut frames: impl futures::Stream<Item = StreamFrame> + Unpin,
    sender: channel::Sender<Delivery>,
    queue: Arc<Queue>,
    abort: AbortSignal,
) {
    let bytes = Arc::new(Semaphore::new(QUEUE_BYTES));
    let terminal = loop {
        let frame = tokio::select! {
            biased;
            _ = abort.cancelled() => break Err(project(aborted())),
            _ = sender.closed() => break Err(project(aborted())),
            frame = frames.next() => frame,
        };
        match frame {
            Some(frame @ StreamFrame::Event { .. }) => {
                let json = match serialize(&frame) {
                    Ok(json) => json,
                    Err(error) => break Err(project(error)),
                };
                if json.len() > MAX_FRAME_BYTES {
                    break Err(project(OpsError::Protocol(ProtocolError::ResourceLimit(
                        "Python stream event exceeds 1 MiB".into(),
                    ))));
                }
                let permit = tokio::select! {
                    biased;
                    _ = abort.cancelled() => break Err(project(aborted())),
                    permit = bytes.clone().acquire_many_owned(json.len() as u32) => {
                        match permit {
                            Ok(permit) => permit,
                            Err(_) => break Err(project(invalid_state("byte budget closed"))),
                        }
                    }
                };
                let delivery = Delivery {
                    json,
                    _bytes: permit,
                };
                tokio::select! {
                    biased;
                    _ = abort.cancelled() => break Err(project(aborted())),
                    result = sender.send(delivery) => {
                        if result.is_err() { break Err(project(aborted())); }
                    }
                }
            }
            Some(StreamFrame::End { outcome, error, .. }) => {
                break match (outcome, error) {
                    (Outcome::Success, None) => Ok(()),
                    (Outcome::Error | Outcome::Cancelled, Some(error)) => Err(error),
                    (Outcome::Cancelled, None) => Err(project(aborted())),
                    _ => Err(project(invalid_state("invalid stream terminal frame"))),
                };
            }
            None => break Err(project(invalid_state("stream closed without an End frame"))),
        }
    };
    *lock(&queue.terminal) = Some(terminal);
    // sender drops only AFTER publishing terminal.
}

#[pyclass(module = "aimux.aimux")]
pub(crate) struct StreamIterator {
    owner: Arc<InterpreterOwner>,
    queue: Arc<Queue>,
    task: Task,
    consuming: std::sync::atomic::AtomicBool,
}

struct ConsumerGuard<'a>(&'a std::sync::atomic::AtomicBool);
impl Drop for ConsumerGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

impl StreamIterator {
    pub(crate) fn start(
        py: Python<'_>,
        owner: Arc<InterpreterOwner>,
        handle: HandleId,
        op: String,
        params: Value,
        signal: AbortSignal,
    ) -> PyResult<Self> {
        let (sender, receiver) = channel::channel(QUEUE_ITEMS);
        let queue = Arc::new(Queue {
            receiver: Mutex::new(receiver),
            terminal: Mutex::new(None),
        });
        let producer_queue = queue.clone();
        let abort = signal.clone();
        let (accepted_tx, accepted_rx) = mpsc::sync_channel(1);
        let worker_owner = owner.clone();
        let task = Task::spawn(py, signal, async move {
            let owner = worker_owner;
            match aimux_ops::dispatch::stream(
                context(owner.token, abort.clone()),
                handle,
                &op,
                params,
            ) {
                Ok(frames) => {
                    if accepted_tx.send(Ok(())).is_ok() {
                        pump(frames, sender, producer_queue, abort).await;
                    }
                }
                Err(error) => {
                    let _ = accepted_tx.send(Err(project(error)));
                }
            }
        })?;
        receive(py, &Mutex::new(accepted_rx), &task)?.map_err(|e| error::envelope_error(py, &e))?;
        Ok(Self {
            owner,
            queue,
            task,
            consuming: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn check_cancelled(&self, py: Python<'_>) -> PyResult<()> {
        if self.task.signal.is_aborted() {
            // A settled terminal is immutable, even when close/cancel is late.
            // Live cancellation may discard queued events immediately.
            let terminal = lock(&self.queue.terminal).clone();
            return match terminal {
                Some(Ok(())) => Ok(()),
                Some(Err(error)) => Err(error::envelope_error(py, &error)),
                None => Err(error::ops_error(py, aborted())),
            };
        }
        Ok(())
    }
}

#[pymethods]
impl StreamIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self, py: Python<'_>) -> PyResult<Option<String>> {
        check_owner(py, self.owner.token)?;
        if self
            .consuming
            .swap(true, std::sync::atomic::Ordering::Acquire)
        {
            return Err(error::ops_error(
                py,
                OpsError::Protocol(ProtocolError::ConcurrentConsumer),
            ));
        }
        let _consumer = ConsumerGuard(&self.consuming);
        let rt = py
            .allow_threads(runtime)
            .map_err(|e| error::ops_error(py, e))?;
        loop {
            self.check_cancelled(py)?;
            let result = py.allow_threads(|| {
                let mut receiver = lock(&self.queue.receiver);
                rt.block_on(async { tokio::time::timeout(WAIT_SLICE, receiver.recv()).await })
            });
            if let Err(error) = py.check_signals() {
                self.task.cancel();
                return Err(error);
            }
            // Live cancellation wins over buffered data.
            self.check_cancelled(py)?;
            match result {
                Ok(Some(delivery)) => return Ok(Some(delivery.json)),
                Ok(None) => {
                    let terminal = lock(&self.queue.terminal).clone();
                    return match terminal {
                        Some(Ok(())) => Ok(None),
                        Some(Err(error)) => Err(error::envelope_error(py, &error)),
                        None => Err(error::ops_error(
                            py,
                            invalid_state("worker lost stream terminal"),
                        )),
                    };
                }
                Err(_) => {}
            }
        }
    }

    fn cancel(&self, py: Python<'_>) -> PyResult<()> {
        check_owner(py, self.owner.token)?;
        self.task.cancel();
        Ok(())
    }

    fn join(&self, py: Python<'_>) -> PyResult<()> {
        check_owner(py, self.owner.token)?;
        self.task.join(py)
    }

    fn close(&self, py: Python<'_>) -> PyResult<()> {
        self.cancel(py)?;
        self.task
            .wait_for_completion(py, Some(Instant::now() + Duration::from_secs(5)))?;
        py.allow_threads(|| {
            let mut receiver = lock(&self.queue.receiver);
            receiver.close();
            while receiver.try_recv().is_ok() {}
        });
        Ok(())
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    #[pyo3(signature = (_ty=None, _value=None, _traceback=None))]
    fn __exit__(
        &self,
        py: Python<'_>,
        _ty: Option<&Bound<'_, PyAny>>,
        _value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        self.close(py)
    }
}
