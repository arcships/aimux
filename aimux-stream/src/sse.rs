//! SSE (Server-Sent Events) parser for streaming model responses.
//!
//! Line-based incremental parser following `eventsource-parser`'s
//! `createParser` (the parser behind the AI SDK's `parseJsonEventStream`) and
//! the WHATWG "parsing an event stream" algorithm:
//!
//! - lines end in `\n`, `\r` or `\r\n`, freely mixed; a `\r` at the end of
//!   the buffered input waits for the next chunk in case a `\n` follows;
//! - a UTF-8 BOM at the very start of the stream is stripped;
//! - a blank line dispatches the event, but only if it had at least one
//!   `data` line; field state resets after every blank line;
//! - `field: value` removes exactly one leading U+0020 SPACE; a line without
//!   `:` is a field with an empty value; `:`-prefixed lines are comments;
//! - an empty `event` value means no event type; an `id` containing U+0000 is
//!   ignored; `retry` is accepted only when it is all ASCII digits; unknown
//!   fields are ignored;
//! - a partial event at end of stream is not dispatched.
//!
//! Two aimux additions: every line is strictly UTF-8 decoded after it has
//! been reassembled (a code point split across chunks is never corrupted, and
//! invalid UTF-8 surfaces as [`SseError::Utf8`] and discards the rest of
//! that event up to the next blank line), and buffered input is bounded by `max_event_size` (the parser's
//! `maxBufferSize`; exceeding it is fatal and ends the stream, as upstream).

use std::collections::VecDeque;

use bytes::Bytes;
use futures::Stream;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};
use thiserror::Error;

/// Default upper bound on buffered event data plus any partial line (1 MiB).
const DEFAULT_MAX_EVENT_SIZE: usize = 1024 * 1024;

const BOM: &[u8] = b"\xEF\xBB\xBF";

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
    /// The `event:` field (`None` when absent or empty).
    pub event: Option<String>,
    /// The `data:` field; multiple `data` lines are joined with `\n`.
    pub data: String,
    /// The `id:` field (optional).
    pub id: Option<String>,
    /// The `retry:` field (optional; all-digit values only).
    pub retry: Option<u64>,
}

/// Incremental SSE line parser (the `createParser` state machine).
#[derive(Debug)]
struct Parser {
    /// Bytes after the last complete line terminator.
    pending: Vec<u8>,
    /// Offset in `pending` from which the next terminator search starts, so a
    /// long line fed in many small chunks is scanned once, not quadratically.
    scan_from: usize,
    bom_checked: bool,
    event: Option<String>,
    data: String,
    data_lines: usize,
    id: Option<String>,
    retry: Option<u64>,
    /// Set after a line fails UTF-8 decoding: the rest of the event (up to
    /// the next blank line) is discarded rather than dispatched as a fragment.
    poisoned: bool,
    max_size: usize,
    terminated: bool,
    ready: VecDeque<Result<SseEvent, SseError>>,
}

impl Parser {
    fn new(max_size: usize) -> Self {
        Self {
            pending: Vec::new(),
            scan_from: 0,
            bom_checked: false,
            event: None,
            data: String::new(),
            data_lines: 0,
            id: None,
            retry: None,
            poisoned: false,
            max_size,
            terminated: false,
            ready: VecDeque::new(),
        }
    }

    fn feed(&mut self, chunk: &[u8]) {
        if self.terminated {
            return;
        }
        self.pending.extend_from_slice(chunk);

        if !self.bom_checked {
            // Wait until the first three bytes can rule a BOM in or out.
            if self.pending.len() < BOM.len() && BOM.starts_with(&self.pending) {
                return;
            }
            if self.pending.starts_with(BOM) {
                self.pending.drain(..BOM.len());
            }
            self.bom_checked = true;
        }

        let mut line_start = 0;
        let mut search = self.scan_from;
        while let Some(offset) = self.pending[search..]
            .iter()
            .position(|&b| b == b'\n' || b == b'\r')
        {
            let end = search + offset;
            let is_cr = self.pending[end] == b'\r';
            // A trailing `\r` may be the first half of a `\r\n` split across
            // chunks: defer it until more input arrives.
            if is_cr && end + 1 == self.pending.len() {
                break;
            }
            let line = self.pending[line_start..end].to_vec();
            self.parse_line(&line);
            if self.terminated {
                return;
            }
            line_start = end + 1;
            if is_cr && self.pending.get(line_start) == Some(&b'\n') {
                line_start += 1;
            }
            search = line_start;
        }
        self.pending.drain(..line_start);
        // Resume the next search at the deferred `\r`, if any.
        self.scan_from = self.pending.len().saturating_sub(1);
        if self.pending.last() != Some(&b'\r') {
            self.scan_from = self.pending.len();
        }
        self.check_size(self.pending.len());
    }

    fn parse_line(&mut self, line: &[u8]) {
        if line.is_empty() {
            self.dispatch();
            return;
        }
        if self.poisoned {
            return;
        }
        let line = match String::from_utf8(line.to_vec()) {
            Ok(line) => line,
            Err(error) => {
                // The event being built can no longer be trusted: drop it and
                // everything up to the next blank line.
                self.reset_event();
                self.poisoned = true;
                self.ready.push_back(Err(SseError::Utf8(error)));
                return;
            }
        };
        if line.starts_with(':') {
            return; // comment
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line.as_str(), ""),
        };
        match field {
            "event" => self.event = (!value.is_empty()).then(|| value.to_string()),
            "data" => {
                if self.data_lines > 0 {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.data_lines += 1;
                // Lines already consumed from `pending` are not buffered any
                // more; only the event data counts until the chunk is done.
                self.check_size(0);
            }
            "id" => {
                if !value.contains('\0') {
                    self.id = Some(value.to_string());
                }
            }
            "retry" => {
                if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
                    self.retry = value.parse().ok();
                }
            }
            _ => {} // unknown field
        }
    }

    fn dispatch(&mut self) {
        if self.data_lines > 0 {
            self.ready.push_back(Ok(SseEvent {
                event: self.event.take(),
                data: std::mem::take(&mut self.data),
                id: self.id.take(),
                retry: self.retry.take(),
            }));
        }
        self.reset_event();
    }

    fn reset_event(&mut self) {
        self.poisoned = false;
        self.event = None;
        self.data.clear();
        self.data_lines = 0;
        self.id = None;
        self.retry = None;
    }

    /// `maxBufferSize`: buffered event data plus the partial line. Exceeding
    /// it is fatal, as in `EventSourceParserStream`.
    fn check_size(&mut self, pending_len: usize) {
        if self.terminated || pending_len + self.data.len() <= self.max_size {
            return;
        }
        self.terminated = true;
        self.pending.clear();
        self.reset_event();
        self.ready.push_back(Err(SseError::FrameTooLarge));
    }
}

pin_project! {
    /// An adapter that decodes a byte stream into SSE events.
    pub struct SseStream<S, E> {
        #[pin]
        inner: S,
        parser: Parser,
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

    /// Create an [`SseStream`] with a custom buffer limit: when buffered event
    /// data plus the current partial line exceed `max_event_size` bytes, the
    /// stream yields [`SseError::FrameTooLarge`] and ends.
    pub fn with_max_event_size(stream: S, max_event_size: usize) -> Self {
        Self {
            inner: stream,
            parser: Parser::new(max_event_size),
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
            if let Some(item) = this.parser.ready.pop_front() {
                return Poll::Ready(Some(item));
            }
            // A partial event at end of stream is not dispatched: the
            // upstream stream has no flush handler.
            if this.done || this.parser.terminated {
                return Poll::Ready(None);
            }
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => this.parser.feed(&bytes),
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(SseError::Stream(e.to_string()))));
                }
                Poll::Ready(None) => this.done = true,
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Vec<SseEvent> {
        let mut parser = Parser::new(DEFAULT_MAX_EVENT_SIZE);
        parser.feed(input.as_bytes());
        parser.ready.into_iter().map(Result::unwrap).collect()
    }

    #[test]
    fn parse_single_event() {
        assert_eq!(parse("data: hello world\n\n")[0].data, "hello world");
    }

    #[test]
    fn parse_multi_line_data() {
        assert_eq!(
            parse("data: line1\ndata: line2\n\n")[0].data,
            "line1\nline2"
        );
    }

    #[test]
    fn parse_event_with_type() {
        let events = parse("event: message\ndata: payload\n\n");
        assert_eq!(events[0].event.as_deref(), Some("message"));
        assert_eq!(events[0].data, "payload");
    }

    #[test]
    fn parse_event_without_data_is_not_dispatched() {
        assert!(parse("event: ping\n\n").is_empty());
    }
}
