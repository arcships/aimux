//! Core-owned operation deadlines and caller cancellation.

use std::future::{Future, pending};

use tokio::time::Instant;

use crate::AbortSignal;
use crate::error::AiMuxError;
use crate::options::TimeoutConfiguration;

/// Deadlines shared by the establishment and streaming phases of one model
/// operation. No timer task is spawned: the future currently driving the
/// operation observes the deadline directly.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OperationTimeout {
    total: Option<TimeoutDeadline>,
    step: Option<TimeoutDeadline>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TimeoutDeadline {
    pub(crate) at: Instant,
    label: &'static str,
    duration_ms: u64,
}

impl TimeoutDeadline {
    pub(crate) fn from_now(label: &'static str, duration_ms: u64) -> Result<Self, AiMuxError> {
        Self::after(Instant::now(), label, duration_ms)
    }

    fn after(start: Instant, label: &'static str, duration_ms: u64) -> Result<Self, AiMuxError> {
        let at = start
            .checked_add(std::time::Duration::from_millis(duration_ms))
            .ok_or_else(|| {
                AiMuxError::InvalidArgument(format!(
                    "{label} timeout of {duration_ms}ms exceeds the supported range"
                ))
            })?;
        Ok(Self {
            at,
            label,
            duration_ms,
        })
    }

    #[must_use]
    pub(crate) fn error(self) -> AiMuxError {
        AiMuxError::Timeout(format!(
            "{} timeout of {}ms exceeded",
            self.label, self.duration_ms
        ))
    }
}

impl OperationTimeout {
    pub(crate) fn new(configuration: TimeoutConfiguration) -> Result<Self, AiMuxError> {
        let now = Instant::now();
        let total = configuration
            .total_ms
            .map(|duration_ms| TimeoutDeadline::after(now, "Total", duration_ms))
            .transpose()?;
        let step = configuration
            .step_ms
            .map(|duration_ms| TimeoutDeadline::after(now, "Step", duration_ms))
            .transpose()?;

        // Validate streaming-only durations here as well, before a provider
        // operation starts. Their actual deadlines begin at stream-specific
        // points and are constructed again with the same checked helper.
        if let Some(duration_ms) = configuration.first_chunk_ms {
            TimeoutDeadline::after(now, "First chunk", duration_ms)?;
        }
        if let Some(duration_ms) = configuration.chunk_ms {
            TimeoutDeadline::after(now, "Chunk", duration_ms)?;
        }

        Ok(Self { total, step })
    }

    /// The first active operation deadline. Aimux currently has one model
    /// step, so total and step begin together.
    #[must_use]
    pub(crate) fn deadline(self) -> Option<TimeoutDeadline> {
        match (self.total, self.step) {
            (Some(total), Some(step)) if step.at < total.at => Some(step),
            (Some(total), _) => Some(total),
            (None, step) => step,
        }
    }
}

/// Run an operation while directly observing caller cancellation and its
/// total/step deadline. Dropping `operation` cancels the in-flight Rust HTTP
/// future; timeout is not represented by mutating an AbortSignal.
pub(crate) async fn run<T>(
    operation: impl Future<Output = Result<T, AiMuxError>>,
    abort_signal: Option<&AbortSignal>,
    timeout: OperationTimeout,
) -> Result<T, AiMuxError> {
    run_until(operation, abort_signal, timeout.deadline()).await
}

/// The earlier of two optional deadlines.
pub(crate) fn min_deadline(
    a: Option<TimeoutDeadline>,
    b: Option<TimeoutDeadline>,
) -> Option<TimeoutDeadline> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if b.at < a.at { b } else { a }),
        (Some(deadline), None) | (None, Some(deadline)) => Some(deadline),
        (None, None) => None,
    }
}

/// [`run`] with an explicit deadline, for callers that race additional
/// phase-specific deadlines (the stream setup phase also observes
/// `first_chunk_ms`).
pub(crate) async fn run_until<T>(
    operation: impl Future<Output = Result<T, AiMuxError>>,
    abort_signal: Option<&AbortSignal>,
    deadline: Option<TimeoutDeadline>,
) -> Result<T, AiMuxError> {
    tokio::select! {
        biased;
        () = wait_for_abort(abort_signal) => {
            Err(AiMuxError::Aborted("request aborted".into()))
        }
        () = wait_for_deadline(deadline) => {
            Err(deadline.expect("deadline future only resolves for a deadline").error())
        }
        result = operation => result,
    }
}

pub(crate) async fn wait_for_abort(signal: Option<&AbortSignal>) {
    match signal {
        Some(signal) => signal.cancelled().await,
        None => pending().await,
    }
}

pub(crate) async fn wait_for_deadline(deadline: Option<TimeoutDeadline>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.at).await,
        None => pending().await,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::retry;

    fn always_retry(_error: &AiMuxError) -> bool {
        true
    }

    fn unchanged_delay(_error: &AiMuxError, delay: u64) -> u64 {
        delay
    }

    #[tokio::test]
    async fn total_timeout_cancels_the_operation_without_mutating_abort_signal() {
        let timeout = OperationTimeout::new(TimeoutConfiguration {
            total_ms: Some(1),
            ..Default::default()
        })
        .unwrap();
        let result = run(
            std::future::pending::<Result<(), AiMuxError>>(),
            None,
            timeout,
        )
        .await;
        assert!(
            matches!(result, Err(AiMuxError::Timeout(message)) if message == "Total timeout of 1ms exceeded")
        );
    }

    #[tokio::test]
    async fn caller_abort_remains_distinct_from_timeout() {
        let caller = AbortSignal::new();
        let timeout = OperationTimeout::new(TimeoutConfiguration {
            total_ms: Some(60_000),
            ..Default::default()
        })
        .unwrap();
        caller.abort();
        let error = run(
            std::future::pending::<Result<(), AiMuxError>>(),
            Some(&caller),
            timeout,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AiMuxError::Aborted(message) if message == "request aborted"));
    }

    #[tokio::test]
    async fn total_timeout_includes_retry_backoff() {
        let attempts = AtomicUsize::new(0);
        let retry = retry::retry_with_exponential_backoff(
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Err::<(), _>(AiMuxError::Other("retry".into())) }
            },
            2,
            60_000,
            2,
            None,
            always_retry,
            unchanged_delay,
        );
        let timeout = OperationTimeout::new(TimeoutConfiguration {
            total_ms: Some(1),
            ..Default::default()
        })
        .unwrap();

        let error = run(retry, None, timeout).await.unwrap_err();

        assert!(
            matches!(error, AiMuxError::Timeout(message) if message == "Total timeout of 1ms exceeded")
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn largest_timeout_durations_never_panic() {
        let configurations = [
            TimeoutConfiguration {
                total_ms: Some(u64::MAX),
                ..Default::default()
            },
            TimeoutConfiguration {
                step_ms: Some(u64::MAX),
                ..Default::default()
            },
            TimeoutConfiguration {
                first_chunk_ms: Some(u64::MAX),
                ..Default::default()
            },
            TimeoutConfiguration {
                chunk_ms: Some(u64::MAX),
                ..Default::default()
            },
        ];

        for configuration in configurations {
            // `Instant` ranges differ by platform, so this value may be
            // representable; the contract is either success or a typed error.
            let result = std::panic::catch_unwind(|| OperationTimeout::new(configuration))
                .expect("timeout validation must not panic");
            assert!(matches!(
                result,
                Ok(_) | Err(AiMuxError::InvalidArgument(_))
            ));
        }
    }
}
