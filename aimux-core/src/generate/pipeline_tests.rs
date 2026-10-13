//! RFC-0031 §13 acceptance for the streaming call pipeline: the chunk timer
//! (items 16-19c), producer-side timing, the stream's terminal fuse, the
//! first-event peek and its retry, and no timer state after a drop
//! (21a-21c). Ported from the master branch's
//! `tests/stream_timeout_matrix_test.rs` and `generate.rs` stream tests,
//! adjusted to the AI SDK's full stream (`Start`, `StartStep`, ...).
//!
//! The chunk timer follows `streamText` (ai@7.0.127 `stream-text.ts`): it is
//! armed and reset only by output chunks (`isOutputChunk`, :182-225, :2842-
//! 2857) and reports `Chunk timeout of {ms}ms exceeded` (:2257-2266). The
//! first-chunk timer is armed at operation start rather than when the
//! response stream begins (upstream :2228-2243), because aimux providers
//! peek the first event inside `do_stream` (RFC-0031 §8.3).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;

use super::*;
use crate::error::ApiCallError;
use crate::options::TimeoutConfiguration;
use crate::types::{FinishReason, Usage};

#[derive(Clone, Copy)]
enum Behavior {
    /// One text delta, then the stream ends.
    Normal,
    /// A text delta at t=0, another part at t=600ms (a delta when `true`,
    /// a non-output `TextStart` otherwise), then silence.
    Probe(bool),
    /// A finish part, then silence.
    FinishThenPending,
    /// A provider error part and a finish part, then silence.
    ProviderErrorThenFinish,
    /// A malformed frame, then a text delta.
    ParseErrorThenDelta,
    /// A retryable transport error, then silence.
    TransportErrorThenPending,
    /// `do_stream` never returns (a 200-then-silence server whose first
    /// event the provider awaits inside `do_stream`).
    NeverReturns,
    /// The first `do_stream` fails with a retryable 429.
    RetryableFirstError,
    /// `do_stream` fails with a non-retryable 400.
    NonRetryableFirstError,
}

struct PipelineModel {
    behavior: Behavior,
    calls: AtomicUsize,
}

impl PipelineModel {
    fn new(behavior: Behavior) -> Self {
        Self {
            behavior,
            calls: AtomicUsize::new(0),
        }
    }
}

fn delta(text: &str) -> StreamPart {
    StreamPart::TextDelta {
        id: "1".into(),
        delta: text.into(),
        provider_metadata: None,
    }
}

fn finish() -> StreamPart {
    StreamPart::Finish {
        finish_reason: FinishReason {
            unified: FinishReasonUnified::Error,
            raw: None,
        },
        usage: Usage::default(),
        provider_metadata: None,
    }
}

fn api_error(status: u16, retryable: bool) -> AiMuxError {
    AiMuxError::ApiCall(Box::new(ApiCallError {
        status_code: Some(status),
        is_retryable: retryable,
        ..ApiCallError::new(
            "failed",
            "https://example.test/stream",
            serde_json::json!({}),
        )
    }))
}

fn stream_result(
    stream: impl Stream<Item = Result<StreamPart, AiMuxError>> + Send + 'static,
) -> StreamResult {
    StreamResult {
        stream: Box::pin(stream),
        request: None,
        response: None,
    }
}

#[async_trait]
impl LanguageModel for PipelineModel {
    fn provider(&self) -> &str {
        "mock.pipeline"
    }

    fn model_id(&self) -> &str {
        "pipeline-1"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        unimplemented!("streaming-only mock")
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let attempt = self.calls.fetch_add(1, Ordering::SeqCst);
        match self.behavior {
            Behavior::Normal => Ok(stream_result(futures::stream::iter([Ok(delta("hello"))]))),
            Behavior::Probe(second_is_output) => Ok(stream_result(async_stream::stream! {
                yield Ok(delta("first"));
                tokio::time::sleep(Duration::from_millis(600)).await;
                if second_is_output {
                    yield Ok(delta("second"));
                } else {
                    yield Ok(StreamPart::TextStart { id: "1".into(), provider_metadata: None });
                }
                futures::future::pending::<()>().await;
            })),
            Behavior::FinishThenPending => Ok(stream_result(
                futures::stream::iter([Ok(delta("done")), Ok(finish())])
                    .chain(futures::stream::pending()),
            )),
            Behavior::ProviderErrorThenFinish => Ok(stream_result(
                futures::stream::iter([
                    Ok(StreamPart::Error {
                        error: AiMuxError::Other("provider error".into()),
                    }),
                    Ok(finish()),
                ])
                .chain(futures::stream::pending()),
            )),
            Behavior::ParseErrorThenDelta => Ok(stream_result(futures::stream::iter([
                Err(AiMuxError::JsonParse("malformed SSE data".into())),
                Ok(delta("after-error")),
            ]))),
            Behavior::TransportErrorThenPending => Ok(stream_result(
                futures::stream::iter([Err(api_error(502, true))])
                    .chain(futures::stream::pending()),
            )),
            Behavior::NeverReturns => std::future::pending().await,
            Behavior::RetryableFirstError if attempt == 0 => Err(api_error(429, true)),
            Behavior::RetryableFirstError => {
                Ok(stream_result(futures::stream::iter([Ok(delta("hello"))])))
            }
            Behavior::NonRetryableFirstError => Err(api_error(400, false)),
        }
    }
}

fn options(timeout: TimeoutConfiguration) -> GenerateTextOptions {
    GenerateTextOptions {
        max_retries: Some(0),
        timeout: Some(timeout),
        ..GenerateTextOptions::default()
    }
}

fn chunk_options() -> GenerateTextOptions {
    options(TimeoutConfiguration {
        chunk_ms: Some(1_000),
        ..TimeoutConfiguration::default()
    })
}

/// The text of each `TextDelta` among `parts`.
fn texts(parts: &[Result<TextStreamPart, AiMuxError>]) -> Vec<String> {
    parts
        .iter()
        .filter_map(|part| match part {
            Ok(TextStreamPart::TextDelta { text, .. }) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Reads the next `TextDelta`'s text, skipping framing parts.
async fn next_text(
    stream: &mut (impl Stream<Item = Result<TextStreamPart, AiMuxError>> + Unpin),
) -> String {
    loop {
        match stream.next().await.expect("stream must not end") {
            Ok(TextStreamPart::TextDelta { text, .. }) => return text,
            Ok(_) => {}
            Err(error) => panic!("unexpected error: {error}"),
        }
    }
}

/// Drives the probe stream to the chunk timeout and returns when it fired.
async fn time_to_chunk_timeout(second_part_is_output: bool) -> Duration {
    let model = PipelineModel::new(Behavior::Probe(second_part_is_output));
    let mut result = stream_text(&model, "hello", chunk_options()).await.unwrap();
    let start = tokio::time::Instant::now();
    loop {
        match result.stream.next().await.expect("stream must not end") {
            Ok(_) => {}
            Err(AiMuxError::Timeout(message)) => {
                assert_eq!(message, "Chunk timeout of 1000ms exceeded");
                return start.elapsed();
            }
            Err(other) => panic!("unexpected error: {other}"),
        }
    }
}

/// §13-17, stream-text.ts:182-225: a `text-start` is not an output chunk, so
/// the timeout fires 1000ms after the only output at t=0, not at t=1600.
#[tokio::test(start_paused = true)]
async fn non_output_part_does_not_reset_the_chunk_timer() {
    assert_eq!(
        time_to_chunk_timeout(false).await,
        Duration::from_millis(1_000)
    );
}

/// §13-17, stream-text.ts:2842-2857: the second delta at t=600 re-arms the
/// deadline to t=1600.
#[tokio::test(start_paused = true)]
async fn output_resets_the_chunk_timer() {
    assert_eq!(
        time_to_chunk_timeout(true).await,
        Duration::from_millis(1_600)
    );
}

/// §13-18: the timers run on the producer side. The second delta arrives at
/// t=600, inside the budget; a consumer that polls again only at t=2000
/// still receives it, and the next poll sees the timeout that fired at
/// t=1600.
#[tokio::test(start_paused = true)]
async fn consumer_delay_does_not_count_against_the_chunk_timer() {
    let model = PipelineModel::new(Behavior::Probe(true));
    let mut result = stream_text(&model, "hello", chunk_options()).await.unwrap();
    assert_eq!(next_text(&mut result.stream).await, "first");

    tokio::time::sleep(Duration::from_millis(2_000)).await;

    assert_eq!(next_text(&mut result.stream).await, "second");
    match result.stream.next().await.expect("timeout") {
        Err(AiMuxError::Timeout(message)) => {
            assert_eq!(message, "Chunk timeout of 1000ms exceeded");
        }
        other => panic!("expected the chunk timeout, got {other:?}"),
    }
    assert!(result.stream.next().await.is_none());
}

/// §13-21b: the deadlines live in the pump task, which is aborted when the
/// returned stream is dropped; no timer is left to fire later and touch the
/// caller's signal.
#[tokio::test(start_paused = true)]
async fn a_dropped_stream_leaves_no_armed_timer() {
    let caller = AbortSignal::new();
    let model = PipelineModel::new(Behavior::Probe(false));
    let options = GenerateTextOptions {
        abort_signal: Some(caller.clone()),
        ..options(TimeoutConfiguration {
            total_ms: Some(5_000),
            chunk_ms: Some(1_000),
            ..TimeoutConfiguration::default()
        })
    };
    let mut result = stream_text(&model, "hello", options).await.unwrap();
    assert_eq!(next_text(&mut result.stream).await, "first");
    drop(result);

    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(!caller.is_aborted());
}

/// §8.3: the event the provider peeked inside `do_stream` is not lost.
#[tokio::test]
async fn stream_setup_preserves_the_peeked_first_event() {
    let model = PipelineModel::new(Behavior::Normal);
    let result = stream_text(&model, "hello", GenerateTextOptions::default())
        .await
        .unwrap();
    let parts: Vec<_> = result.stream.collect().await;
    assert_eq!(texts(&parts), ["hello"]);
}

/// §13-18: the first-chunk budget measures the arrival of the first output,
/// not when the consumer first polls.
#[tokio::test]
async fn a_late_consumer_still_receives_a_first_chunk_delivered_on_time() {
    let model = PipelineModel::new(Behavior::Normal);
    let result = stream_text(
        &model,
        "hello",
        options(TimeoutConfiguration {
            first_chunk_ms: Some(1),
            ..TimeoutConfiguration::default()
        }),
    )
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_millis(20)).await;

    let parts: Vec<_> = result.stream.collect().await;
    assert!(parts.iter().all(Result::is_ok), "{parts:?}");
    assert_eq!(texts(&parts), ["hello"]);
}

/// The model's `finish` ends the stream even when the provider keeps the
/// connection open: no total timeout fires afterwards.
#[tokio::test(start_paused = true)]
async fn the_finish_part_ends_the_stream() {
    let model = PipelineModel::new(Behavior::FinishThenPending);
    let result = stream_text(
        &model,
        "hello",
        options(TimeoutConfiguration {
            total_ms: Some(50),
            ..TimeoutConfiguration::default()
        }),
    )
    .await
    .unwrap();
    let parts: Vec<_> = result.stream.collect().await;
    let kinds: Vec<String> = parts
        .iter()
        .map(|part| part.as_ref().map(super::tests::kind).unwrap_or_default())
        .collect();
    assert_eq!(
        kinds,
        ["start", "start-step", "text-delta", "finish-step", "finish"]
    );
}

/// A provider's error part does not end the stream before its `finish`.
#[tokio::test]
async fn a_provider_error_part_does_not_hide_the_finish() {
    let model = PipelineModel::new(Behavior::ProviderErrorThenFinish);
    let result = stream_text(&model, "hello", GenerateTextOptions::default())
        .await
        .unwrap();
    let parts: Vec<_> = result.stream.collect().await;
    let kinds: Vec<String> = parts
        .iter()
        .map(|part| part.as_ref().map(super::tests::kind).unwrap_or_default())
        .collect();
    assert_eq!(
        kinds,
        ["start", "start-step", "error", "finish-step", "finish"]
    );
    let Some(Ok(TextStreamPart::Finish { finish_reason, .. })) = parts.last() else {
        unreachable!()
    };
    assert_eq!(*finish_reason, FinishReasonUnified::Error);
}

/// A malformed frame is reported and the stream goes on, with a total
/// timeout or an abort signal armed alike.
#[tokio::test]
async fn the_stream_continues_after_a_malformed_frame() {
    for options in [
        options(TimeoutConfiguration {
            total_ms: Some(1_000),
            ..TimeoutConfiguration::default()
        }),
        GenerateTextOptions {
            abort_signal: Some(AbortSignal::new()),
            ..GenerateTextOptions::default()
        },
    ] {
        let model = PipelineModel::new(Behavior::ParseErrorThenDelta);
        let result = stream_text(&model, "hello", options).await.unwrap();
        let parts: Vec<_> = result.stream.collect().await;
        let error = parts
            .iter()
            .position(|part| matches!(part, Err(AiMuxError::JsonParse(_))));
        let text = parts.iter().position(
            |part| matches!(part, Ok(TextStreamPart::TextDelta { text, .. }) if text == "after-error"),
        );
        assert!(
            error.is_some() && text.is_some() && error < text,
            "{parts:?}"
        );
    }
}

/// A transport error ends the stream instead of waiting on the dead source.
#[tokio::test]
async fn a_transport_error_ends_the_stream() {
    let model = PipelineModel::new(Behavior::TransportErrorThenPending);
    let mut result = stream_text(&model, "hello", GenerateTextOptions::default())
        .await
        .unwrap();
    loop {
        match result.stream.next().await.expect("the error part") {
            Ok(_) => {}
            Err(error) => {
                assert!(matches!(error, AiMuxError::ApiCall(_)), "{error:?}");
                break;
            }
        }
    }
    assert!(result.stream.next().await.is_none());
}

/// §8.3: with `first_chunk_ms` alone, a provider awaiting its first event
/// inside `do_stream` is bounded.
#[tokio::test]
async fn the_first_chunk_timeout_bounds_stream_setup() {
    let model = PipelineModel::new(Behavior::NeverReturns);
    let Err(error) = stream_text(
        &model,
        "hello",
        options(TimeoutConfiguration {
            first_chunk_ms: Some(5),
            ..TimeoutConfiguration::default()
        }),
    )
    .await
    else {
        panic!("expected a timeout");
    };
    assert!(
        matches!(error, AiMuxError::Timeout(ref message) if message == "First chunk timeout of 5ms exceeded"),
        "got {error:?}"
    );
}

/// A retryable error from `do_stream` (the provider's first event, e.g. a
/// 429) is retried before the stream is returned (stream-text.ts:2409-2420
/// wraps the model call in `prepareRetries`' retry).
#[tokio::test(start_paused = true)]
async fn a_retryable_first_stream_error_is_retried() {
    let model = PipelineModel::new(Behavior::RetryableFirstError);
    let options = GenerateTextOptions {
        max_retries: Some(2),
        ..GenerateTextOptions::default()
    };
    let result = stream_text(&model, "hello", options).await.unwrap();
    assert_eq!(model.calls.load(Ordering::SeqCst), 2);
    let parts: Vec<_> = result.stream.collect().await;
    assert_eq!(texts(&parts), ["hello"]);
}

/// A non-retryable error from `do_stream` is returned once, unchanged.
#[tokio::test]
async fn a_non_retryable_first_stream_error_is_returned_unchanged() {
    let model = PipelineModel::new(Behavior::NonRetryableFirstError);
    let options = GenerateTextOptions {
        max_retries: Some(2),
        ..GenerateTextOptions::default()
    };
    let Err(error) = stream_text(&model, "hello", options).await else {
        panic!("expected the error");
    };
    assert!(matches!(error, AiMuxError::ApiCall(ref detail) if detail.status_code == Some(400)));
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
}
