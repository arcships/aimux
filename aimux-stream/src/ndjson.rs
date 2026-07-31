//! NDJSON (newline-delimited JSON) stream decoder.

use bytes::Bytes;
use futures::Stream;
use pin_project_lite::pin_project;
use serde::de::DeserializeOwned;
use std::pin::Pin;
use std::task::{Context, Poll};
use thiserror::Error;

/// Default upper bound on a single buffered NDJSON line's size (1 MiB).
const DEFAULT_MAX_LINE_SIZE: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum NdjsonError {
    #[error("utf-8 error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("stream error: {0}")]
    Stream(String),
    #[error("NDJSON line exceeded maximum allowed size")]
    LineTooLarge,
}

pin_project! {
    /// Decodes a byte stream into parsed NDJSON values.
    ///
    /// Bytes are accumulated in a raw `Vec<u8>` buffer and split on `\n`. Each
    /// complete line is strictly UTF-8 decoded only *after* reassembly, so a
    /// multi-byte character split across two network chunks is never corrupted
    /// into replacement chars — unlike a per-chunk
    /// `String::from_utf8_lossy` decode, which would emit a `U+FFFD` on each
    /// side of the split.
    pub struct NdjsonStream<S, T, E> {
        #[pin]
        inner: S,
        buffer: Vec<u8>,
        max_line_size: usize,
        _marker: std::marker::PhantomData<(T, E)>,
    }
}

impl<S, T, E> NdjsonStream<S, T, E>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    T: DeserializeOwned,
{
    pub fn new(stream: S) -> Self {
        Self::with_max_line_size(stream, DEFAULT_MAX_LINE_SIZE)
    }

    /// Create an [`NdjsonStream`] with a custom per-line size limit. A line
    /// longer than `max_line_size` bytes (excluding the `\n`) yields
    /// [`NdjsonError::LineTooLarge`], as does a buffer that grows past the
    /// limit while waiting for a newline.
    pub fn with_max_line_size(stream: S, max_line_size: usize) -> Self {
        Self {
            inner: stream,
            buffer: Vec::new(),
            max_line_size,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<S, T, E> Stream for NdjsonStream<S, T, E>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    T: DeserializeOwned,
    E: std::fmt::Display,
{
    type Item = Result<T, NdjsonError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();

        loop {
            // Try to parse a complete JSON line.
            if let Some(pos) = this.buffer.iter().position(|&b| b == b'\n') {
                if pos > this.max_line_size {
                    // Drop the oversized line so a retried poll makes progress.
                    this.buffer.drain(..pos + 1);
                    return Poll::Ready(Some(Err(NdjsonError::LineTooLarge)));
                }
                // Extract the line bytes (without the newline) and decode
                // strictly. Decoding the fully reassembled line (rather than
                // `from_utf8_lossy` per chunk) preserves code points split
                // across chunks and surfaces invalid UTF-8 as an error.
                let line_bytes: Vec<u8> = this.buffer.drain(..pos).collect();
                this.buffer.drain(..1); // the newline
                let line = match String::from_utf8(line_bytes) {
                    Ok(s) => s,
                    Err(e) => return Poll::Ready(Some(Err(NdjsonError::Utf8(e)))),
                };
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<T>(line) {
                    Ok(value) => return Poll::Ready(Some(Ok(value))),
                    Err(e) => return Poll::Ready(Some(Err(NdjsonError::Json(e)))),
                }
            }

            // No complete line yet. Guard against unbounded buffer growth when
            // a newline never arrives. The partial can never form a valid line
            // under the limit, so drop it — a retried poll then makes progress
            // instead of looping on the same oversized buffer.
            if this.buffer.len() > this.max_line_size {
                this.buffer.clear();
                return Poll::Ready(Some(Err(NdjsonError::LineTooLarge)));
            }

            // Read more bytes.
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    this.buffer.extend_from_slice(&bytes);
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(NdjsonError::Stream(e.to_string()))));
                }
                Poll::Ready(None) => {
                    // Stream ended — try to parse any remaining buffer.
                    if this.buffer.len() > this.max_line_size {
                        return Poll::Ready(Some(Err(NdjsonError::LineTooLarge)));
                    }
                    let remaining_bytes = std::mem::take(&mut this.buffer);
                    let remaining = match String::from_utf8(remaining_bytes) {
                        Ok(s) => s,
                        Err(e) => return Poll::Ready(Some(Err(NdjsonError::Utf8(e)))),
                    };
                    let remaining = remaining.trim();
                    if remaining.is_empty() {
                        return Poll::Ready(None);
                    }
                    match serde_json::from_str::<T>(remaining) {
                        Ok(value) => return Poll::Ready(Some(Ok(value))),
                        Err(e) => return Poll::Ready(Some(Err(NdjsonError::Json(e)))),
                    }
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
