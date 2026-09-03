//! RFC-0031 §13 acceptance: chunk-timer semantics (item 17),
//! producer-side timing (consumer delay never counts), and
//! no-lingering-timer-state after drop (item 21b).
//!
//! Paused tokio time makes every assertion exact: auto-advance jumps to the
//! earliest armed deadline, so elapsed times distinguish "reset" from
//! "not reset" deterministically.

use std::time::Duration;

use aimux_core::AbortSignal;
use aimux_core::error::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, stream_text};
use aimux_core::language_model::LanguageModel;
use aimux_core::options::{CallOptions, TimeoutConfiguration};
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use async_trait::async_trait;
use futures::StreamExt;

/// Streams: TextDelta at t=0, then after 600ms one more part (semantic or
/// not), then pends forever. The follow-up part is the probe: only semantic
/// output may reset the chunk timer.
struct ProbeModel {
    second_part_is_output: bool,
}

fn delta(text: &str) -> StreamPart {
    StreamPart::TextDelta {
        id: "1".into(),
        delta: text.into(),
        provider_metadata: None,
    }
}

#[async_trait]
impl LanguageModel for ProbeModel {
    fn provider(&self) -> &str {
        "mock-probe"
    }

    fn model_id(&self) -> &str {
        "probe-1"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        unimplemented!("streaming-only mock")
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let second_is_output = self.second_part_is_output;
        Ok(StreamResult {
            stream: Box::pin(async_stream::stream! {
                yield Ok(delta("first"));
                tokio::time::sleep(Duration::from_millis(600)).await;
                if second_is_output {
                    yield Ok(delta("second"));
                } else {
                    // Non-semantic per the AI SDK's isOutputChunk: must NOT
                    // reset the chunk timer.
                    yield Ok(StreamPart::TextStart {
                        id: "1".into(),
                        provider_metadata: None,
                    });
                }
                futures::future::pending::<()>().await;
            }),
            request_body: None,
            response_headers: None,
        })
    }
}

fn chunk_options() -> GenerateTextOptions {
    GenerateTextOptions {
        max_retries: Some(0),
        timeout: Some(TimeoutConfiguration {
            chunk_ms: Some(1_000),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Drives the probe stream to the chunk timeout and returns when it fired.
async fn time_to_chunk_timeout(second_part_is_output: bool) -> Duration {
    let model = ProbeModel {
        second_part_is_output,
    };
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

#[tokio::test(start_paused = true)]
async fn non_semantic_part_does_not_reset_the_chunk_timer() {
    // TextStart at t=600 must not push the deadline: timeout fires 1000ms
    // after the only semantic output at t=0, not 1600ms.
    assert_eq!(
        time_to_chunk_timeout(false).await,
        Duration::from_millis(1_000)
    );
}

#[tokio::test(start_paused = true)]
async fn semantic_output_resets_the_chunk_timer() {
    // The second TextDelta at t=600 re-arms the deadline to t=1600.
    assert_eq!(
        time_to_chunk_timeout(true).await,
        Duration::from_millis(1_600)
    );
}

#[tokio::test(start_paused = true)]
async fn consumer_delay_does_not_count_against_the_chunk_timer() {
    // The second delta arrives at t=600, inside the 1000ms chunk budget
    // measured from the first delta. A consumer that sleeps until t=2000
    // before polling again must still receive it; only the *next* poll sees
    // the timeout, which fired at t=1600 on the producer side.
    let model = ProbeModel {
        second_part_is_output: true,
    };
    let mut result = stream_text(&model, "hello", chunk_options()).await.unwrap();
    assert!(result.stream.next().await.expect("first part").is_ok());

    tokio::time::sleep(Duration::from_millis(2_000)).await;

    match result.stream.next().await.expect("second part") {
        Ok(StreamPart::TextDelta { delta, .. }) => assert_eq!(delta, "second"),
        other => panic!("expected the buffered second delta, got {other:?}"),
    }
    match result.stream.next().await.expect("timeout") {
        Err(AiMuxError::Timeout(message)) => {
            assert_eq!(message, "Chunk timeout of 1000ms exceeded");
        }
        other => panic!("expected chunk timeout, got {other:?}"),
    }
    assert!(result.stream.next().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn dropped_operation_leaves_no_armed_timer_state() {
    // §8.0/§13-21b: deadlines live inside the pump task that drives the
    // provider stream, and that task is aborted when the returned stream is
    // dropped. Dropping the stream mid-flight must leave no timer that could
    // later fire and mutate the caller's signal (the rejected abort_after
    // design would trip this).
    let caller = AbortSignal::new();
    let model = ProbeModel {
        second_part_is_output: false,
    };
    let options = GenerateTextOptions {
        max_retries: Some(0),
        abort_signal: Some(caller.clone()),
        timeout: Some(TimeoutConfiguration {
            total_ms: Some(5_000),
            chunk_ms: Some(1_000),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut result = stream_text(&model, "hello", options).await.unwrap();
    let first = result.stream.next().await.expect("first part");
    assert!(first.is_ok());
    drop(result);

    // Sail far past every configured deadline.
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(!caller.is_aborted());
}
