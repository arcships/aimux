use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use aimux_core::{AbortSignal, AiMuxError};
use futures::{Stream, StreamExt};
use serde::Serialize;

use crate::dispatch::EventStream;
use crate::envelope::{BindingError, ErrorEnvelope, OpsError, ProtocolError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(test, derive(serde::Deserialize))]
pub enum Outcome {
    Success,
    Error,
    Cancelled,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
#[cfg_attr(test, derive(serde::Deserialize))]
pub enum StreamFrame {
    Event {
        #[schemars(extend("x-aimux-codec" = "u64-string"))]
        seq: String,
        #[schemars(with = "StreamEvent")]
        event: serde_json::Value,
    },
    End {
        #[schemars(extend("x-aimux-codec" = "u64-string"))]
        seq: String,
        outcome: Outcome,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<ErrorEnvelope>,
    },
}

/// The two event formats carried by an operation stream.
#[derive(Serialize, schemars::JsonSchema)]
#[serde(untagged)]
#[schemars(untagged, extend("x-aimux-codec" = "ordered-untagged"))]
// Keep native events inline rather than allocating a box for every frame.
#[allow(clippy::large_enum_variant)]
pub enum StreamEvent {
    Text(aimux_core::stream_part::TextStreamPart),
    OpenAi(aimux_core::openai_output::ChatCompletionChunk),
    #[schemars(skip)]
    Json(serde_json::Value),
}

pub type TypedEventStream = futures::stream::BoxStream<'static, Result<StreamEvent, OpsError>>;

/// Pull-driven framing: at most one upstream item is read for each output item.
/// The terminal transition drops upstream immediately and is never rewritten.
pub struct FrameStream {
    events: Option<TypedEventStream>,
    cancelled: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    abort: Option<AbortSignal>,
    seq: u64,
}

impl FrameStream {
    #[must_use]
    pub fn new(events: EventStream, abort: Option<AbortSignal>) -> Self {
        Self::typed(
            Box::pin(events.map(|event| event.map(StreamEvent::Json))),
            abort,
        )
    }

    pub(crate) fn typed(events: TypedEventStream, abort: Option<AbortSignal>) -> Self {
        Self {
            events: Some(events),
            cancelled: abort.as_ref().map(|abort| Box::pin(abort.cancelled()) as _),
            abort,
            seq: 0,
        }
    }

    /// Transfer ownership to a transport that consumes already encoded JSON.
    #[must_use]
    pub fn into_wire(self) -> WireFrameStream {
        WireFrameStream(self)
    }

    fn end(&mut self, outcome: Outcome, error: Option<OpsError>) -> StreamFrame {
        self.events = None;
        self.cancelled = None;
        StreamFrame::End {
            seq: self.seq.to_string(),
            outcome,
            error: error.map(|error| error.envelope()),
        }
    }

    fn poll_with<T>(
        &mut self,
        cx: &mut Context<'_>,
        project: impl FnOnce(u64, StreamEvent) -> Result<T, OpsError>,
        terminal: impl FnOnce(StreamFrame) -> T,
    ) -> Poll<Option<T>> {
        let this = self;
        if this
            .cancelled
            .as_mut()
            .is_some_and(|cancelled| cancelled.as_mut().poll(cx).is_ready())
        {
            return Poll::Ready(Some(terminal(this.end(
                Outcome::Cancelled,
                Some(OpsError::AiMux(AiMuxError::Aborted(
                    "request aborted".into(),
                ))),
            ))));
        }
        let next = match this.events.as_mut() {
            Some(events) => events.as_mut().poll_next(cx),
            None => return Poll::Ready(None),
        };
        match next {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(Some(terminal(this.end(Outcome::Success, None)))),
            Poll::Ready(Some(Err(error))) => {
                let outcome = if matches!(error, OpsError::AiMux(AiMuxError::Aborted(_))) {
                    Outcome::Cancelled
                } else {
                    Outcome::Error
                };
                Poll::Ready(Some(terminal(this.end(outcome, Some(error)))))
            }
            Poll::Ready(Some(Ok(event))) => {
                let Some(next_seq) = this.seq.checked_add(1) else {
                    return Poll::Ready(Some(terminal(this.end(
                        Outcome::Error,
                        Some(OpsError::Protocol(ProtocolError::ResourceLimit(
                            "stream sequence exhausted".into(),
                        ))),
                    ))));
                };
                match project(this.seq, event) {
                    Ok(frame) => {
                        this.seq = next_seq;
                        Poll::Ready(Some(frame))
                    }
                    Err(error) => {
                        Poll::Ready(Some(terminal(this.end(Outcome::Error, Some(error)))))
                    }
                }
            }
        }
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

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.poll_with(
            cx,
            |seq, event| {
                let event = match event {
                    StreamEvent::Json(value) => value,
                    event => serde_json::to_value(event)
                        .map_err(|error| BindingError::result_serialization(&error))?,
                };
                let frame = StreamFrame::Event {
                    seq: seq.to_string(),
                    event,
                };
                crate::limits::check_wire_item(&frame, crate::limits::ITEM_BYTES)?;
                Ok(frame)
            },
            |frame| frame,
        )
    }
}

/// JSON data frames are already bounded and encoded. Terminal metadata stays
/// structured so adapters can either write an End frame or translate it to EOF.
pub enum WireFrame {
    Event {
        seq: u64,
        json: String,
    },
    End {
        seq: String,
        outcome: Outcome,
        error: Option<ErrorEnvelope>,
    },
}

impl WireFrame {
    /// Encode a typed frame for a transport adapter.
    /// # Errors
    /// Returns an error if the escaped data frame exceeds the wire item bound.
    pub fn encode(frame: StreamFrame) -> Result<Self, OpsError> {
        match frame {
            StreamFrame::Event { seq, event } => {
                let seq = seq.parse().map_err(|_| {
                    OpsError::Protocol(ProtocolError::InvalidState(
                        "invalid stream sequence".into(),
                    ))
                })?;
                Self::event(seq, StreamEvent::Json(event))
            }
            StreamFrame::End {
                seq,
                outcome,
                error,
            } => Ok(Self::End {
                seq,
                outcome,
                error,
            }),
        }
    }

    /// Encode an End frame, replacing an oversized error payload with ResourceLimit.
    /// # Errors
    /// Returns a serialization error if the replacement terminal cannot be written.
    pub fn encode_end(
        seq: String,
        outcome: Outcome,
        error: Option<ErrorEnvelope>,
    ) -> Result<String, OpsError> {
        use crate::limits::{ITEM_BYTES, encode_wire_item};
        let frame = StreamFrame::End {
            seq,
            outcome,
            error,
        };
        match encode_wire_item(&frame, ITEM_BYTES) {
            Ok(json) => Ok(json),
            Err(error) => {
                let StreamFrame::End { seq, .. } = frame else {
                    unreachable!()
                };
                encode_wire_item(
                    &StreamFrame::End {
                        seq,
                        outcome: Outcome::Error,
                        error: Some(error.envelope()),
                    },
                    ITEM_BYTES,
                )
            }
        }
    }

    fn event(seq: u64, event: StreamEvent) -> Result<Self, OpsError> {
        #[derive(Serialize)]
        struct WireEvent {
            #[serde(rename = "type")]
            kind: &'static str,
            #[serde(serialize_with = "serialize_seq")]
            seq: u64,
            event: StreamEvent,
        }
        fn serialize_seq<S: serde::Serializer>(
            seq: &u64,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            serializer.collect_str(seq)
        }
        let json = crate::limits::encode_wire_item(
            &WireEvent {
                kind: "event",
                seq,
                event,
            },
            crate::limits::ITEM_BYTES,
        )?;
        Ok(Self::Event { seq, json })
    }
}

pub struct WireFrameStream(FrameStream);

impl Stream for WireFrameStream {
    type Item = WireFrame;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_with(cx, WireFrame::event, |frame| {
            let StreamFrame::End {
                seq,
                outcome,
                error,
            } = frame
            else {
                unreachable!()
            };
            WireFrame::End {
                seq,
                outcome,
                error,
            }
        })
    }
}

impl futures::stream::FusedStream for FrameStream {
    fn is_terminated(&self) -> bool {
        self.events.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimux_core::stream_part::TextStreamPart;

    #[test]
    fn direct_text_frame_matches_typed_frame_wire_bytes() {
        let seq = u64::MAX - 1;
        let event = StreamEvent::Text(TextStreamPart::TextDelta {
            id: "0".into(),
            provider_metadata: None,
            text: "非 ASCII\n\u{0000}".into(),
        });
        let expected = serde_json::to_string(&StreamFrame::Event {
            seq: seq.to_string(),
            event: serde_json::to_value(&event).unwrap(),
        })
        .unwrap();
        let WireFrame::Event {
            seq: wire_seq,
            json,
        } = WireFrame::event(seq, event).unwrap()
        else {
            panic!("expected data frame");
        };
        assert_eq!(wire_seq, seq);
        assert_eq!(json, expected);
    }

    #[test]
    fn direct_wire_frame_accepts_exact_limit_and_rejects_one_byte_more() {
        let build = |text: String| WireFrame::event(0, StreamEvent::Json(text.into()));
        let WireFrame::Event { json, .. } = build(String::new()).unwrap() else {
            panic!("expected data frame");
        };
        let content_length = crate::limits::ITEM_BYTES - json.len();
        let WireFrame::Event { json, .. } = build("x".repeat(content_length)).unwrap() else {
            panic!("expected data frame");
        };
        assert_eq!(json.len(), crate::limits::ITEM_BYTES);
        assert!(matches!(
            build("x".repeat(content_length + 1)),
            Err(OpsError::Protocol(ProtocolError::ResourceLimit(_)))
        ));
    }
}
