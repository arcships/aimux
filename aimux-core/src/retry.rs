//! Operation-level retry, aligned with the AI SDK retry utilities.

use std::future::Future;
use std::time::Duration;

use rand::Rng;

use crate::AbortSignal;
use crate::error::{AiMuxError, RetryError, RetryErrorReason};

const DEFAULT_MAX_RETRIES: u32 = 2;
const INITIAL_DELAY_MS: u64 = 2_000;
const BACKOFF_FACTOR: u32 = 2;

/// Default retry settings for model operations.
///
/// Per-call `max_retries` overrides only [`Self::max_retries`]; the configured
/// delay and backoff factor remain in effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryConfig {
    /// Maximum number of retries after the initial attempt.
    pub max_retries: u32,
    /// Initial delay between attempts.
    pub initial_delay: Duration,
    /// Multiplier applied to the delay after each retry.
    pub backoff_factor: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            initial_delay: Duration::from_millis(INITIAL_DELAY_MS),
            backoff_factor: BACKOFF_FACTOR,
        }
    }
}

/// A prepared operation retry function and its resolved retry count.
///
/// This is the Rust representation of the AI SDK's
/// `prepareRetries()` result: `{ maxRetries, retry }`.
#[derive(Debug, Clone)]
pub struct PreparedRetries {
    /// The effective maximum after applying the default.
    pub max_retries: u32,
    initial_delay_ms: u64,
    backoff_factor: u64,
    abort_signal: Option<AbortSignal>,
}

/// Bind retry settings, applying a per-call retry-count override.
#[must_use]
pub fn prepare_retries(
    max_retries: Option<u32>,
    mut config: RetryConfig,
    abort_signal: Option<AbortSignal>,
) -> PreparedRetries {
    if let Some(max_retries) = max_retries {
        config.max_retries = max_retries;
    }
    PreparedRetries {
        max_retries: config.max_retries,
        initial_delay_ms: u64::try_from(config.initial_delay.as_millis()).unwrap_or(u64::MAX),
        backoff_factor: u64::from(config.backoff_factor),
        abort_signal,
    }
}

impl PreparedRetries {
    /// Retry a complete operation using API-call retryability and response hints.
    ///
    /// # Errors
    ///
    /// Returns the operation error, retry exhaustion, or caller cancellation.
    pub async fn retry<F, Fut, T>(&self, operation: F) -> Result<T, AiMuxError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, AiMuxError>>,
    {
        retry_with_exponential_backoff_respecting_retry_headers(
            operation,
            self.max_retries,
            self.initial_delay_ms,
            self.backoff_factor,
            self.abort_signal.as_ref(),
        )
        .await
    }
}

async fn retry_with_exponential_backoff_respecting_retry_headers<F, Fut, T>(
    operation: F,
    max_retries: u32,
    initial_delay_ms: u64,
    backoff_factor: u64,
    abort_signal: Option<&AbortSignal>,
) -> Result<T, AiMuxError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, AiMuxError>>,
{
    retry_with_exponential_backoff(
        operation,
        max_retries,
        initial_delay_ms,
        backoff_factor,
        abort_signal,
        |error: &AiMuxError| matches!(error, AiMuxError::ApiCall(detail) if detail.is_retryable),
        |error: &AiMuxError, exponential_delay| {
            retry_delay_with_jitter(error, exponential_delay, |maximum| {
                rand::thread_rng().gen_range(0..=maximum)
            })
        },
    )
    .await
}

/// Generic exponential retry primitive. The caller supplies retry
/// classification and delay selection.
///
/// # Errors
///
/// Returns the operation error, retry exhaustion, or caller cancellation.
pub(crate) async fn retry_with_exponential_backoff<F, Fut, T, ShouldRetry, GetDelay>(
    mut operation: F,
    max_retries: u32,
    initial_delay_ms: u64,
    backoff_factor: u64,
    abort_signal: Option<&AbortSignal>,
    mut should_retry: ShouldRetry,
    mut get_delay_ms: GetDelay,
) -> Result<T, AiMuxError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, AiMuxError>>,
    ShouldRetry: FnMut(&AiMuxError) -> bool,
    GetDelay: FnMut(&AiMuxError, u64) -> u64,
{
    let mut errors = Vec::new();
    let mut exponential_delay = initial_delay_ms;

    loop {
        let result = match abort_signal {
            Some(signal) => {
                if signal.is_aborted() {
                    return Err(AiMuxError::from_abort_signal(signal));
                }
                tokio::select! {
                    biased;
                    () = signal.cancelled() => Err(AiMuxError::from_abort_signal(signal)),
                    result = async { operation().await } => result,
                }
            }
            None => operation().await,
        };

        match result {
            Ok(value) => return Ok(value),
            Err(error @ (AiMuxError::Aborted(_) | AiMuxError::Timeout(_))) => {
                return Err(error);
            }
            // A safe inner exchange may exhaust its own retries inside a
            // larger non-idempotent operation. Pass that exhaustion through
            // so the enclosing operation is not submitted again.
            Err(error @ AiMuxError::Retry(_)) => return Err(error),
            Err(error) if max_retries == 0 => return Err(error),
            Err(error) => {
                tracing::warn!(
                    target: "aimux_core::retry",
                    attempt = errors.len() + 1,
                    error = %error,
                    "operation attempt failed"
                );
                errors.push(error);
                let try_number = errors.len() as u32;

                if try_number > max_retries {
                    tracing::error!(
                        target: "aimux_core::retry",
                        attempts = errors.len(),
                        error = %errors.last().expect("retry history is non-empty"),
                        "operation retry exhausted"
                    );
                    return Err(AiMuxError::Retry(RetryError {
                        reason: RetryErrorReason::MaxRetriesExceeded,
                        errors,
                    }));
                }

                let error = errors.last().expect("attempt error was just appended");
                if should_retry(error) {
                    let retry_delay = get_delay_ms(error, exponential_delay);
                    delay(Duration::from_millis(retry_delay), abort_signal).await?;
                    exponential_delay = exponential_delay.saturating_mul(backoff_factor);
                    continue;
                }

                if try_number == 1 {
                    tracing::error!(
                        target: "aimux_core::retry",
                        error = %error,
                        "operation failed without retry"
                    );
                    return Err(errors.pop().expect("the first attempt error exists"));
                }

                tracing::error!(
                    target: "aimux_core::retry",
                    attempts = errors.len(),
                    error = %error,
                    "operation stopped on non-retryable error"
                );
                return Err(AiMuxError::Retry(RetryError {
                    reason: RetryErrorReason::ErrorNotRetryable,
                    errors,
                }));
            }
        }
    }
}

/// Abort-aware delay between attempts (also used by video poll pacing).
///
/// # Errors
///
/// Returns [`AiMuxError::Aborted`] when the caller cancels during the delay.
pub(crate) async fn delay(
    duration: Duration,
    abort_signal: Option<&AbortSignal>,
) -> Result<(), AiMuxError> {
    match abort_signal {
        Some(signal) => {
            tokio::select! {
                biased;
                () = signal.cancelled() => Err(AiMuxError::from_abort_signal(signal)),
                () = tokio::time::sleep(duration) => Ok(()),
            }
        }
        None => {
            tokio::time::sleep(duration).await;
            Ok(())
        }
    }
}

fn retry_header_delay(error: &AiMuxError, exponential_delay: u64) -> Option<u64> {
    let parsed = u64::try_from(error.retry_after_hint()?).ok()?;

    (parsed < 60_000 || parsed < exponential_delay).then_some(parsed)
}

fn retry_delay_with_jitter(
    error: &AiMuxError,
    exponential_delay: u64,
    jitter: impl FnOnce(u64) -> u64,
) -> u64 {
    retry_header_delay(error, exponential_delay)
        .unwrap_or_else(|| jitter(exponential_delay).min(exponential_delay))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::ApiCallError;

    fn api_error(retryable: bool) -> AiMuxError {
        AiMuxError::ApiCall(Box::new(ApiCallError {
            is_retryable: retryable,
            ..ApiCallError::new("retry", "https://example.test", serde_json::json!({}))
        }))
    }

    fn api_error_with_headers(status: u16, headers: &[(&str, &str)]) -> AiMuxError {
        AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: Some(status),
            response_headers: Some(
                headers
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                    .collect::<HashMap<_, _>>(),
            ),
            is_retryable: true,
            ..ApiCallError::new("retry", "https://example.test", serde_json::json!({}))
        }))
    }

    fn should_retry(error: &AiMuxError) -> bool {
        error.is_retryable()
    }

    fn unchanged_delay(_error: &AiMuxError, delay: u64) -> u64 {
        delay
    }

    #[tokio::test]
    async fn prepared_retries_resolves_the_default_and_binds_abort() {
        assert_eq!(
            prepare_retries(None, RetryConfig::default(), None).max_retries,
            2
        );

        let signal = AbortSignal::new();
        signal.abort();
        let retries = prepare_retries(Some(4), RetryConfig::default(), Some(signal));
        let attempts = AtomicUsize::new(0);

        let error = retries
            .retry(|| {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Ok::<(), AiMuxError>(()) }
            })
            .await
            .unwrap_err();

        assert_eq!(retries.max_retries, 4);
        assert!(matches!(error, AiMuxError::Aborted(_)));
        assert_eq!(attempts.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn per_call_max_preserves_the_model_delay_and_backoff() {
        let retries = prepare_retries(
            Some(4),
            RetryConfig {
                max_retries: 7,
                initial_delay: Duration::from_millis(123),
                backoff_factor: 3,
            },
            None,
        );

        assert_eq!(retries.max_retries, 4);
        assert_eq!(retries.initial_delay_ms, 123);
        assert_eq!(retries.backoff_factor, 3);
    }

    #[tokio::test]
    async fn first_non_retryable_error_is_not_wrapped() {
        let error = retry_with_exponential_backoff(
            || async { Err::<(), _>(AiMuxError::InvalidArgument("bad".into())) },
            2,
            0,
            2,
            None,
            should_retry,
            unchanged_delay,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AiMuxError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn zero_retries_returns_the_original_error() {
        let attempts = AtomicUsize::new(0);
        let error = retry_with_exponential_backoff(
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Err::<(), _>(api_error(true)) }
            },
            0,
            0,
            2,
            None,
            should_retry,
            unchanged_delay,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AiMuxError::ApiCall(_)));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn exhausted_attempts_preserve_the_complete_history() {
        let attempts = AtomicUsize::new(0);
        let error = retry_with_exponential_backoff(
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Err::<(), _>(api_error(true)) }
            },
            2,
            0,
            2,
            None,
            should_retry,
            unchanged_delay,
        )
        .await
        .unwrap_err();
        let AiMuxError::Retry(error) = error else {
            panic!("expected retry error")
        };
        assert_eq!(error.reason, RetryErrorReason::MaxRetriesExceeded);
        assert_eq!(error.errors.len(), 3);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn later_non_retryable_error_preserves_both_attempts() {
        let attempts = AtomicUsize::new(0);
        let error = retry_with_exponential_backoff(
            || {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                async move { Err::<(), _>(api_error(attempt == 0)) }
            },
            2,
            0,
            2,
            None,
            should_retry,
            unchanged_delay,
        )
        .await
        .unwrap_err();
        let AiMuxError::Retry(error) = error else {
            panic!("expected retry error")
        };
        assert_eq!(error.reason, RetryErrorReason::ErrorNotRetryable);
        assert_eq!(error.errors.len(), 2);
    }

    #[tokio::test]
    async fn retry_error_is_never_nested() {
        let inner = RetryError {
            reason: RetryErrorReason::MaxRetriesExceeded,
            errors: vec![api_error(true), api_error(true)],
        };
        let error = retry_with_exponential_backoff(
            || {
                let error = inner.clone();
                async move { Err::<(), _>(AiMuxError::Retry(error)) }
            },
            2,
            0,
            2,
            None,
            should_retry,
            unchanged_delay,
        )
        .await
        .unwrap_err();

        assert!(matches!(error, AiMuxError::Retry(_)));
        let AiMuxError::Retry(error) = error else {
            unreachable!()
        };
        assert!(
            error
                .errors
                .iter()
                .all(|error| !matches!(error, AiMuxError::Retry(_)))
        );
    }

    #[tokio::test]
    async fn timeout_and_abort_are_never_wrapped() {
        for source in [
            AiMuxError::Timeout("Total timeout of 1ms exceeded".into()),
            AiMuxError::Aborted("request aborted".into()),
        ] {
            let expected = source.to_string();
            let error = retry_with_exponential_backoff(
                || {
                    let source = source.clone();
                    async move { Err::<(), _>(source) }
                },
                2,
                0,
                2,
                None,
                should_retry,
                unchanged_delay,
            )
            .await
            .unwrap_err();
            assert_eq!(error.to_string(), expected);
            assert!(!matches!(error, AiMuxError::Retry(_)));
        }
    }

    #[tokio::test]
    async fn max_retries_check_precedes_final_retryability() {
        let attempts = AtomicUsize::new(0);
        let error = retry_with_exponential_backoff(
            || {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                async move { Err::<(), _>(api_error(attempt < 2)) }
            },
            2,
            0,
            2,
            None,
            should_retry,
            unchanged_delay,
        )
        .await
        .unwrap_err();
        let AiMuxError::Retry(error) = error else {
            panic!("expected retry error")
        };
        assert_eq!(error.reason, RetryErrorReason::MaxRetriesExceeded);
        assert_eq!(error.errors.len(), 3);
    }

    #[test]
    fn retry_headers_are_exact_and_only_exponential_delay_is_jittered() {
        let milliseconds = api_error_with_headers(503, &[("retry-after-ms", "1500")]);
        assert_eq!(retry_header_delay(&milliseconds, 2_000), Some(1_500));
        assert_eq!(
            retry_delay_with_jitter(&milliseconds, 2_000, |_| panic!("hint must not jitter")),
            1_500
        );

        let seconds = api_error_with_headers(503, &[("retry-after", "1.25")]);
        assert_eq!(retry_header_delay(&seconds, 2_000), Some(1_250));

        let unreasonable = api_error_with_headers(503, &[("retry-after-ms", "120000")]);
        assert_eq!(retry_header_delay(&unreasonable, 2_000), None);
        assert_eq!(retry_header_delay(&unreasonable, u64::MAX), Some(120_000));
        assert_eq!(retry_delay_with_jitter(&unreasonable, 2_000, |_| 0), 0);
        assert_eq!(
            retry_delay_with_jitter(&unreasonable, 2_000, |max| max),
            2_000
        );

        let nan_then_seconds =
            api_error_with_headers(503, &[("retry-after-ms", "NaN"), ("retry-after", "1")]);
        assert_eq!(retry_header_delay(&nan_then_seconds, 2_000), Some(1_000));
    }

    #[tokio::test]
    async fn abort_during_backoff_returns_abort_without_history() {
        let signal = AbortSignal::new();
        let abort_after_attempt = signal.clone();
        let error = retry_with_exponential_backoff(
            || async { Err::<(), _>(api_error(true)) },
            2,
            60_000,
            2,
            Some(&signal),
            move |_error: &AiMuxError| {
                abort_after_attempt.abort();
                true
            },
            unchanged_delay,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AiMuxError::Aborted(message) if message == "request aborted"));
    }

    #[tokio::test]
    async fn pre_aborted_signal_does_not_start_an_attempt() {
        let signal = AbortSignal::new();
        signal.abort();
        let attempts = AtomicUsize::new(0);

        let error = retry_with_exponential_backoff(
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Ok::<(), AiMuxError>(()) }
            },
            2,
            0,
            2,
            Some(&signal),
            should_retry,
            unchanged_delay,
        )
        .await
        .unwrap_err();

        assert!(matches!(error, AiMuxError::Aborted(message) if message == "request aborted"));
        assert_eq!(attempts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn abort_during_attempt_drops_the_attempt_future() {
        struct DropMarker(Arc<AtomicUsize>);

        impl Drop for DropMarker {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let signal = AbortSignal::new();
        let dropped = Arc::new(AtomicUsize::new(0));
        let marker = dropped.clone();
        let future = retry_with_exponential_backoff(
            move || {
                let marker = DropMarker(marker.clone());
                async move {
                    let _marker = marker;
                    std::future::pending::<Result<(), AiMuxError>>().await
                }
            },
            2,
            0,
            2,
            Some(&signal),
            should_retry,
            unchanged_delay,
        );
        tokio::pin!(future);
        assert!(matches!(
            futures::poll!(future.as_mut()),
            std::task::Poll::Pending
        ));

        signal.abort();
        let error = future.await.unwrap_err();

        assert!(matches!(error, AiMuxError::Aborted(message) if message == "request aborted"));
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn retry_error_messages_match_the_ai_sdk() {
        let max = RetryError {
            reason: RetryErrorReason::MaxRetriesExceeded,
            errors: vec![api_error(true), api_error(true), api_error(true)],
        };
        assert_eq!(
            max.to_string(),
            "Failed after 3 attempts. Last error: API call error: retry"
        );

        let non_retryable = RetryError {
            reason: RetryErrorReason::ErrorNotRetryable,
            errors: vec![api_error(true), api_error(false)],
        };
        assert_eq!(
            non_retryable.to_string(),
            "Failed after 2 attempts with non-retryable error: 'API call error: retry'"
        );
    }
}
