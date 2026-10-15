//! Lossless bridge: 64 data frames / 8 MiB, with terminal information outside the queue.
use std::future::Future;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use aimux_core::AbortSignal;
use aimux_ops::envelope::{ErrorEnvelope, OpsError, ProtocolError};
use aimux_ops::frame::{Outcome, StreamFrame};
use futures::{Stream, StreamExt};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use tokio::sync::{Semaphore, mpsc};

use crate::error::{WireResult, envelope, serialize};
use crate::{Request, invalid_state};

const ITEMS: usize = 64;
const BYTES: usize = 8 * 1024 * 1024;
const SINGLE: usize = 1024 * 1024;

type Terminal = Arc<Mutex<Option<std::result::Result<String, ErrorEnvelope>>>>;

struct Queued {
    json: String,
    _bytes: tokio::sync::OwnedSemaphorePermit,
}

struct Bridge {
    receiver: tokio::sync::Mutex<mpsc::Receiver<Queued>>,
    terminal: Terminal,
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
        if let Ok(mut receiver) = self.bridge.receiver.try_lock() {
            receiver.close();
            while receiver.try_recv().is_ok() {}
        }
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
            if bridge.closed.swap(true, Ordering::AcqRel) {
                return Ok(None);
            }
            let terminal = bridge
                .terminal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .unwrap_or_else(|| Err(invalid_state("stream ended without a terminal frame")));
            Ok(Some(WireResult(terminal)))
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

struct Size(usize);
impl Write for Size {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .filter(|size| *size <= SINGLE)
            .ok_or_else(|| io::Error::other("stream frame exceeds 1 MiB"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn frame_size(frame: &StreamFrame) -> std::result::Result<usize, ErrorEnvelope> {
    // Size before allocating the serialized buffer (escaping can expand text).
    let mut size = Size(0);
    serde_json::to_writer(&mut size, frame).map_err(|_| {
        envelope(OpsError::Protocol(ProtocolError::ResourceLimit(
            "stream frame exceeds 1 MiB".into(),
        )))
    })?;
    Ok(size.0)
}

fn end(
    seq: String,
    outcome: Outcome,
    error: Option<ErrorEnvelope>,
) -> std::result::Result<String, ErrorEnvelope> {
    serialize(&StreamFrame::End {
        seq,
        outcome,
        error,
    })
}

pub(super) fn start<S>(stream: S, request: Request) -> FrameIterator
where
    S: Stream<Item = StreamFrame> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel(ITEMS);
    let terminal = Arc::new(Mutex::new(None));
    let bridge = Arc::new(Bridge {
        receiver: tokio::sync::Mutex::new(receiver),
        terminal: terminal.clone(),
        signal: request.signal.clone(),
        reading: AtomicBool::new(false),
        closed: AtomicBool::new(false),
    });
    // napi async methods enter the documented napi Tokio runtime before calling start.
    tokio::spawn(produce(stream, sender, terminal, request));
    FrameIterator { bridge }
}

async fn produce<S>(stream: S, sender: mpsc::Sender<Queued>, terminal: Terminal, request: Request)
where
    S: Stream<Item = StreamFrame> + Send + 'static,
{
    futures::pin_mut!(stream);
    let budget = Arc::new(Semaphore::new(BYTES));
    let signal = &request.signal;
    let mut next_seq = "0".to_owned();
    let result = loop {
        // Reserve an item slot before polling upstream: a full bridge pulls no extra event.
        let slot = tokio::select! {
            biased;
            slot = sender.reserve() => match slot {
                Ok(slot) => slot,
                Err(_) => { signal.abort(); break end(next_seq, Outcome::Cancelled, None); }
            },
            _ = signal.cancelled() => break end(next_seq, Outcome::Cancelled, None),
        };
        let frame = tokio::select! {
            biased;
            frame = stream.next() => match frame {
                Some(frame) => frame,
                None => break Err(invalid_state("dispatch stream ended without a terminal frame")),
            },
            _ = signal.cancelled() => break end(next_seq, Outcome::Cancelled, None),
            _ = sender.closed() => { signal.abort(); break end(next_seq, Outcome::Cancelled, None); }
        };
        let seq = match &frame {
            StreamFrame::Event { seq, .. } | StreamFrame::End { seq, .. } => seq.clone(),
        };
        let size = match frame_size(&frame) {
            Ok(size) => size,
            Err(error) => {
                signal.abort();
                break end(seq, Outcome::Error, Some(error));
            }
        };
        if matches!(frame, StreamFrame::End { .. }) {
            break serialize(&frame);
        }
        let permit = tokio::select! {
            biased;
            _ = signal.cancelled() => break end(seq, Outcome::Cancelled, None),
            permit = budget.clone().acquire_many_owned(size as u32) => match permit {
                Ok(permit) => permit,
                Err(_) => break Err(invalid_state("stream byte budget is closed")),
            }
        };
        let json = match serialize(&frame) {
            Ok(json) => json,
            Err(error) => {
                signal.abort();
                break end(seq, Outcome::Error, Some(error));
            }
        };
        if signal.is_aborted() {
            break end(seq, Outcome::Cancelled, None);
        }
        slot.send(Queued {
            json,
            _bytes: permit,
        });
        next_seq = match seq.parse::<u64>().ok().and_then(|seq| seq.checked_add(1)) {
            Some(seq) => seq.to_string(),
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

    #[test]
    fn escaped_bytes_are_limited_before_serialization() {
        let frame = StreamFrame::Event {
            seq: "1".into(),
            event: json!("\u{0000}".repeat(SINGLE / 6)),
        };
        assert!(frame_size(&frame).is_err());
        assert!(
            frame_size(&StreamFrame::Event {
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
        let request = request();
        let signal = request.signal.clone();
        let mut iterator = start(futures::stream::iter([frame]), request);
        let item = iterator.next(None).await.expect("next").expect("terminal");
        let text = match item.0 {
            Ok(text) => text,
            Err(_) => panic!("expected end"),
        };
        let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(value["type"], "end");
        assert_eq!(value["outcome"], "error");
        assert_eq!(value["error"]["code"], 305);
        assert!(signal.is_aborted());
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
