use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::SystemTime;

use aimux_core::{AbortSignal, AiMuxError};
use futures::Stream;
use serde::Serialize;

use crate::dispatch::EventStream;
use crate::envelope::{ErrorEnvelope, OpsError, ProtocolError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Success,
    Error,
    Cancelled,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StreamFrame {
    Event {
        seq: String,
        event: serde_json::Value,
    },
    End {
        seq: String,
        outcome: Outcome,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<ErrorEnvelope>,
    },
}

/// Pull-driven framing: at most one upstream item is read for each output item.
/// The terminal transition drops upstream immediately and is never rewritten.
pub struct FrameStream {
    events: Option<EventStream>,
    cancelled: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    abort: Option<AbortSignal>,
    now: fn() -> SystemTime,
    seq: u64,
}

impl FrameStream {
    #[must_use]
    pub fn new(events: EventStream, abort: Option<AbortSignal>, now: fn() -> SystemTime) -> Self {
        Self {
            events: Some(events),
            cancelled: abort.as_ref().map(|abort| Box::pin(abort.cancelled()) as _),
            abort,
            now,
            seq: 0,
        }
    }

    fn end(&mut self, outcome: Outcome, error: Option<OpsError>) -> Poll<Option<StreamFrame>> {
        self.events = None;
        self.cancelled = None;
        Poll::Ready(Some(StreamFrame::End {
            seq: self.seq.to_string(),
            outcome,
            error: error.map(|error| {
                error
                    .project_with_clock(self.now)
                    .envelope(SystemTime::UNIX_EPOCH)
            }),
        }))
    }
}

impl Drop for FrameStream {
    fn drop(&mut self) {
        if let (Some(_), Some(abort)) = (&self.events, &self.abort) {
            abort.abort();
        }
    }
}

impl Stream for FrameStream {
    type Item = StreamFrame;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this
            .cancelled
            .as_mut()
            .is_some_and(|cancelled| cancelled.as_mut().poll(cx).is_ready())
        {
            return this.end(
                Outcome::Cancelled,
                Some(OpsError::AiMux(AiMuxError::Aborted(
                    "request aborted".into(),
                ))),
            );
        }
        let next = match this.events.as_mut() {
            Some(events) => events.as_mut().poll_next(cx),
            None => return Poll::Ready(None),
        };
        match next {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => this.end(Outcome::Success, None),
            Poll::Ready(Some(Err(error))) => {
                let outcome =
                    if matches!(error.unprojected(), OpsError::AiMux(AiMuxError::Aborted(_))) {
                        Outcome::Cancelled
                    } else {
                        Outcome::Error
                    };
                this.end(outcome, Some(error))
            }
            Poll::Ready(Some(Ok(event))) => {
                let Some(next_seq) = this.seq.checked_add(1) else {
                    return this.end(
                        Outcome::Error,
                        Some(OpsError::Protocol(ProtocolError::ResourceLimit(
                            "stream sequence exhausted".into(),
                        ))),
                    );
                };
                let frame = StreamFrame::Event {
                    seq: this.seq.to_string(),
                    event,
                };
                if let Err(error) =
                    crate::limits::check_wire_item(&frame, crate::limits::ITEM_BYTES)
                {
                    return this.end(Outcome::Error, Some(error));
                }
                this.seq = next_seq;
                Poll::Ready(Some(frame))
            }
        }
    }
}

impl futures::stream::FusedStream for FrameStream {
    fn is_terminated(&self) -> bool {
        self.events.is_none()
    }
}
