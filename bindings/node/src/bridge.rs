//! Lossless bridge: 64 admitted data frames / 8 MiB across Rust and the unread JS batch.
//! The producer reserves an item before polling upstream. It may additionally
//! hold up to 1 MiB of encoded JSON while waiting for byte admission. Terminal
//! information has its own slot.
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use aimux_core::AbortSignal;
use aimux_ops::envelope::{ErrorEnvelope, OpsError, ProtocolError};
#[cfg(test)]
use aimux_ops::frame::StreamFrame;
use aimux_ops::frame::{Outcome, WireFrame};
use futures::{Stream, StreamExt};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

use crate::error::{WireResult, envelope};
use crate::{Request, invalid_state};

const ITEMS: usize = 64;
const BYTES: usize = 8 * 1024 * 1024;

type Terminal = Arc<Mutex<Option<std::result::Result<String, ErrorEnvelope>>>>;

struct Admission {
    _item: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
}

struct Queued {
    json: String,
    admission: Admission,
}

struct Bridge {
    receiver: tokio::sync::Mutex<mpsc::UnboundedReceiver<Queued>>,
    terminal: Terminal,
    host_batch: Mutex<Vec<Admission>>,
    signal: Arc<AbortSignal>,
    reading: AtomicBool,
    closed: AtomicBool,
}

#[napi(async_iterator)]
pub struct FrameIterator {
    bridge: Arc<Bridge>,
}

impl Drop for FrameIterator {
    fn drop(&mut self) {
        self.close();
    }
}

impl FrameIterator {
    fn close(&self) {
        self.bridge.closed.store(true, Ordering::Release);
        self.bridge.signal.abort();
        self.bridge.host_batch().clear();
        if let Ok(mut receiver) = self.bridge.receiver.try_lock() {
            receiver.close();
            while receiver.try_recv().is_ok() {}
        }
    }
}

impl Bridge {
    fn host_batch(&self) -> std::sync::MutexGuard<'_, Vec<Admission>> {
        self.host_batch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn terminal(&self) -> Option<std::result::Result<String, ErrorEnvelope>> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(
            self.terminal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .unwrap_or_else(|| Err(invalid_state("stream ended without a terminal frame"))),
        )
    }
}

#[napi]
impl FrameIterator {
    /// Drain ready data after waiting for at most the first frame.
    #[napi(ts_return_type = "Promise<string[]>")]
    pub async fn next_batch(&self) -> WireResult<Vec<String>> {
        let bridge = &self.bridge;
        if bridge.reading.swap(true, Ordering::AcqRel) {
            return WireResult(Err(envelope(OpsError::Protocol(
                ProtocolError::ConcurrentConsumer,
            ))));
        }
        let _reading = Reading(bridge.clone());
        // The facade asks again only after draining the previous batch.
        bridge.host_batch().clear();
        if bridge.closed.load(Ordering::Acquire) {
            return WireResult(Ok(Vec::new()));
        }

        let mut receiver = bridge.receiver.lock().await;
        let Some(first) = receiver.recv().await else {
            return WireResult(bridge.terminal().map_or_else(
                || Ok(Vec::new()),
                |terminal| terminal.map(|frame| vec![frame]),
            ));
        };
        let mut permits = bridge.host_batch();
        if bridge.closed.load(Ordering::Acquire) {
            return WireResult(Ok(Vec::new()));
        }
        // Keep both reservations while the batch is buffered in JS.
        permits.push(first.admission);
        let mut frames = vec![first.json];
        while frames.len() < ITEMS {
            let Ok(queued) = receiver.try_recv() else {
                break;
            };
            permits.push(queued.admission);
            frames.push(queued.json);
        }
        WireResult(Ok(frames))
    }
}

struct Reading(Arc<Bridge>);
impl Drop for Reading {
    fn drop(&mut self) {
        self.0.reading.store(false, Ordering::Release);
    }
}

#[napi]
impl AsyncGenerator for FrameIterator {
    type Yield = WireResult<String>;
    type Next = ();
    type Return = ();

    fn next(
        &mut self,
        _: Option<()>,
    ) -> impl Future<Output = napi::Result<Option<Self::Yield>>> + Send + 'static {
        let bridge = self.bridge.clone();
        let busy = bridge.reading.swap(true, Ordering::AcqRel);
        let reading = if busy {
            None
        } else {
            Some(Reading(bridge.clone()))
        };
        async move {
            let _reading = reading;
            if busy {
                return Ok(Some(WireResult(Err(envelope(OpsError::Protocol(
                    ProtocolError::ConcurrentConsumer,
                ))))));
            }
            if bridge.closed.load(Ordering::Acquire) {
                return Ok(None);
            }
            let mut receiver = bridge.receiver.lock().await;
            if let Some(queued) = receiver.recv().await {
                if bridge.closed.load(Ordering::Acquire) {
                    return Ok(None);
                }
                return Ok(Some(WireResult(Ok(queued.json))));
            }
            Ok(bridge.terminal().map(WireResult))
        }
    }

    fn complete(
        &mut self,
        _: Option<()>,
    ) -> impl Future<Output = napi::Result<Option<Self::Yield>>> + Send + 'static {
        // Signal synchronously, before the future runs or any receiver lock is acquired.
        self.close();
        async { Ok(None) }
    }

    fn catch(
        &mut self,
        _env: Env,
        value: Unknown,
    ) -> impl Future<Output = napi::Result<Option<Self::Yield>>> + Send + 'static {
        self.close();
        let error = napi::Error::from_unknown_without_coercion(value);
        async move { Err(error) }
    }
}

fn end(
    seq: String,
    outcome: Outcome,
    error: Option<ErrorEnvelope>,
) -> std::result::Result<String, ErrorEnvelope> {
    WireFrame::encode_end(seq, outcome, error).map_err(envelope)
}

pub(super) fn start<S>(stream: S, request: Request) -> FrameIterator
where
    S: Stream<Item = WireFrame> + Send + 'static,
{
    let (sender, receiver) = mpsc::unbounded_channel();
    let terminal = Arc::new(Mutex::new(None));
    let bridge = Arc::new(Bridge {
        receiver: tokio::sync::Mutex::new(receiver),
        terminal: terminal.clone(),
        host_batch: Mutex::new(Vec::new()),
        signal: request.signal.clone(),
        reading: AtomicBool::new(false),
        closed: AtomicBool::new(false),
    });
    // napi async methods enter the documented napi Tokio runtime before calling start.
    tokio::spawn(produce(stream, sender, terminal, request));
    FrameIterator { bridge }
}

async fn produce<S>(
    stream: S,
    sender: mpsc::UnboundedSender<Queued>,
    terminal: Terminal,
    request: Request,
) where
    S: Stream<Item = WireFrame> + Send + 'static,
{
    futures::pin_mut!(stream);
    // Only this producer sends frames, and every queued frame owns a permit.
    // The semaphore covers the channel and JS batch, including after dequeue.
    let items = Arc::new(Semaphore::new(ITEMS));
    let bytes = Arc::new(Semaphore::new(BYTES));
    let signal = &request.signal;
    let mut next_seq = 0_u64;
    let result = loop {
        // Reserve an item slot before polling upstream: a full bridge pulls no extra event.
        let slot = tokio::select! {
            biased;
            _ = sender.closed() => { signal.abort(); break end(next_seq.to_string(), Outcome::Cancelled, None); }
            slot = items.clone().acquire_owned() => match slot {
                Ok(slot) => slot,
                Err(_) => break Err(invalid_state("stream item budget is closed")),
            },
            _ = signal.cancelled() => break end(next_seq.to_string(), Outcome::Cancelled, None),
        };
        let frame = tokio::select! {
            biased;
            frame = stream.next() => match frame {
                Some(frame) => frame,
                None => break Err(invalid_state("dispatch stream ended without a terminal frame")),
            },
            _ = signal.cancelled() => break end(next_seq.to_string(), Outcome::Cancelled, None),
            _ = sender.closed() => { signal.abort(); break end(next_seq.to_string(), Outcome::Cancelled, None); }
        };
        let (seq, json) = match frame {
            WireFrame::Event { seq, json } => (seq, json),
            WireFrame::End {
                seq,
                outcome,
                error,
            } => break end(seq, outcome, error),
        };
        let permit = tokio::select! {
            biased;
            _ = signal.cancelled() => break end(seq.to_string(), Outcome::Cancelled, None),
            permit = bytes.clone().acquire_many_owned(json.len() as u32) => match permit {
                Ok(permit) => permit,
                Err(_) => break Err(invalid_state("stream byte budget is closed")),
            }
        };
        if signal.is_aborted() {
            break end(seq.to_string(), Outcome::Cancelled, None);
        }
        let admission = Admission {
            _item: slot,
            _bytes: permit,
        };
        if sender.send(Queued { json, admission }).is_err() {
            signal.abort();
            break end(seq.to_string(), Outcome::Cancelled, None);
        }
        next_seq = match seq.checked_add(1) {
            Some(seq) => seq,
            None => {
                signal.abort();
                break Err(invalid_state("stream sequence exhausted"));
            }
        };
    };
    *terminal
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result);
    // Dropping the sender wakes the consumer after it drains all accepted data.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Realm, Resources};
    use aimux_ops::handle::Owner;
    use serde_json::json;
    use std::sync::atomic::AtomicU64;

    fn request() -> Request {
        let realm = Arc::new(Realm {
            owner: Owner::new_node().expect("test owner"),
            next_request: AtomicU64::new(1),
            resources: Mutex::new(Resources::default()),
        });
        match realm.request(Arc::new(AbortSignal::new())) {
            Ok(request) => request,
            Err(_) => panic!("request setup failed"),
        }
    }

    const SINGLE: usize = aimux_ops::limits::ITEM_BYTES;

    fn start(
        frames: impl Stream<Item = StreamFrame> + Send + 'static,
        request: Request,
    ) -> FrameIterator {
        let frames = frames.map(|frame| {
            let seq = match &frame {
                StreamFrame::Event { seq, .. } | StreamFrame::End { seq, .. } => seq.clone(),
            };
            WireFrame::encode(frame).unwrap_or_else(|error| WireFrame::End {
                seq,
                outcome: Outcome::Error,
                error: Some(error.envelope()),
            })
        });
        super::start(frames, request)
    }

    #[test]
    fn escaped_bytes_are_limited_before_serialization() {
        let frame = StreamFrame::Event {
            seq: "1".into(),
            event: json!("\u{0000}".repeat(SINGLE / 6)),
        };
        assert!(WireFrame::encode(frame).is_err());
        assert!(
            WireFrame::encode(StreamFrame::Event {
                seq: "1".into(),
                event: json!("ok")
            })
            .is_ok()
        );
    }

    #[tokio::test]
    async fn abort_wakes_full_queue_and_terminal_survives() {
        let request = request();
        let signal = request.signal.clone();
        let polls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = polls.clone();
        let stream = futures::stream::iter((1..=1000).map(|seq| StreamFrame::Event {
            seq: seq.to_string(),
            event: json!(seq),
        }))
        .inspect(move |_| {
            observed.fetch_add(1, Ordering::Relaxed);
        });
        let mut iterator = start(stream, request);
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if iterator.bridge.receiver.lock().await.len() == ITEMS {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("producer fills queue");
        assert_eq!(polls.load(Ordering::Relaxed), ITEMS);
        signal.abort();
        let mut events = 0;
        let mut ends = 0;
        while let Some(item) = iterator.next(None).await.expect("next") {
            let json = match item.0 {
                Ok(json) => json,
                Err(_) => panic!("expected a JSON frame"),
            };
            let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
            if value["type"] == "end" {
                ends += 1;
                assert_eq!(value["outcome"], "cancelled");
            } else {
                events += 1;
            }
        }
        assert_eq!(events, ITEMS);
        assert_eq!(ends, 1);
    }

    #[tokio::test]
    async fn return_aborts_without_waiting_for_a_reader() {
        let request = request();
        let signal = request.signal.clone();
        let mut iterator = start(futures::stream::pending::<StreamFrame>(), request);
        let pending = iterator.next(None);
        iterator.complete(None).await.expect("return");
        assert!(signal.is_aborted());
        assert!(pending.await.expect("pending next").is_none());
    }

    #[tokio::test]
    async fn byte_budget_bounds_large_items() {
        let request = request();
        let signal = request.signal.clone();
        let stream = futures::stream::iter((1..100).map(|seq| StreamFrame::Event {
            seq: seq.to_string(),
            event: json!("a".repeat(SINGLE / 2)),
        }));
        let iterator = start(stream, request);
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        let count = iterator.bridge.receiver.lock().await.len();
        assert!(
            count <= 15,
            "byte budget must bind before the 64-item limit: {count}"
        );
        signal.abort();
    }
    #[tokio::test]
    async fn unread_host_batch_holds_the_same_limits_as_native_queue() {
        // A 16th half-MiB payload would exceed the byte budget with JSON overhead.
        for (payload_bytes, capacity) in [(1, ITEMS), (SINGLE / 2, 15)] {
            let end_seq = capacity * 2 + 1;
            let events = futures::stream::iter((1..end_seq).map(move |seq| StreamFrame::Event {
                seq: seq.to_string(),
                event: json!("a".repeat(payload_bytes)),
            }));
            let terminal = futures::stream::iter([StreamFrame::End {
                seq: end_seq.to_string(),
                outcome: Outcome::Success,
                error: None,
            }]);
            let iterator = start(events.chain(terminal), request());
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while iterator.bridge.receiver.lock().await.len() != capacity {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("producer fills the budget");

            let first = iterator.next_batch().await.0.ok().expect("batch");
            assert_eq!(first.len(), capacity);
            tokio::task::yield_now().await;
            assert_eq!(iterator.bridge.receiver.lock().await.len(), 0);
            drop(first);

            let mut expected_seq = capacity + 1;
            loop {
                let batch = iterator.next_batch().await.0.ok().expect("batch");
                if batch.is_empty() {
                    break;
                }
                for frame in batch {
                    let value: serde_json::Value = serde_json::from_str(&frame).unwrap();
                    assert_eq!(value["seq"], expected_seq.to_string());
                    if expected_seq == end_seq {
                        assert_eq!(value["outcome"], "success");
                    }
                    expected_seq += 1;
                }
            }
            assert_eq!(expected_seq, end_seq + 1);
        }
    }

    #[tokio::test]
    async fn slow_reader_loses_no_events_and_reads_one_terminal() {
        let events = futures::stream::iter((1..=150).map(|seq| StreamFrame::Event {
            seq: seq.to_string(),
            event: json!(seq),
        }));
        let terminal = futures::stream::iter([StreamFrame::End {
            seq: "151".into(),
            outcome: Outcome::Success,
            error: None,
        }]);
        let mut iterator = start(events.chain(terminal), request());
        for seq in 1..=151 {
            let item = iterator.next(None).await.expect("next").expect("frame");
            let text = match item.0 {
                Ok(text) => text,
                Err(_) => panic!("expected JSON"),
            };
            let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
            assert_eq!(value["seq"], seq.to_string());
            if seq == 151 {
                assert_eq!(value["outcome"], "success");
            }
            tokio::task::yield_now().await;
        }
        assert!(iterator.next(None).await.expect("exhausted").is_none());
    }

    #[tokio::test]
    async fn concurrent_next_does_not_cancel_the_first_reader() {
        let request = request();
        let signal = request.signal.clone();
        let mut iterator = start(futures::stream::pending::<StreamFrame>(), request);
        let first = iterator.next(None);
        let second = iterator
            .next(None)
            .await
            .expect("second next")
            .expect("error item");
        assert!(matches!(second.0, Err(ref error) if error.code == 303));
        assert!(!signal.is_aborted());
        iterator.complete(None).await.expect("return");
        assert!(first.await.expect("first next").is_none());
    }

    #[tokio::test]
    async fn missing_terminal_is_not_success() {
        let mut iterator = start(futures::stream::empty::<StreamFrame>(), request());
        let item = iterator.next(None).await.expect("next").expect("failure");
        assert!(matches!(item.0, Err(ref error) if error.code == 304));
        assert!(iterator.next(None).await.expect("exhausted").is_none());
    }

    #[tokio::test]
    async fn oversized_event_delivers_resource_limit_in_terminal_slot() {
        let frame = StreamFrame::Event {
            seq: "1".into(),
            event: json!("x".repeat(SINGLE)),
        };
        let mut iterator = start(futures::stream::iter([frame]), request());
        let item = iterator.next(None).await.expect("next").expect("terminal");
        let text = match item.0 {
            Ok(text) => text,
            Err(_) => panic!("expected end"),
        };
        let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(value["type"], "end");
        assert_eq!(value["outcome"], "error");
        assert_eq!(value["error"]["code"], 305);
        assert!(iterator.next(None).await.expect("exhausted").is_none());
    }

    #[tokio::test]
    async fn environment_cleanup_cancels_a_never_consumed_stream() {
        let request = request();
        let realm = request.realm.clone();
        let signal = request.signal.clone();
        let _iterator = start(futures::stream::pending::<StreamFrame>(), request);
        realm.cleanup();
        assert!(signal.is_aborted());
        assert!(realm.request(Arc::new(AbortSignal::new())).is_err());
    }
    #[tokio::test]
    async fn cancellation_after_terminal_does_not_rewrite_success() {
        let request = request();
        let signal = request.signal.clone();
        let mut iterator = start(
            futures::stream::iter([StreamFrame::End {
                seq: "1".into(),
                outcome: Outcome::Success,
                error: None,
            }]),
            request,
        );
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if iterator.bridge.terminal.lock().unwrap().is_some() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("terminal saved");
        signal.abort();
        let item = iterator.next(None).await.expect("next").expect("end");
        let text = match item.0 {
            Ok(text) => text,
            Err(_) => panic!("expected end"),
        };
        let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(value["outcome"], "success");
    }
}
