//! The bounded stages of an asynchronous job: poll and download.
//!
//! A vendor whose API answers a submit with a job id polls that job until it
//! finishes, then downloads the result. Both stages are safe to repeat; the
//! submit is not, so nothing here (or in the callers) ever sends it a second
//! time. The budgets are fixed constants of each package: an attempt count
//! nobody can configure, and a poll interval that `providerOptions` may
//! shorten or lengthen (`pollIntervalMs`, the same key the Google Files
//! package reads). There is no call-level retry count in this module and no
//! use of Core's operation-level retry: a poll that fails with a transient error
//! (408/409/429/5xx, a connection failure) simply spends one attempt, and the
//! loop gives up when the attempts are gone.

use std::future::Future;
use std::time::Duration;

use serde_json::Value;

use aimux_core::{AbortSignal, AiMuxError};
use aimux_provider_utils::sleep_or_abort;

/// How many times a download of a finished job's result is attempted.
pub(crate) const DOWNLOAD_ATTEMPTS: u32 = 3;

/// The longest a `Retry-After` hint can stretch the wait between two attempts.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

/// What one poll saw.
pub(crate) enum PollStep<T> {
    /// The job finished; the loop returns this value.
    Ready(T),
    /// The job is still running; wait and ask again.
    Pending,
}

/// The providerOptions key that sets the poll interval of a package with no
/// upstream counterpart.
pub(crate) const POLL_INTERVAL_MS_KEY: &str = "pollIntervalMs";

/// The key `@ai-sdk/luma` and `@ai-sdk/black-forest-labs` read for the same
/// thing; those two packages read it instead of [`POLL_INTERVAL_MS_KEY`].
pub(crate) const POLL_INTERVAL_MILLIS_KEY: &str = "pollIntervalMillis";

/// Whether a providerOptions field is pacing control rather than request data.
///
/// These four keys are never forwarded to a vendor, in any package, whether or
/// not the package reads them: only the interval key of the package is read,
/// the rest (`pollTimeoutMillis`, `maxPollAttempts`) are ignored because the
/// attempt budget is a package constant.
pub(crate) fn is_poll_control_key(key: &str) -> bool {
    matches!(
        key,
        "pollIntervalMs" | "pollIntervalMillis" | "pollTimeoutMillis" | "maxPollAttempts"
    )
}

/// The poll interval a package's providerOptions namespace sets under `key`,
/// or `default_ms`.
pub(crate) fn poll_interval_ms(namespace: Option<&Value>, key: &str, default_ms: u64) -> u64 {
    namespace
        .and_then(|options| options.get(key))
        .and_then(Value::as_u64)
        .unwrap_or(default_ms)
}

fn check_aborted(abort: Option<&AbortSignal>) -> Result<(), AiMuxError> {
    match abort {
        Some(signal) if signal.is_aborted() => Err(AiMuxError::from_abort_signal(signal)),
        _ => Ok(()),
    }
}

/// Wait before the next attempt. A zero wait (tests, or a caller that wants
/// back-to-back polls) only yields: a zero-length timer would still cost a
/// full timer tick.
async fn wait(duration: Duration, abort: Option<&AbortSignal>) -> Result<(), AiMuxError> {
    if duration.is_zero() {
        tokio::task::yield_now().await;
        return check_aborted(abort);
    }
    sleep_or_abort(duration, abort).await
}

/// The wait before the next attempt: the interval, stretched to a
/// `Retry-After` hint when the failed response carried one.
fn wait_after(interval: Duration, error: Option<&AiMuxError>) -> Duration {
    let hint = error
        .and_then(AiMuxError::retry_after_hint)
        .and_then(|ms| u64::try_from(ms).ok())
        .map(Duration::from_millis)
        .map(|hint| hint.min(MAX_RETRY_AFTER));
    interval.max(hint.unwrap_or_default())
}

/// Poll until `step` reports [`PollStep::Ready`].
///
/// At most `max_attempts` polls are made. Cancellation is checked before every
/// poll and interrupts every wait. A transient error ([`AiMuxError::is_retryable`])
/// spends the attempt like a pending answer would; any other error ends the
/// loop. When the attempts are gone the error of the last poll is returned if
/// that poll failed, otherwise `AiMuxError::Timeout`.
///
/// # Errors
///
/// Returns the first non-transient error of `step`, the cancellation, the
/// last transient error once the attempts are spent, or `Timeout` when the job
/// was still pending after the last attempt.
pub(crate) async fn poll_until<T, F, Fut>(
    what: &str,
    abort: Option<&AbortSignal>,
    interval: Duration,
    max_attempts: u32,
    mut step: F,
) -> Result<T, AiMuxError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<PollStep<T>, AiMuxError>>,
{
    let mut last_error: Option<AiMuxError> = None;
    for attempt in 0..max_attempts {
        check_aborted(abort)?;
        match step().await {
            Ok(PollStep::Ready(value)) => return Ok(value),
            Ok(PollStep::Pending) => last_error = None,
            Err(error) if error.is_retryable() => last_error = Some(error),
            Err(error) => return Err(error),
        }
        if attempt + 1 < max_attempts {
            wait(wait_after(interval, last_error.as_ref()), abort).await?;
        }
    }
    match last_error {
        Some(error) => Err(error),
        None => Err(AiMuxError::Timeout(format!(
            "{what} polling timed out after {max_attempts} attempts ({}ms apart)",
            interval.as_millis()
        ))),
    }
}

/// Download a finished job's result: [`DOWNLOAD_ATTEMPTS`] tries, a transient
/// error waits `interval` and tries again, any other error ends it.
///
/// # Errors
///
/// Returns the first non-transient error, the cancellation, or the error of
/// the last attempt.
pub(crate) async fn retry_download<T, F, Fut>(
    abort: Option<&AbortSignal>,
    interval: Duration,
    mut attempt: F,
) -> Result<T, AiMuxError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, AiMuxError>>,
{
    let mut tries = 0;
    loop {
        check_aborted(abort)?;
        tries += 1;
        match attempt().await {
            Ok(value) => return Ok(value),
            Err(error) if error.is_retryable() && tries < DOWNLOAD_ATTEMPTS => {
                wait(wait_after(interval, Some(&error)), abort).await?;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use aimux_core::ApiCallError;

    use super::*;

    fn transient() -> AiMuxError {
        AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: Some(503),
            is_retryable: true,
            ..ApiCallError::new("unavailable", "http://x/job", serde_json::json!({}))
        }))
    }

    fn fatal() -> AiMuxError {
        AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: Some(400),
            ..ApiCallError::new("bad", "http://x/job", serde_json::json!({}))
        }))
    }

    #[test]
    fn interval_override_reads_the_namespace_only() {
        let options = serde_json::json!({ "pollIntervalMs": 7, "maxPollAttempts": 1 });
        assert_eq!(
            poll_interval_ms(Some(&options), POLL_INTERVAL_MS_KEY, 500),
            7
        );
        // Only the key asked for is read.
        assert_eq!(
            poll_interval_ms(Some(&options), POLL_INTERVAL_MILLIS_KEY, 500),
            500
        );
        assert_eq!(
            poll_interval_ms(Some(&serde_json::json!({})), POLL_INTERVAL_MS_KEY, 500),
            500
        );
        assert_eq!(poll_interval_ms(None, POLL_INTERVAL_MS_KEY, 500), 500);
        // A string is not a number: the default stays.
        assert_eq!(
            poll_interval_ms(
                Some(&serde_json::json!({ "pollIntervalMs": "7" })),
                POLL_INTERVAL_MS_KEY,
                500
            ),
            500
        );
    }

    #[test]
    fn the_four_pacing_keys_are_control_keys_and_nothing_else_is() {
        for key in [
            "pollIntervalMs",
            "pollIntervalMillis",
            "pollTimeoutMillis",
            "maxPollAttempts",
        ] {
            assert!(is_poll_control_key(key), "{key}");
        }
        for key in ["prompt", "maxWaitTimeInSeconds", "poll", "pollInterval"] {
            assert!(!is_poll_control_key(key), "{key}");
        }
    }

    #[tokio::test]
    async fn returns_the_first_ready_value() {
        let polls = AtomicU32::new(0);
        let value = poll_until("job", None, Duration::ZERO, 10, || async {
            Ok(if polls.fetch_add(1, Ordering::SeqCst) == 2 {
                PollStep::Ready("done")
            } else {
                PollStep::Pending
            })
        })
        .await
        .unwrap();
        assert_eq!(value, "done");
        assert_eq!(polls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_transient_error_spends_an_attempt_and_the_next_poll_can_succeed() {
        let polls = AtomicU32::new(0);
        let value = poll_until("job", None, Duration::ZERO, 10, || async {
            match polls.fetch_add(1, Ordering::SeqCst) {
                0 => Err(transient()),
                _ => Ok(PollStep::Ready(1)),
            }
        })
        .await
        .unwrap();
        assert_eq!(value, 1);
        assert_eq!(polls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_fatal_error_ends_the_loop_at_once() {
        let polls = AtomicU32::new(0);
        let error = poll_until::<(), _, _>("job", None, Duration::ZERO, 10, || async {
            polls.fetch_add(1, Ordering::SeqCst);
            Err(fatal())
        })
        .await
        .unwrap_err();
        assert!(matches!(error, AiMuxError::ApiCall(ref d) if d.status_code == Some(400)));
        assert_eq!(polls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn exhausted_attempts_return_the_last_transient_error() {
        let polls = AtomicU32::new(0);
        let error = poll_until::<(), _, _>("job", None, Duration::ZERO, 4, || async {
            polls.fetch_add(1, Ordering::SeqCst);
            Err(transient())
        })
        .await
        .unwrap_err();
        assert!(matches!(error, AiMuxError::ApiCall(ref d) if d.status_code == Some(503)));
        assert_eq!(polls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn exhausted_attempts_while_pending_are_a_timeout() {
        let polls = AtomicU32::new(0);
        let error = poll_until::<(), _, _>("job", None, Duration::ZERO, 3, || async {
            polls.fetch_add(1, Ordering::SeqCst);
            Ok(PollStep::Pending)
        })
        .await
        .unwrap_err();
        assert!(
            matches!(error, AiMuxError::Timeout(ref m) if m.contains("job") && m.contains('3'))
        );
        assert_eq!(polls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn an_aborted_signal_stops_before_the_next_poll() {
        let signal = AbortSignal::new();
        let polls = AtomicU32::new(0);
        let error = poll_until::<(), _, _>("job", Some(&signal), Duration::ZERO, 10, || async {
            polls.fetch_add(1, Ordering::SeqCst);
            signal.abort();
            Ok(PollStep::Pending)
        })
        .await
        .unwrap_err();
        assert!(matches!(error, AiMuxError::Aborted(_)));
        assert_eq!(polls.load(Ordering::SeqCst), 1);

        // Aborted before the first poll: no poll at all.
        let polls = AtomicU32::new(0);
        let error = poll_until::<(), _, _>("job", Some(&signal), Duration::ZERO, 10, || async {
            polls.fetch_add(1, Ordering::SeqCst);
            Ok(PollStep::Pending)
        })
        .await
        .unwrap_err();
        assert!(matches!(error, AiMuxError::Aborted(_)));
        assert_eq!(polls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn download_tries_a_bounded_number_of_times() {
        let tries = AtomicU32::new(0);
        let error = retry_download::<(), _, _>(None, Duration::ZERO, || async {
            tries.fetch_add(1, Ordering::SeqCst);
            Err(transient())
        })
        .await
        .unwrap_err();
        assert!(matches!(error, AiMuxError::ApiCall(_)));
        assert_eq!(tries.load(Ordering::SeqCst), DOWNLOAD_ATTEMPTS);
    }

    #[tokio::test]
    async fn download_recovers_from_a_transient_error_and_stops_on_a_fatal_one() {
        let tries = AtomicU32::new(0);
        let value = retry_download(None, Duration::ZERO, || async {
            if tries.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(transient())
            } else {
                Ok(5)
            }
        })
        .await
        .unwrap();
        assert_eq!((value, tries.load(Ordering::SeqCst)), (5, 2));

        let tries = AtomicU32::new(0);
        let error = retry_download::<(), _, _>(None, Duration::ZERO, || async {
            tries.fetch_add(1, Ordering::SeqCst);
            Err(fatal())
        })
        .await
        .unwrap_err();
        assert!(matches!(error, AiMuxError::ApiCall(ref d) if d.status_code == Some(400)));
        assert_eq!(tries.load(Ordering::SeqCst), 1);
    }
}
