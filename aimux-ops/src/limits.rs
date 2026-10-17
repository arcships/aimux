//! Byte- and item-bounded queues for operation input and output.
//!
//! Reserve capacity before allocating a payload. A reservation counts against
//! both budgets until it is dropped or its payload is consumed. Terminal state
//! has a separate slot and never waits for data capacity.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use aimux_core::AbortSignal;
use tokio::sync::Notify;

use crate::envelope::{OpsError, ProtocolError};

pub const AUDIO_INPUT_ITEMS: usize = 64;
pub const OUTPUT_ITEMS: usize = 256;
pub const QUEUE_BYTES: usize = 8 * 1024 * 1024;
pub const ITEM_BYTES: usize = 1024 * 1024;

/// Finite queue budgets. Construct a default, then explicitly lower it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueLimits {
    max_items: usize,
    max_bytes: usize,
    max_item_bytes: usize,
}

impl QueueLimits {
    #[must_use]
    pub const fn audio_input() -> Self {
        Self {
            max_items: AUDIO_INPUT_ITEMS,
            max_bytes: QUEUE_BYTES,
            max_item_bytes: ITEM_BYTES,
        }
    }

    #[must_use]
    pub const fn output() -> Self {
        Self {
            max_items: OUTPUT_ITEMS,
            ..Self::audio_input()
        }
    }

    /// Lower any budget; zero and increases are rejected.
    ///
    /// # Errors
    /// Returns ResourceLimit for invalid or unbounded budgets.
    pub fn lowered(
        self,
        max_items: usize,
        max_bytes: usize,
        max_item_bytes: usize,
    ) -> Result<Self, OpsError> {
        if max_items == 0
            || max_bytes == 0
            || max_item_bytes == 0
            || max_items > self.max_items
            || max_bytes > self.max_bytes
            || max_item_bytes > self.max_item_bytes
        {
            return Err(resource_limit(
                "queue limits must be positive and may only be lowered",
            ));
        }
        Ok(Self {
            max_items,
            max_bytes,
            max_item_bytes,
        })
    }

    #[must_use]
    pub const fn max_items(self) -> usize {
        self.max_items
    }

    #[must_use]
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }

    #[must_use]
    pub const fn max_item_bytes(self) -> usize {
        self.max_item_bytes
    }

    /// Validate a wire payload length before allocating it.
    ///
    /// # Errors
    /// Returns ResourceLimit when the item cannot fit either byte budget.
    pub fn check_item(self, bytes: usize) -> Result<(), OpsError> {
        if bytes > self.max_item_bytes || bytes > self.max_bytes {
            return Err(resource_limit(
                "payload exceeds the single-item or queue byte limit",
            ));
        }
        Ok(())
    }
}

/// Size of the actual queued payload, including any wire encoding overhead.
/// Implementations must report the full retained payload size.
pub trait QueueItem {
    fn byte_len(&self) -> usize;
}

impl QueueItem for Vec<u8> {
    fn byte_len(&self) -> usize {
        self.capacity()
    }
}

impl QueueItem for Box<[u8]> {
    fn byte_len(&self) -> usize {
        self.len()
    }
}

impl QueueItem for String {
    fn byte_len(&self) -> usize {
        self.capacity()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueStatus {
    Open,
    Finished,
    ConsumerClosed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum QueueEvent<T, E> {
    Data(T),
    Terminal(E),
}

enum Terminal<E> {
    Open,
    Ready(E),
    Delivered,
}

struct State<T, E> {
    data: VecDeque<(T, usize)>,
    used_items: usize,
    used_bytes: usize,
    terminal: Terminal<E>,
    consumer_closed: bool,
}

struct Shared<T, E> {
    limits: QueueLimits,
    state: Mutex<State<T, E>>,
    readable: Notify,
    writable: Notify,
}

impl<T, E> Shared<T, E> {
    fn lock(&self) -> MutexGuard<'_, State<T, E>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Cloneable producer. The operation must explicitly publish its true terminal
/// outcome with `finish`; dropping a producer does not imply success.
pub struct QueueSender<T, E> {
    shared: Arc<Shared<T, E>>,
}

impl<T, E> Clone for QueueSender<T, E> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

/// Single consumer; dropping it explicitly closes the consumer side.
pub struct QueueReceiver<T, E> {
    shared: Arc<Shared<T, E>>,
}

/// Capacity reserved before allocation. Dropping it releases both budgets.
#[must_use = "dropping the permit releases its reserved capacity"]
pub struct QueuePermit<T, E> {
    shared: Arc<Shared<T, E>>,
    bytes: Option<usize>,
}

#[must_use]
pub fn bounded_queue<T: QueueItem, E>(
    limits: QueueLimits,
) -> (QueueSender<T, E>, QueueReceiver<T, E>) {
    let shared = Arc::new(Shared {
        limits,
        state: Mutex::new(State {
            data: VecDeque::new(),
            used_items: 0,
            used_bytes: 0,
            terminal: Terminal::Open,
            consumer_closed: false,
        }),
        readable: Notify::new(),
        writable: Notify::new(),
    });
    (
        QueueSender {
            shared: Arc::clone(&shared),
        },
        QueueReceiver { shared },
    )
}

impl<T: QueueItem, E> QueueSender<T, E> {
    /// Wait for both budgets before the caller allocates the payload.
    ///
    /// # Errors
    /// Returns ResourceLimit for oversized items, Aborted on abort, or
    /// InvalidState after terminal publication or consumer closure.
    pub async fn reserve(
        &self,
        bytes: usize,
        abort: Option<&AbortSignal>,
    ) -> Result<QueuePermit<T, E>, OpsError> {
        self.shared.limits.check_item(bytes)?;
        loop {
            let notified = self.shared.writable.notified();
            tokio::pin!(notified);
            // Register before checking state so release/close cannot be missed.
            notified.as_mut().enable();
            {
                let mut state = self.shared.lock();
                if abort.is_some_and(AbortSignal::is_aborted) {
                    return Err(OpsError::AiMux(aimux_core::error::AiMuxError::Aborted(
                        "queue send aborted".into(),
                    )));
                }
                ensure_open(&state)?;
                if state.used_items < self.shared.limits.max_items
                    && bytes <= self.shared.limits.max_bytes - state.used_bytes
                {
                    state.used_items += 1;
                    state.used_bytes += bytes;
                    return Ok(QueuePermit {
                        shared: Arc::clone(&self.shared),
                        bytes: Some(bytes),
                    });
                }
            }
            if let Some(abort) = abort {
                tokio::select! {
                    biased;
                    () = abort.cancelled() => {
                        return Err(OpsError::AiMux(aimux_core::error::AiMuxError::Aborted(
                            "queue send aborted".into(),
                        )));
                    }
                    () = notified => {}
                }
            } else {
                notified.await;
            }
        }
    }

    /// Send an already allocated payload. Use `reserve` before allocation when
    /// accepting a wire length from an untrusted caller.
    ///
    /// # Errors
    /// Returns the same errors as `reserve`.
    pub async fn send(&self, item: T, abort: Option<&AbortSignal>) -> Result<(), OpsError> {
        self.reserve(item.byte_len(), abort).await?.send(item)
    }

    /// Publish the first terminal outcome without acquiring data capacity.
    /// Already accepted data is delivered before this terminal. Pending
    /// reservations can no longer commit. Later outcomes cannot replace it.
    pub fn finish(&self, terminal: E) -> bool {
        {
            let mut state = self.shared.lock();
            if state.consumer_closed || !matches!(state.terminal, Terminal::Open) {
                return false;
            }
            state.terminal = Terminal::Ready(terminal);
        }
        self.shared.readable.notify_waiters();
        self.shared.writable.notify_waiters();
        true
    }

    /// Admitted and reserved item/byte counts, for backpressure instrumentation.
    #[must_use]
    pub fn usage(&self) -> (usize, usize) {
        let state = self.shared.lock();
        (state.used_items, state.used_bytes)
    }

    #[must_use]
    pub fn status(&self) -> QueueStatus {
        let state = self.shared.lock();
        if state.consumer_closed {
            QueueStatus::ConsumerClosed
        } else if matches!(state.terminal, Terminal::Open) {
            QueueStatus::Open
        } else {
            QueueStatus::Finished
        }
    }
}

impl<T: QueueItem, E> QueuePermit<T, E> {
    /// Commit a payload whose actual size must not exceed its reservation.
    ///
    /// # Errors
    /// Returns ResourceLimit for an undersized reservation, or InvalidState if
    /// terminal publication/consumer closure occurred before this commit.
    pub fn send(mut self, item: T) -> Result<(), OpsError> {
        let actual = item.byte_len();
        let reserved = self.bytes.unwrap_or_default();
        if actual > reserved {
            return Err(resource_limit("payload exceeds its reserved byte budget"));
        }
        {
            let mut state = self.shared.lock();
            ensure_open(&state)?;
            state.used_bytes -= reserved - actual;
            state.data.push_back((item, actual));
            self.bytes = None;
        }
        self.shared.readable.notify_one();
        self.shared.writable.notify_waiters();
        Ok(())
    }
}

impl<T, E> Drop for QueuePermit<T, E> {
    fn drop(&mut self) {
        if let Some(bytes) = self.bytes.take() {
            {
                let mut state = self.shared.lock();
                state.used_items -= 1;
                state.used_bytes -= bytes;
            }
            self.shared.writable.notify_waiters();
        }
    }
}

impl<T, E> QueueReceiver<T, E> {
    /// Drain accepted data, then deliver the independent terminal exactly once.
    /// Returns None after terminal delivery or consumer closure.
    pub async fn recv(&mut self) -> Option<QueueEvent<T, E>> {
        loop {
            let notified = self.shared.readable.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut state = self.shared.lock();
                if state.consumer_closed {
                    return None;
                }
                if let Some((item, bytes)) = state.data.pop_front() {
                    state.used_items -= 1;
                    state.used_bytes -= bytes;
                    drop(state);
                    self.shared.writable.notify_waiters();
                    return Some(QueueEvent::Data(item));
                }
                match std::mem::replace(&mut state.terminal, Terminal::Delivered) {
                    Terminal::Ready(terminal) => return Some(QueueEvent::Terminal(terminal)),
                    Terminal::Delivered => return None,
                    Terminal::Open => state.terminal = Terminal::Open,
                }
            }
            notified.await;
        }
    }

    /// Explicitly abandon queued data and wake blocked producers. The caller
    /// must abort the associated operation and report cancelled/consumerClosed.
    pub fn close(&mut self) {
        let abandoned = {
            let mut state = self.shared.lock();
            if state.consumer_closed || matches!(state.terminal, Terminal::Delivered) {
                return;
            }
            state.consumer_closed = true;
            let abandoned = std::mem::take(&mut state.data);
            state.used_items -= abandoned.len();
            state.used_bytes -= abandoned.iter().map(|(_, bytes)| bytes).sum::<usize>();
            abandoned
        };
        // Payload destructors run outside the mutex.
        drop(abandoned);
        self.shared.writable.notify_waiters();
        self.shared.readable.notify_waiters();
    }
}

impl<T, E> Drop for QueueReceiver<T, E> {
    fn drop(&mut self) {
        self.close();
    }
}

fn resource_limit(message: &str) -> OpsError {
    OpsError::Protocol(ProtocolError::ResourceLimit(message.into()))
}

fn ensure_open<T, E>(state: &State<T, E>) -> Result<(), OpsError> {
    if state.consumer_closed {
        Err(OpsError::Protocol(ProtocolError::InvalidState(
            "consumerClosed".into(),
        )))
    } else if !matches!(state.terminal, Terminal::Open) {
        Err(OpsError::Protocol(ProtocolError::InvalidState(
            "queue already terminal".into(),
        )))
    } else {
        Ok(())
    }
}

/// Measure serialized wire bytes without allocating an output buffer.
/// # Errors
/// Returns ResourceLimit above the item bound or ResultSerialization for a serializer error.
pub fn check_wire_item(value: &impl serde::Serialize, maximum: usize) -> Result<usize, OpsError> {
    struct Counter {
        bytes: usize,
        maximum: usize,
        exceeded: bool,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            match self.bytes.checked_add(bytes.len()) {
                Some(total) if total <= self.maximum => {
                    self.bytes = total;
                    Ok(bytes.len())
                }
                _ => {
                    self.exceeded = true;
                    Err(std::io::Error::other("wire item exceeds byte budget"))
                }
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter {
        bytes: 0,
        maximum,
        exceeded: false,
    };
    if let Err(error) = serde_json::to_writer(&mut counter, value) {
        return if counter.exceeded {
            Err(resource_limit("wire item exceeds byte budget"))
        } else {
            Err(crate::envelope::BindingError::ResultSerialization(error.to_string()).into())
        };
    }
    Ok(counter.bytes)
}
