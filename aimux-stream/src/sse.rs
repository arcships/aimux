//! SSE (Server-Sent Events) parser for streaming model responses.

use bytes::Bytes;
use futures::Stream;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};
use thiserror::Error;

/// Default upper bound on a single buffered SSE event's size (1 MiB).
const DEFAULT_MAX_EVENT_SIZE: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum SseError {
    #[error("utf-8 decode error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("stream error: {0}")]
    Stream(String),
    #[error("SSE frame exceeded maximum allowed size")]
    FrameTooLarge,
}

/// A parsed SSE event.
#[derive(Debug, Clone, Default)]
pub struct SseEvent {
    /// The `event:` field (optional).
    pub event: Option<String>,
    /// The `data:` field.
    pub data: String,
    /// The `id:` field (optional).
    pub id: Option<String>,
    /// The `retry:` field (optional).
    pub retry: Option<u64>,
}

pin_project! {
    /// An adapter that decodes a byte stream into SSE events.
    ///
    /// Bytes are accumulated in a raw `Vec<u8>` buffer and split on the SSE
    /// event terminator (a blank line). Each complete frame is strictly
    /// UTF-8 decoded only *after* reassembly, so a multi-byte character split
    /// across two network chunks is never corrupted into replacement chars —
    /// unlike a per-chunk `String::from_utf8_lossy` decode, which would emit a
    /// `U+FFFD` on each side of the split.
    pub struct SseStream<S, E> {
        #[pin]
        inner: S,
        buffer: Vec<u8>,
        max_event_size: usize,
        done: bool,
        _err: std::marker::PhantomData<E>,
    }
}

impl<S, E> SseStream<S, E>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
{
    pub fn new(stream: S) -> Self {
        Self::with_max_event_size(stream, DEFAULT_MAX_EVENT_SIZE)
    }

    /// Create an [`SseStream`] with a custom per-event size limit. A single
    /// event frame (the bytes between two terminators, excluding the
    /// terminator itself) larger than `max_event_size` bytes yields
    /// [`SseError::FrameTooLarge`], as does a buffer that grows past the limit
    /// while waiting for a terminator.
    pub fn with_max_event_size(stream: S, max_event_size: usize) -> Self {
        Self {
            inner: stream,
            buffer: Vec::new(),
            max_event_size,
            done: false,
            _err: std::marker::PhantomData,
        }
    }
}

impl<S, E> Stream for SseStream<S, E>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    type Item = Result<SseEvent, SseError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();

        loop {
            // Try to emit a complete event from the buffer.
            if let Some((frame_len, sep_len)) = find_separator(&this.buffer) {
                if frame_len > this.max_event_size {
                    // Drop the oversized frame so a retried poll makes progress.
                    this.buffer.drain(..frame_len + sep_len);
                    return Poll::Ready(Some(Err(SseError::FrameTooLarge)));
                }
                // Extract the frame bytes and drop the terminator. Decoding the
                // fully reassembled frame with `String::from_utf8` (rather than
                // `from_utf8_lossy` per chunk) preserves code points split
                // across chunks and surfaces invalid UTF-8 as an error.
                let frame_bytes: Vec<u8> = this.buffer.drain(..frame_len).collect();
                this.buffer.drain(..sep_len);
                let frame = match String::from_utf8(frame_bytes) {
                    Ok(s) => s,
                    Err(e) => return Poll::Ready(Some(Err(SseError::Utf8(e)))),
                };
                let (event, has_data_line) = try_parse_event(&frame);
                // Per the SSE spec / eventsource-parser, an event is dispatched
                // only if it had at least one `data:` line (dataLines > 0).
                // This covers comment-only, `event:`/`id:`/`retry:`-only, and
                // blank-line keep-alives (all dispatched as nothing), while an
                // explicit empty data line (`data:\n\n`) is still dispatched.
                if !has_data_line {
                    continue;
                }
                return Poll::Ready(Some(Ok(event)));
            }

            // No complete event yet. Guard against unbounded buffer growth when
            // a terminator never arrives. The partial can never form a valid
            // frame under the limit, so drop it — a retried poll then makes
            // progress instead of looping on the same oversized buffer.
            if this.buffer.len() > this.max_event_size {
                this.buffer.clear();
                return Poll::Ready(Some(Err(SseError::FrameTooLarge)));
            }

            if this.done {
                // No terminating blank line: the buffered partial is not a
                // complete event and is dropped. This matches eventsource-parser's
                // `EventSourceParserStream`, which has no flush handler — a
                // partial event at EOF is simply not dispatched.
                return Poll::Ready(None);
            }

            // Read more data.
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    this.buffer.extend_from_slice(&bytes);
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(SseError::Stream(e.to_string()))));
                }
                Poll::Ready(None) => {
                    this.done = true;
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Locate the next SSE event terminator (a blank line) in `buf`.
///
/// Returns `(frame_len, sep_len)` where `frame_len` is the number of bytes
/// *before* the terminator and `sep_len` is the terminator's length. Prefers
/// `\n\n` (so a pure-CRLF stream — which contains no bare `\n\n` — falls
/// through to `\r\n\r\n`), mirroring the str-based
/// `find("\n\n").or_else(|| find("\r\n\r\n"))`.
fn find_separator(buf: &[u8]) -> Option<(usize, usize)> {
    if let Some(pos) = find_subsequence(buf, b"\n\n") {
        return Some((pos, 2));
    }
    find_subsequence(buf, b"\r\n\r\n").map(|pos| (pos, 4))
}

/// First index of `needle` in `haystack`, comparing raw bytes.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Parse a single complete SSE event frame (the bytes between two
/// terminators, already strictly UTF-8 decoded) into an [`SseEvent`] plus a
/// flag indicating whether the event contained at least one `data:` line.
///
/// A complete event is terminated by a blank line: `\n\n` or `\r\n\r\n` (the
/// terminator is consumed by the caller before this runs). Per the SSE spec,
/// exactly one leading U+0020 SPACE after the `:` is removed from each field
/// value (not all leading whitespace). Comment lines (starting with `:`) and
/// unknown fields are ignored.
fn try_parse_event(frame: &str) -> (SseEvent, bool) {
    let mut event = SseEvent::default();
    let mut has_data_line = false;
    for line in frame.lines() {
        if let Some(value) = field_value(line, "data:") {
            has_data_line = true;
            if event.data.is_empty() {
                event.data = value.to_string();
            } else {
                event.data.push('\n');
                event.data.push_str(value);
            }
        } else if let Some(value) = field_value(line, "event:") {
            event.event = Some(value.to_string());
        } else if let Some(value) = field_value(line, "id:") {
            event.id = Some(value.to_string());
        } else if let Some(value) = field_value(line, "retry:") {
            event.retry = value.parse().ok();
        }
        // Comment lines (`:` prefix) and unknown fields are ignored.
    }

    (event, has_data_line)
}

/// Strip `prefix` from `line` and remove exactly one leading U+0020 SPACE from
/// the remainder (per the SSE spec). Returns `None` if `line` does not start
/// with `prefix`.
fn field_value<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(prefix)?;
    // Remove exactly one leading space, if present.
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_event() {
        let (event, has_data_line) = try_parse_event("data: hello world");
        assert_eq!(event.data, "hello world");
        assert!(has_data_line);
    }

    #[test]
    fn parse_multi_line_data() {
        let (event, has_data_line) = try_parse_event("data: line1\ndata: line2");
        assert_eq!(event.data, "line1\nline2");
        assert!(has_data_line);
    }

    #[test]
    fn parse_event_with_type() {
        let (event, has_data_line) = try_parse_event("event: message\ndata: payload");
        assert_eq!(event.event.as_deref(), Some("message"));
        assert_eq!(event.data, "payload");
        assert!(has_data_line);
    }

    #[test]
    fn parse_event_without_data_has_no_data_line() {
        // An `event:`-only event has no data line -> not dispatched.
        let (event, has_data_line) = try_parse_event("event: ping");
        assert_eq!(event.event.as_deref(), Some("ping"));
        assert!(event.data.is_empty());
        assert!(!has_data_line);
    }
}
