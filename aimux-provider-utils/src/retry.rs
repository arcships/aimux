//! Retry with exponential backoff.
//!
//! Two flavours are provided:
//! - [`retry_with_exponential_backoff`] — a plain exponential-backoff retry
//!   that retries on `AiMuxError::is_retryable()` errors.
//! - [`retry_with_exponential_backoff_respecting_retry_headers`] — the same,
//!   but consults a `retry-after` hint carried by the error (e.g. from a 429
//!   `retry-after-ms` / `retry-after` response header) and uses it in
//!   preference to the exponential delay when the hint is reasonable.
//!
//! The header-parsing and delay-selection logic is exposed as the pure helpers
//! [`parse_retry_after`] and [`get_retry_delay_ms`] so it can be unit-tested
//! independently of the async retry loop. These mirror the TS SDK's
//! `getRetryDelayInMs` / `retryWithExponentialBackoffRespectingRetryHeaders`.

use std::time::{Duration, SystemTime};

use aimux_core::AiMuxError;
use rand::Rng;

/// Retry a fallible async operation with exponential backoff.
///
/// - `max_retries`: maximum number of retry attempts (0 = no retry).
/// - `initial_delay`: initial delay between retries (doubled each time).
/// - Only retries on `AiMuxError::is_retryable()` errors.
///
/// # Errors
///
/// Returns the operation's last error once retries are exhausted (or
/// immediately for non-retryable errors).
pub async fn retry_with_exponential_backoff<F, T>(
    max_retries: u32,
    initial_delay: Duration,
    mut f: F,
) -> Result<T, AiMuxError>
where
    F: FnMut()
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, AiMuxError>> + Send>>,
    T: Send,
{
    let mut last_error = AiMuxError::Other("no attempts made".to_string());
    let mut delay = initial_delay;

    for attempt in 0..=max_retries {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                last_error = e;
                if !last_error.is_retryable() || attempt == max_retries {
                    return Err(last_error);
                }
                tokio::time::sleep(delay).await;
                delay = delay.saturating_mul(2);
            }
        }
    }

    Err(last_error)
}

/// Configuration for [`retry_with_exponential_backoff_respecting_retry_headers`].
///
/// Mirrors the TS options `{ maxRetries, initialDelayInMs, backoffFactor }`.
#[derive(Debug, Clone, Copy)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (0 = no retry). Default 2.
    pub max_retries: u32,
    /// Initial delay between retries. Default 2000ms.
    pub initial_delay: Duration,
    /// Backoff factor applied to the delay after each retry. Default 2.
    pub backoff_factor: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 2,
            initial_delay: Duration::from_millis(2000),
            backoff_factor: 2,
        }
    }
}

/// Retry a fallible async operation with exponential backoff, respecting
/// `retry-after` hints carried by the error when they are reasonable
/// (0 <= delay < 60s, or shorter than the exponential backoff would be).
///
/// Only retries on `AiMuxError::is_retryable()` errors. The delay for each
/// retry is chosen by [`get_retry_delay_ms`], fed from
/// [`AiMuxError::retry_after_hint`].
///
/// # Errors
///
/// Returns the operation's last error once retries are exhausted (or
/// immediately for non-retryable errors).
pub async fn retry_with_exponential_backoff_respecting_retry_headers<F, T>(
    config: RetryConfig,
    mut f: F,
) -> Result<T, AiMuxError>
where
    F: FnMut()
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, AiMuxError>> + Send>>,
    T: Send,
{
    let mut last_error = AiMuxError::Other("no attempts made".to_string());
    let mut exponential_delay_ms = config.initial_delay.as_millis() as i64;

    for attempt in 0..=config.max_retries {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                last_error = e;
                if !last_error.is_retryable() || attempt == config.max_retries {
                    return Err(last_error);
                }

                let hint = last_error.retry_after_hint();
                let delay_ms = {
                    let mut rng = rand::thread_rng();
                    get_retry_delay_ms_with_jitter(hint, exponential_delay_ms, &mut rng)
                };
                tokio::time::sleep(Duration::from_millis(delay_ms.max(0) as u64)).await;

                exponential_delay_ms =
                    exponential_delay_ms.saturating_mul(config.backoff_factor as i64);
            }
        }
    }

    Err(last_error)
}

/// Choose the retry delay (in ms) given an optional `retry-after` hint and the
/// current exponential-backoff delay.
///
/// Mirrors the TS `getRetryDelayInMs` "reasonable delay" check: the hint is
/// used when it is present, non-negative, and either shorter than 60 seconds
/// or shorter than the exponential backoff would be. Otherwise the exponential
/// backoff delay is used.
#[must_use]
pub fn get_retry_delay_ms(hint: Option<i64>, exponential_delay_ms: i64) -> i64 {
    match hint {
        // Use the hint when it is non-negative AND (shorter than 60s OR shorter
        // than the exponential backoff would be). Otherwise fall back.
        Some(ms) if ms >= 0 && (ms < 60_000 || ms < exponential_delay_ms) => ms,
        _ => exponential_delay_ms,
    }
}

/// 在 [`get_retry_delay_ms`] 基础上叠加 Full Jitter（参考 catcher
/// `DecorrelatedJitter`，即 AWS Full Jitter）。
///
/// `delay = random(0, base)`，其中 `base` 仍优先采用 `retry-after` hint，
/// 回退指数退避。防并发 429 惊群，且不丢 retry-after 语义（RFC-0009 §4.2）。
///
/// `base <= 0` 时返回 0（`gen_range(0..0)` 会 panic，此处提前保护）。
pub fn get_retry_delay_ms_with_jitter(
    hint: Option<i64>,
    exponential_delay_ms: i64,
    rng: &mut impl Rng,
) -> i64 {
    let base = get_retry_delay_ms(hint, exponential_delay_ms);
    if base <= 0 {
        return 0;
    }
    rng.gen_range(0..base)
}

/// Parse a `retry-after` hint (in milliseconds) from response header values.
///
/// `retry_after_ms_header` is the more precise `retry-after-ms` header (used by
/// e.g. OpenAI); `retry_after_header` is the standard `retry-after` header,
/// which may be either a number of seconds or an HTTP-date. `retry-after-ms`
/// takes precedence when both are present and parseable.
///
/// `now` is the reference instant used to compute the delay for HTTP-date
/// values. Returns `None` when no header is present or none parse to a usable
/// value. Mirrors the TS `getRetryDelayInMs` header-reading branch.
#[must_use]
pub fn parse_retry_after(
    retry_after_ms_header: Option<&str>,
    retry_after_header: Option<&str>,
    now: SystemTime,
) -> Option<i64> {
    let mut ms: Option<i64> = None;

    // retry-after-ms is more precise than retry-after and used by e.g. OpenAI.
    if let Some(raw) = retry_after_ms_header
        && let Ok(v) = raw.trim().parse::<f64>()
        && v.is_finite()
    {
        ms = Some(v as i64);
    }

    // About the Retry-After header:
    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Retry-After
    if ms.is_none()
        && let Some(raw) = retry_after_header
    {
        let trimmed = raw.trim();
        // First try to parse as a number of seconds.
        if let Ok(seconds) = trimmed.parse::<f64>() {
            if seconds.is_finite() {
                ms = Some((seconds * 1000.0) as i64);
            }
        } else {
            // Otherwise try to parse as an HTTP date.
            if let Ok(target) = httpdate::parse_http_date(trimmed) {
                if let Ok(duration) = target.duration_since(now) {
                    ms = Some(duration.as_millis() as i64);
                } else if let Ok(duration) = now.duration_since(target) {
                    // Date is in the past → non-positive delay.
                    ms = Some(-(duration.as_millis() as i64));
                }
            }
        }
    }

    ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_delay_uses_hint_when_reasonable() {
        assert_eq!(get_retry_delay_ms(Some(3000), 2000), 3000);
    }

    #[test]
    fn get_delay_uses_exponential_when_hint_too_long() {
        assert_eq!(get_retry_delay_ms(Some(70_000), 2000), 2000);
    }

    #[test]
    fn get_delay_uses_exponential_when_hint_negative() {
        assert_eq!(get_retry_delay_ms(Some(-1000), 2000), 2000);
    }

    #[test]
    fn get_delay_uses_exponential_when_no_hint() {
        assert_eq!(get_retry_delay_ms(None, 2000), 2000);
    }

    #[test]
    fn get_delay_uses_hint_when_shorter_than_exponential_even_if_over_60s() {
        // 70000ms hint but exponential is even larger → hint wins.
        assert_eq!(get_retry_delay_ms(Some(70_000), 80_000), 70_000);
    }

    #[test]
    fn jitter_returns_zero_when_base_is_zero() {
        // base == 0 → must return 0 (gen_range(0..0) would panic).
        let mut rng = rand::thread_rng();
        assert_eq!(get_retry_delay_ms_with_jitter(None, 0, &mut rng), 0);
    }

    #[test]
    fn jitter_returns_zero_when_base_negative() {
        let mut rng = rand::thread_rng();
        assert_eq!(get_retry_delay_ms_with_jitter(None, -100, &mut rng), 0);
    }

    #[test]
    fn jitter_stays_within_full_jitter_bounds() {
        // Full Jitter: delay ∈ [0, base). base here = exponential 2000 (no hint).
        let mut rng = rand::thread_rng();
        for _ in 0..1000 {
            let d = get_retry_delay_ms_with_jitter(None, 2000, &mut rng);
            assert!((0..2000).contains(&d), "delay {d} out of [0, 2000)");
        }
    }

    #[test]
    fn jitter_uses_retry_after_hint_as_upper_bound() {
        // hint 3000 → base 3000 → delay ∈ [0, 3000).
        let mut rng = rand::thread_rng();
        for _ in 0..1000 {
            let d = get_retry_delay_ms_with_jitter(Some(3000), 2000, &mut rng);
            assert!((0..3000).contains(&d), "delay {d} out of [0, 3000)");
        }
    }

    #[test]
    fn parse_retry_after_ms_header() {
        assert_eq!(
            parse_retry_after(Some("3000"), None, SystemTime::now()),
            Some(3000)
        );
    }

    #[test]
    fn parse_retry_after_seconds_header() {
        assert_eq!(
            parse_retry_after(None, Some("5"), SystemTime::now()),
            Some(5000)
        );
    }

    #[test]
    fn parse_retry_after_prefers_ms_over_seconds() {
        assert_eq!(
            parse_retry_after(Some("3000"), Some("10"), SystemTime::now()),
            Some(3000)
        );
    }

    #[test]
    fn parse_retry_after_invalid_falls_back_to_none() {
        assert_eq!(
            parse_retry_after(Some("invalid"), Some("not-a-number"), SystemTime::now()),
            None
        );
    }

    #[test]
    fn parse_retry_after_negative_ms() {
        assert_eq!(
            parse_retry_after(Some("-1000"), None, SystemTime::now()),
            Some(-1000)
        );
    }
}
