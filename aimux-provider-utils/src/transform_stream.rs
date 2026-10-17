//! The Rust form of the WHATWG `TransformStream` that the AI SDK's models
//! build their `doStream` output with (`response.pipeThrough(new
//! TransformStream({ start, transform, flush }))`).
//!
//! The state the TypeScript closures capture lives in a [`Transformer`]
//! value instead, so a model's stream logic is a plain struct with methods
//! that can be driven and tested without any I/O.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use futures::Stream;

use aimux_core::error::AiMuxError;

/// What a transformer writes to: the `TransformStreamDefaultController`.
///
/// [`pipe_through`] owns one and drains it after every call into the
/// transformer; tests can own one directly, call the transformer's methods
/// with it and [`drain`](Self::drain) what was emitted.
pub struct TransformStreamController<O> {
    queue: VecDeque<Result<O, AiMuxError>>,
    closed: bool,
}

impl<O> Default for TransformStreamController<O> {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            closed: false,
        }
    }
}

impl<O> TransformStreamController<O> {
    /// An open controller with nothing queued.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Take everything emitted so far, in order. The buffer is kept for reuse.
    pub fn drain(&mut self) -> impl Iterator<Item = Result<O, AiMuxError>> + '_ {
        self.queue.drain(..)
    }

    /// Whether [`error`](Self::error) or [`terminate`](Self::terminate) ended
    /// the stream.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Emit one value downstream.
    pub fn enqueue(&mut self, value: O) {
        if !self.closed {
            self.queue.push_back(Ok(value));
        }
    }

    /// Emit `error` and keep the stream open: the next chunk is transformed
    /// as usual. For errors a consumer may skip (a malformed event inside an
    /// otherwise valid stream). The AI SDK's models do this by enqueueing an
    /// `{ type: 'error', error }` part and carrying on.
    pub fn enqueue_error(&mut self, error: AiMuxError) {
        if !self.closed {
            self.queue.push_back(Err(error));
        }
    }

    /// Emit `error` and end the stream: no further chunk is transformed and
    /// `flush` is not called. WHATWG `controller.error(e)`.
    pub fn error(&mut self, error: AiMuxError) {
        if !self.closed {
            self.queue.push_back(Err(error));
            self.closed = true;
        }
    }

    /// End the stream without an error: no further chunk is transformed and
    /// `flush` is not called. WHATWG `controller.terminate()`. Anything
    /// emitted after the stream ended is dropped.
    pub fn terminate(&mut self) {
        self.closed = true;
    }
}

/// The `{ start, transform, flush }` object of a `TransformStream`.
pub trait Transformer: Send + 'static {
    type Input: Send + 'static;
    type Output: Send + 'static;

    /// Called once, before the first chunk.
    fn start(&mut self, controller: &mut TransformStreamController<Self::Output>) {
        let _ = controller;
    }

    /// Called for every input chunk.
    fn transform(
        &mut self,
        chunk: Self::Input,
        controller: &mut TransformStreamController<Self::Output>,
    );

    /// Called once when the input ends, unless the stream was ended earlier
    /// with [`TransformStreamController::error`] or
    /// [`TransformStreamController::terminate`].
    fn flush(self, controller: &mut TransformStreamController<Self::Output>);
}

/// `input.pipeThrough(new TransformStream(transformer))`.
///
/// Like the WHATWG constructor, this calls [`Transformer::start`] right away;
/// every later call happens while the returned stream is polled.
pub fn pipe_through<S, T>(input: S, mut transformer: T) -> PipeThrough<S, T>
where
    S: Stream<Item = T::Input> + Send + 'static,
    T: Transformer,
{
    let mut controller = TransformStreamController::new();
    transformer.start(&mut controller);
    PipeThrough {
        input: Box::pin(input),
        transformer: Some(transformer),
        controller,
    }
}

/// The stream [`pipe_through`] returns: it pulls a chunk from `input` only
/// when everything the transformer emitted so far has been handed out.
pub struct PipeThrough<S, T: Transformer> {
    input: Pin<Box<S>>,
    /// `None` once `flush` has run.
    transformer: Option<T>,
    controller: TransformStreamController<T::Output>,
}

// The transformer is never pinned: it is only reached through `&mut`.
impl<S, T: Transformer> Unpin for PipeThrough<S, T> {}

impl<S, T> Stream for PipeThrough<S, T>
where
    S: Stream<Item = T::Input>,
    T: Transformer,
{
    type Item = Result<T::Output, AiMuxError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(item) = this.controller.queue.pop_front() {
                return Poll::Ready(Some(item));
            }
            if this.controller.closed {
                return Poll::Ready(None);
            }
            let Some(transformer) = this.transformer.as_mut() else {
                return Poll::Ready(None);
            };
            match ready!(this.input.as_mut().poll_next(cx)) {
                Some(chunk) => transformer.transform(chunk, &mut this.controller),
                None => {
                    if let Some(transformer) = this.transformer.take() {
                        transformer.flush(&mut this.controller);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;

    use super::*;

    /// Doubles each number, reports a skippable error for zero, stops with an
    /// error on a negative one, and emits the running total on flush.
    #[derive(Default)]
    struct Doubler {
        total: i32,
    }

    impl Transformer for Doubler {
        type Input = i32;
        type Output = i32;

        fn start(&mut self, controller: &mut TransformStreamController<i32>) {
            controller.enqueue(0);
        }

        fn transform(&mut self, chunk: i32, controller: &mut TransformStreamController<i32>) {
            if chunk == 0 {
                controller.enqueue_error(AiMuxError::InvalidResponseData("zero".into()));
                return;
            }
            if chunk < 0 {
                controller.error(AiMuxError::InvalidResponseData("negative".into()));
                return;
            }
            self.total += chunk;
            controller.enqueue(chunk * 2);
        }

        fn flush(self, controller: &mut TransformStreamController<i32>) {
            controller.enqueue(self.total);
        }
    }

    async fn run(input: Vec<i32>) -> Vec<Result<i32, String>> {
        pipe_through(futures::stream::iter(input), Doubler::default())
            .map(|item| item.map_err(|error| error.to_string()))
            .collect()
            .await
    }

    #[tokio::test]
    async fn start_transform_then_flush_in_order() {
        assert_eq!(run(vec![1, 2]).await, vec![Ok(0), Ok(2), Ok(4), Ok(3)]);
    }

    #[tokio::test]
    async fn enqueue_error_keeps_transforming_and_flushes() {
        let out = run(vec![1, 0, 2]).await;
        assert_eq!(out.len(), 5);
        assert_eq!(out[..2], [Ok(0), Ok(2)]);
        assert!(out[2].is_err());
        assert_eq!(out[3..], [Ok(4), Ok(3)]);
    }

    #[tokio::test]
    async fn error_ends_the_stream_without_flush() {
        let out = run(vec![1, -1, 5]).await;
        assert_eq!(out.len(), 3);
        assert_eq!(out[..2], [Ok(0), Ok(2)]);
        assert!(out[2].is_err());
    }

    #[test]
    fn a_transformer_can_be_driven_without_a_stream() {
        let mut controller = TransformStreamController::new();
        let mut doubler = Doubler::default();
        doubler.start(&mut controller);
        doubler.transform(3, &mut controller);
        let emitted: Vec<i32> = controller.drain().map(Result::unwrap).collect();
        assert_eq!(emitted, [0, 6]);
        doubler.transform(-1, &mut controller);
        assert!(controller.is_closed());
        assert!(controller.drain().next().unwrap().is_err());
    }

    #[test]
    fn nothing_is_emitted_after_the_stream_ended() {
        let mut controller = TransformStreamController::new();
        controller.enqueue(1);
        controller.terminate();
        controller.enqueue(2);
        controller.enqueue_error(AiMuxError::InvalidResponseData("late".into()));
        controller.error(AiMuxError::InvalidResponseData("late".into()));
        let emitted: Vec<i32> = controller.drain().map(Result::unwrap).collect();
        assert_eq!(emitted, [1]);
        assert!(controller.is_closed());
    }

    #[tokio::test]
    async fn waits_for_input_that_is_not_ready_yet() {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let mut out = pipe_through(rx, Doubler::default());
        assert_eq!(out.next().await.unwrap().unwrap(), 0);
        tx.unbounded_send(4).unwrap();
        assert_eq!(out.next().await.unwrap().unwrap(), 8);
        drop(tx);
        assert_eq!(out.next().await.unwrap().unwrap(), 4);
        assert!(out.next().await.is_none());
        assert!(out.next().await.is_none());
    }
}
