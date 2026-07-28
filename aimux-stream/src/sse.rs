//! SSE (Server-Sent Events) parser for streaming model responses.

use bytes::Bytes;
use futures::Stream;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SseError {
    #[error("utf-8 decode error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("stream error: {0}")]
    Stream(String),
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
    pub struct SseStream<S, E> {
        #[pin]
        inner: S,
        buffer: String,
        done: bool,
        _err: std::marker::PhantomData<E>,
    }
}

impl<S, E> SseStream<S, E>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
{
    pub fn new(stream: S) -> Self {
        Self {
            inner: stream,
            buffer: String::new(),
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
            if let Some((event, rest, has_data_line)) = try_parse_event(&this.buffer) {
                this.buffer = rest;
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
                    this.buffer.push_str(&String::from_utf8_lossy(&bytes));
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

/// Try to extract one complete SSE event from `buf`.
/// Returns `(event, remaining_buffer, has_data_line)` if a full event was
/// found. `has_data_line` is true if the event contained at least one `data:`
/// line (per the SSE spec, an event is only dispatched when it has a data
/// line — `dataLines > 0` in eventsource-parser).
///
/// A complete event is terminated by a blank line: `\n\n` or `\r\n\r\n`.
/// Per the SSE spec, exactly one leading U+0020 SPACE after the `:` is removed
/// from each field value (not all leading whitespace). Comment lines (starting
/// with `:`) and unknown fields are ignored.
fn try_parse_event(buf: &str) -> Option<(SseEvent, String, bool)> {
    // SSE events are separated by a blank line (`\n\n` or `\r\n\r\n`).
    // Prefer `\n\n`: a CRLF stream contains no bare `\n\n` (the two `\n`s in
    // `\r\n\r\n` are separated by a `\r`), so pure-CRLF input falls through to
    // the `\r\n\r\n` arm, while mixed-ending input splits at the earliest `\n\n`.
    let separator = buf.find("\n\n").or_else(|| buf.find("\r\n\r\n"))?;
    let (raw_event, rest) = buf.split_at(separator);
    // Skip the separator itself (`\r\n\r\n` or `\n\n`).
    let rest = match rest
        .strip_prefix("\r\n\r\n")
        .or_else(|| rest.strip_prefix("\n\n"))
    {
        Some(remaining) => remaining.to_string(),
        None => rest.to_string(),
    };

    let mut event = SseEvent::default();
    let mut has_data_line = false;
    for line in raw_event.lines() {
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

    Some((event, rest, has_data_line))
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
        let buf = "data: hello world\n\n";
        let (event, rest, has_data_line) = try_parse_event(buf).unwrap();
        assert_eq!(event.data, "hello world");
        assert!(rest.is_empty());
        assert!(has_data_line);
    }

    #[test]
    fn parse_multi_line_data() {
        let buf = "data: line1\ndata: line2\n\n";
        let (event, _, has_data_line) = try_parse_event(buf).unwrap();
        assert_eq!(event.data, "line1\nline2");
        assert!(has_data_line);
    }

    #[test]
    fn parse_event_with_type() {
        let buf = "event: message\ndata: payload\n\n";
        let (event, _, has_data_line) = try_parse_event(buf).unwrap();
        assert_eq!(event.event.as_deref(), Some("message"));
        assert_eq!(event.data, "payload");
        assert!(has_data_line);
    }

    #[test]
    fn parse_event_without_data_has_no_data_line() {
        // An `event:`-only event has no data line -> not dispatched.
        let buf = "event: ping\n\n";
        let (event, _, has_data_line) = try_parse_event(buf).unwrap();
        assert_eq!(event.event.as_deref(), Some("ping"));
        assert!(event.data.is_empty());
        assert!(!has_data_line);
    }
}
