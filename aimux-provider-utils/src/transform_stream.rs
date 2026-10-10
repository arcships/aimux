//! The Rust form of the WHATWG `TransformStream` that the AI SDK's models
//! build their `doStream` output with (`response.pipeThrough(new
//! TransformStream({ start, transform, flush }))`).
//!
//! The state the TypeScript closures capture lives in a [`Transformer`]
//! value instead, so a model's stream logic is a plain struct with methods
//! that can be driven and tested without any I/O.

use futures::{Stream, StreamExt};

use aimux_core::error::AiMuxError;

/// What a transformer writes to: the `TransformStreamDefaultController`.
pub struct TransformStreamController<O> {
    queue: Vec<Result<O, AiMuxError>>,
    closed: bool,
}

impl<O> TransformStreamController<O> {
    fn new() -> Self {
        Self {
            queue: Vec::new(),
            closed: false,
        }
    }

    /// Emit one value downstream.
    pub fn enqueue(&mut self, value: O) {
        if !self.closed {
            self.queue.push(Ok(value));
        }
    }

    /// Emit `error` and keep the stream open: the next chunk is transformed
    /// as usual. For errors a consumer may skip (a malformed event inside an
    /// otherwise valid stream).
    pub fn enqueue_error(&mut self, error: AiMuxError) {
        if !self.closed {
            self.queue.push(Err(error));
        }
    }

    /// Emit `error` and end the stream: no further chunk is transformed and
    /// `flush` is not called.
    pub fn error(&mut self, error: AiMuxError) {
        if !self.closed {
            self.queue.push(Err(error));
            self.closed = true;
        }
    }

    /// End the stream without an error: no further chunk is transformed and
    /// `flush` is not called.
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
pub fn pipe_through<S, T>(
    input: S,
    mut transformer: T,
) -> impl Stream<Item = Result<T::Output, AiMuxError>> + Send
where
    S: Stream<Item = T::Input> + Send + 'static,
    T: Transformer,
{
    async_stream::stream! {
        let mut controller = TransformStreamController::new();
        transformer.start(&mut controller);
        for item in std::mem::take(&mut controller.queue) {
            yield item;
        }
        if controller.closed {
            return;
        }
        let mut input = Box::pin(input);
        while let Some(chunk) = input.next().await {
            transformer.transform(chunk, &mut controller);
            for item in std::mem::take(&mut controller.queue) {
                yield item;
            }
            if controller.closed {
                return;
            }
        }
        transformer.flush(&mut controller);
        for item in std::mem::take(&mut controller.queue) {
            yield item;
        }
    }
}

#[cfg(test)]
mod tests {
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
}
