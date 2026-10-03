//! SSE (Server-Sent Events) decoding for streaming model responses.
//!
//! The parser is the [`sse-stream`](https://crates.io/crates/sse-stream) crate
//! (the WHATWG "parsing an event stream" algorithm: `\n`, `\r` and `\r\n`
//! line endings in any mix, a leading UTF-8 BOM, `field: value` with one
//! optional leading space, lines without `:` as empty-valued fields,
//! `:`-prefixed comments, an `id` containing U+0000 ignored, `retry` only when
//! all ASCII digits, unknown fields ignored, a partial block at end of stream
//! dropped).
//!
//! This module is the thin adapter that gives it the AI SDK's dispatch
//! semantics, the way `parseJsonEventStream` wraps `eventsource-parser`:
//!
//! - a block is dispatched only if it had at least one `data` line; comment-
//!   only and metadata-only blocks are dropped;
//! - an empty `event` value means no event type;
//! - `retry` is reported on the event it was parsed with (upstream reports it
//!   through `onRetry`), so a `retry` block without `data` is dropped;
//! - there is no buffer size limit, as in `parseJsonEventStream`.
//!
//! Differences from upstream kept on purpose: a field value that is not valid
//! UTF-8 is a strict [`SseError::Utf8`] and ends the stream (upstream decodes
//! lossily), and a transport error ends the stream after it is reported.

use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use bytes::Bytes;
use futures::Stream;
use pin_project_lite::pin_project;
use sse_stream::SseByteStream;
use thiserror::Error;

/// A terminal decoding error: after yielding one, the stream ends.
#[derive(Debug, Error)]
pub enum SseError {
    /// A recognized field value (`data`, `event`, `id`, `retry`) is not valid
    /// UTF-8.
    #[error("utf-8 decode error: {0}")]
    Utf8(#[source] std::str::Utf8Error),
    /// The underlying byte stream failed; the source is the transport error.
    #[error("stream error: {0}")]
    Stream(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Any other decoder error.
    #[error("SSE decode error: {0}")]
    Decode(#[source] sse_stream::Error),
}

impl From<sse_stream::Error> for SseError {
    fn from(error: sse_stream::Error) -> Self {
        match error {
            sse_stream::Error::Body(source) => Self::Stream(source),
            sse_stream::Error::Utf8Parse(source) => Self::Utf8(source),
            other => Self::Decode(other),
        }
    }
}

/// A parsed SSE event.
#[derive(Debug, Clone, Default)]
pub struct SseEvent {
    /// The `event:` field (`None` when absent or empty).
    pub event: Option<String>,
    /// The `data:` field; multiple `data` lines are joined with `\n`.
    pub data: String,
    /// The `id:` field (optional).
    pub id: Option<String>,
    /// The `retry:` field (optional; all-digit values only).
    pub retry: Option<u64>,
}

pin_project! {
    /// An adapter that decodes a byte stream into SSE events.
    pub struct SseStream<S, E>
    where
        S: Stream<Item = Result<Bytes, E>>,
    {
        #[pin]
        inner: SseByteStream<S>,
        _err: PhantomData<E>,
    }
}

impl<S, E> SseStream<S, E>
where
    S: Stream<Item = Result<Bytes, E>>,
{
    pub fn new(stream: S) -> Self {
        Self {
            inner: SseByteStream::new(stream),
            _err: PhantomData,
        }
    }
}

impl<S, E> Stream for SseStream<S, E>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: std::error::Error + Send + Sync + 'static,
{
    type Item = Result<SseEvent, SseError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        loop {
            match ready!(this.inner.as_mut().poll_next(cx)) {
                None => return Poll::Ready(None),
                Some(Err(error)) => return Poll::Ready(Some(Err(error.into()))),
                Some(Ok(block)) => {
                    // Dispatch only blocks that carried at least one `data`
                    // line (`dataLines > 0` upstream).
                    let Some(data) = block.data else { continue };
                    return Poll::Ready(Some(Ok(SseEvent {
                        event: block.event.filter(|event| !event.is_empty()),
                        data,
                        id: block.id,
                        retry: block.retry,
                    })));
                }
            }
        }
    }
}
