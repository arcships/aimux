//! NDJSON (newline-delimited JSON) stream decoder.

use bytes::Bytes;
use futures::Stream;
use pin_project_lite::pin_project;
use serde::de::DeserializeOwned;
use std::pin::Pin;
use std::task::{Context, Poll};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NdjsonError {
    #[error("utf-8 error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("stream error: {0}")]
    Stream(String),
}

pin_project! {
    /// Decodes a byte stream into parsed NDJSON values.
    pub struct NdjsonStream<S, T, E> {
        #[pin]
        inner: S,
        buffer: String,
        _marker: std::marker::PhantomData<(T, E)>,
    }
}

impl<S, T, E> NdjsonStream<S, T, E>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    T: DeserializeOwned,
{
    pub fn new(stream: S) -> Self {
        Self {
            inner: stream,
            buffer: String::new(),
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
            if let Some(pos) = this.buffer.find('\n') {
                let line = this.buffer[..pos].trim().to_string();
                this.buffer = this.buffer[pos + 1..].to_string();

                if line.is_empty() {
                    continue;
                }

                match serde_json::from_str::<T>(&line) {
                    Ok(value) => return Poll::Ready(Some(Ok(value))),
                    Err(e) => return Poll::Ready(Some(Err(NdjsonError::Json(e)))),
                }
            }

            // Read more bytes.
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    this.buffer.push_str(&String::from_utf8_lossy(&bytes));
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(NdjsonError::Stream(e.to_string()))));
                }
                Poll::Ready(None) => {
                    // Stream ended — try to parse any remaining buffer.
                    let remaining = std::mem::take(&mut this.buffer);
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
