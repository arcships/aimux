//! Rust translation of
//! `packages/ai/src/util/retry-with-exponential-backoff.test.ts`.
//!
//! The TS suite exercises `retryWithExponentialBackoffRespectingRetryHeaders`,
//! which retries on retryable errors and chooses each retry's delay from the
//! error's `retry-after-ms` / `retry-after` response headers when reasonable,
//! falling back to exponential backoff otherwise.
//!
//! In Rust, the equivalent is
//! [`aimux_provider_utils::retry_with_exponential_backoff_respecting_retry_headers`].
//! The header-parsing and delay-selection logic lives in the pure helpers
//! [`aimux_provider_utils::parse_retry_after`] and
//! [`aimux_provider_utils::get_retry_delay_ms`]; the end-to-end timing behaviour
//! is driven through the async retry loop with a paused tokio clock
//! (`#[tokio::test(start_paused = true)]` + `tokio::time::advance`).
//!
//! The TS tests are written against `APICallError` carrying `responseHeaders`.
//! Rust's rate-limit error carries the parsed hint in the detail's
//! delay hint (exposed via `AiMuxError::retry_after_hint()`), so a
//! 429 `Provider` error stands in for an `APICallError` with a `retry-after-ms`
//! header. Errors without a hint (e.g. `ApiCall`, `Http`) stand in for
//! `APICallError` with no retry headers.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use aimux_core::{AiMuxError, ApiCallError};

/// A retryable transient failure: a 5xx `ApiCall` error. `is_retryable` is
/// stored at construction, as `error_for_status` does for a live 5xx.
fn server_error(msg: &str) -> AiMuxError {
    AiMuxError::ApiCall(ApiCallError {
        status_code: Some(500),
        message: msg.into(),
        is_retryable: true,
        ..Default::default()
    })
}

fn rate_limited(ms: u64) -> AiMuxError {
    AiMuxError::ApiCall(ApiCallError {
        status_code: Some(429),
        retry_after_ms: Some(ms),
        is_retryable: true,
        ..Default::default()
    })
}
use aimux_provider_utils::{
    RetryConfig, get_retry_delay_ms, parse_retry_after,
    retry_with_exponential_backoff_respecting_retry_headers,
};

/// Helper: build a boxed future-producing closure that counts attempts via a
/// shared counter and fails on the attempt numbers for which `fail_with`
/// returns `Some(error)`.
#[allow(clippy::type_complexity)]
fn failing_then_success(
    counter: Arc<AtomicU32>,
    fail_with: impl Fn(u32) -> Option<AiMuxError> + Send + Sync + 'static,
) -> Box<
    dyn FnMut() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, AiMuxError>> + Send>,
        > + Send,
> {
    // Erase to a shared closure so each spawned future can hold its own clone.
    let fail_with: Arc<dyn Fn(u32) -> Option<AiMuxError> + Send + Sync> = Arc::new(fail_with);
    Box::new(move || {
        let counter = counter.clone();
        let fail_with = fail_with.clone();
        Box::pin(async move {
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(err) = fail_with(n) {
                Err(err)
            } else {
                Ok("success".to_string())
            }
        })
    })
}

/// Advance paused time and yield so the spawned retry task can process woken
/// timers and make progress.
async fn advance_and_yield(dur: Duration) {
    tokio::time::advance(dur).await;
    // Let the spawned task run any woken timers to completion.
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
}

// ===========================================================================
// Pure helper tests — mirror the TS `getRetryDelayInMs` edge cases that the
// TS suite asserts on indirectly through time advances.
// ===========================================================================

#[test]
fn get_delay_uses_hint_when_reasonable() {
    // TS: "should use rate limit header delay when present and reasonable"
    assert_eq!(get_retry_delay_ms(Some(3000), 2000), 3000);
}

#[test]
fn get_delay_uses_exponential_when_hint_too_long() {
    // TS: "should use exponential backoff when rate limit delay is too long"
    // (retry-after-ms 70000 is >= 60s and >= the 2000ms exponential delay)
    assert_eq!(get_retry_delay_ms(Some(70_000), 2000), 2000);
}

#[test]
fn get_delay_falls_back_when_negative() {
    // TS: "should fall back to exponential backoff when rate limit delay is
    // negative". The detail stores the delay as u64 so it cannot carry a
    // negative value end-to-end; the negative branch is covered here via the
    // pure helper that the retry loop uses.
    assert_eq!(get_retry_delay_ms(Some(-1000), 2000), 2000);
}

#[test]
fn get_delay_falls_back_when_no_hint() {
    // TS: "should fall back to exponential backoff when no rate limit headers"
    assert_eq!(get_retry_delay_ms(None, 2000), 2000);
}

#[test]
fn parse_retry_after_ms_header_value() {
    // TS: "should use rate limit header delay when present and reasonable"
    assert_eq!(
        parse_retry_after(Some("3000"), None, SystemTime::now()),
        Some(3000)
    );
}

#[test]
fn parse_retry_after_seconds_header() {
    // TS: "should parse retry-after header in seconds" (5s -> 5000ms)
    assert_eq!(
        parse_retry_after(None, Some("5"), SystemTime::now()),
        Some(5000)
    );
}

#[test]
fn parse_retry_after_prefers_ms_over_seconds() {
    // TS: "should prefer retry-after-ms over retry-after when both present"
    assert_eq!(
        parse_retry_after(Some("3000"), Some("10"), SystemTime::now()),
        Some(3000)
    );
}

#[test]
fn parse_retry_after_invalid_headers_yield_none() {
    // TS: "should handle invalid rate limit header values"
    assert_eq!(
        parse_retry_after(Some("invalid"), Some("not-a-number"), SystemTime::now()),
        None
    );
}

#[test]
fn parse_retry_after_negative_ms() {
    // TS: "should fall back to exponential backoff when rate limit delay is
    // negative" — the parse step still surfaces the negative value; the
    // "reasonable" clamp happens in get_retry_delay_ms (covered above).
    assert_eq!(
        parse_retry_after(Some("-1000"), None, SystemTime::now()),
        Some(-1000)
    );
}

#[test]
fn parse_retry_after_openai_seconds_header() {
    // TS: "should handle OpenAI 429 response with retry-after header" (30s)
    assert_eq!(
        parse_retry_after(None, Some("30"), SystemTime::now()),
        Some(30_000)
    );
}

#[test]
fn parse_retry_after_http_date() {
    // TS: "should handle retry-after header with HTTP date format"
    let now = SystemTime::UNIX_EPOCH + Duration::from_millis(1_700_000_000_000);
    let target = now + Duration::from_millis(5000);
    let date_header = httpdate::fmt_http_date(target);
    assert_eq!(parse_retry_after(None, Some(&date_header), now), Some(5000));
}

// ===========================================================================
// End-to-end retry tests — mirror the TS timing assertions via a paused clock.
// ===========================================================================

#[tokio::test(start_paused = true)]
async fn uses_rate_limit_header_delay_when_reasonable() {
    // TS: "should use rate limit header delay when present and reasonable"
    // retry-after-ms 3000 -> the retry must wait ~3000ms, not the 2000ms
    // exponential default.
    let counter = Arc::new(AtomicU32::new(0));
    let closure = failing_then_success(counter.clone(), |n| {
        if n == 1 {
            Some(rate_limited(3000))
        } else {
            None
        }
    });

    let handle = tokio::spawn(retry_with_exponential_backoff_respecting_retry_headers(
        RetryConfig::default(),
        closure,
    ));
    tokio::task::yield_now().await; // run attempt 1, park on 3000ms sleep

    // Full Jitter (RFC-0009 §4.2): delay ∈ [0, 3000). Advancing past 3000ms
    // guarantees the retry fired; it may fire earlier with jitter.
    advance_and_yield(Duration::from_millis(3000)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    let result = handle.await.unwrap().unwrap();
    assert_eq!(result, "success");
}

#[tokio::test(start_paused = true)]
async fn uses_exponential_backoff_when_delay_too_long() {
    // TS: "should use exponential backoff when rate limit delay is too long"
    // retry-after-ms 70000 (>= 60s) -> fall back to the 2000ms exponential delay.
    let counter = Arc::new(AtomicU32::new(0));
    let closure = failing_then_success(counter.clone(), |n| {
        if n == 1 {
            Some(rate_limited(70_000))
        } else {
            None
        }
    });

    let config = RetryConfig {
        initial_delay: Duration::from_millis(2000),
        ..RetryConfig::default()
    };
    let handle = tokio::spawn(retry_with_exponential_backoff_respecting_retry_headers(
        config, closure,
    ));
    tokio::task::yield_now().await;

    // Full Jitter: delay ∈ [0, 2000). Advancing past 2000ms guarantees the retry.
    advance_and_yield(Duration::from_millis(2000)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    assert_eq!(handle.await.unwrap().unwrap(), "success");
}

#[tokio::test(start_paused = true)]
async fn falls_back_to_exponential_when_no_rate_limit_headers() {
    // TS: "should fall back to exponential backoff when no rate limit headers"
    // A retryable error with no retry-after hint uses the 2000ms exponential delay.
    let counter = Arc::new(AtomicU32::new(0));
    let closure = failing_then_success(counter.clone(), |n| {
        if n == 1 {
            Some(server_error("Temporary error"))
        } else {
            None
        }
    });

    let config = RetryConfig {
        initial_delay: Duration::from_millis(2000),
        ..RetryConfig::default()
    };
    let handle = tokio::spawn(retry_with_exponential_backoff_respecting_retry_headers(
        config, closure,
    ));
    tokio::task::yield_now().await;

    // Full Jitter (RFC-0009 §4.2): delay ∈ [0, 2000). Advancing past 2000ms
    // guarantees the retry fired; it may fire earlier with jitter.
    advance_and_yield(Duration::from_millis(2000)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    assert_eq!(handle.await.unwrap().unwrap(), "success");
}

#[tokio::test(start_paused = true)]
async fn handles_anthropic_429_with_retry_after_ms() {
    // TS: "should handle Anthropic 429 response with retry-after-ms header"
    let counter = Arc::new(AtomicU32::new(0));
    let closure = failing_then_success(counter.clone(), |n| {
        if n == 1 {
            Some(rate_limited(5000))
        } else {
            None
        }
    });

    let handle = tokio::spawn(retry_with_exponential_backoff_respecting_retry_headers(
        RetryConfig::default(),
        closure,
    ));
    tokio::task::yield_now().await;

    // Full Jitter: delay ∈ [0, 5000). Advancing past 5000ms guarantees the retry.
    advance_and_yield(Duration::from_millis(5000)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    assert_eq!(handle.await.unwrap().unwrap(), "success");
}

#[tokio::test(start_paused = true)]
async fn multiple_retries_with_exponential_progression() {
    // TS: "should handle multiple retries with exponential backoff progression"
    // attempt 1 -> retry-after-ms 5000 (header wins, 5000 < 60s)
    // attempt 2 -> retry-after-ms 2000 (header wins, 2000 < 60s); the
    // exponential delay would be 4000ms by then, but the 2000ms header still
    // wins per get_retry_delay_ms. The TS test advances 4000ms for the second
    // retry, which covers the 2000ms delay.
    let counter = Arc::new(AtomicU32::new(0));
    let closure = failing_then_success(counter.clone(), |n| match n {
        1 => Some(rate_limited(5000)),
        2 => Some(rate_limited(2000)),
        _ => None,
    });

    let config = RetryConfig {
        max_retries: 3,
        ..RetryConfig::default()
    };
    let handle = tokio::spawn(retry_with_exponential_backoff_respecting_retry_headers(
        config, closure,
    ));
    tokio::task::yield_now().await;

    // First retry uses the 5000ms header delay.
    advance_and_yield(Duration::from_millis(5000)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    // Second retry: header delay is 2000ms; advancing 4000ms covers it.
    advance_and_yield(Duration::from_millis(4000)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 3);

    assert_eq!(handle.await.unwrap().unwrap(), "success");
}

#[tokio::test(start_paused = true)]
async fn retries_on_gateway_internal_server_error() {
    // TS: "should retry on GatewayInternalServerError" — a retryable 5xx-style
    // error with no retry-after hint uses exponential backoff.
    let counter = Arc::new(AtomicU32::new(0));
    let closure = failing_then_success(counter.clone(), |n| {
        if n == 1 {
            Some(server_error("Internal server error"))
        } else {
            None
        }
    });

    let config = RetryConfig {
        initial_delay: Duration::from_millis(2000),
        ..RetryConfig::default()
    };
    let handle = tokio::spawn(retry_with_exponential_backoff_respecting_retry_headers(
        config, closure,
    ));
    tokio::task::yield_now().await;

    // Full Jitter (RFC-0009 §4.2): delay ∈ [0, 2000). Advancing past 2000ms
    // guarantees the retry fired; it may fire earlier with jitter.
    advance_and_yield(Duration::from_millis(2000)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    assert_eq!(handle.await.unwrap().unwrap(), "success");
}

#[tokio::test(start_paused = true)]
async fn retries_on_gateway_rate_limit_error() {
    // TS: "should retry on GatewayRateLimitError" — a retryable rate-limit
    // error. Modelled as a 429 Provider error carrying a reasonable hint.
    let counter = Arc::new(AtomicU32::new(0));
    let closure = failing_then_success(counter.clone(), |n| {
        if n == 1 {
            Some(rate_limited(2000))
        } else {
            None
        }
    });

    let config = RetryConfig {
        initial_delay: Duration::from_millis(2000),
        ..RetryConfig::default()
    };
    let handle = tokio::spawn(retry_with_exponential_backoff_respecting_retry_headers(
        config, closure,
    ));
    tokio::task::yield_now().await;

    // Full Jitter (RFC-0009 §4.2): delay ∈ [0, 2000). Advancing past 2000ms
    // guarantees the retry fired; it may fire earlier with jitter.
    advance_and_yield(Duration::from_millis(2000)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    assert_eq!(handle.await.unwrap().unwrap(), "success");
}

#[tokio::test(start_paused = true)]
async fn does_not_retry_on_non_retryable_auth_error() {
    // TS: "should not retry on non-retryable GatewayAuthenticationError"
    let counter = Arc::new(AtomicU32::new(0));
    let closure = failing_then_success(counter.clone(), |_| {
        Some(AiMuxError::ApiCall(ApiCallError {
            status_code: Some(401),
            message: "Invalid API key".into(),
            ..Default::default()
        }))
    });

    let result =
        retry_with_exponential_backoff_respecting_retry_headers(RetryConfig::default(), closure)
            .await;

    // Auth is not retryable: a single attempt, no delay, error propagates.
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    let err = result.unwrap_err();
    assert!(matches!(err, AiMuxError::ApiCall(ref m) if m.message == "Invalid API key"));
}

#[tokio::test(start_paused = true)]
async fn uses_retry_after_hint_from_wrapped_error() {
    // TS: "should use retry-after headers from APICallError cause" — the TS
    // unwraps a GatewayInternalServerError's `cause` (an APICallError with
    // retry-after-ms) to find the hint. Rust's flat `AiMuxError` has no cause
    // chain, so the hint is modelled directly on the outer error (a
    // `RateLimited` carrying the 3000ms hint).
    let counter = Arc::new(AtomicU32::new(0));
    let closure = failing_then_success(counter.clone(), |n| {
        if n == 1 {
            Some(rate_limited(3000))
        } else {
            None
        }
    });

    let handle = tokio::spawn(retry_with_exponential_backoff_respecting_retry_headers(
        RetryConfig::default(),
        closure,
    ));
    tokio::task::yield_now().await;

    // Full Jitter: delay ∈ [0, 3000). Advancing past 3000ms guarantees the retry.
    advance_and_yield(Duration::from_millis(3000)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    assert_eq!(handle.await.unwrap().unwrap(), "success");
}

#[tokio::test(start_paused = true)]
async fn succeeds_on_first_attempt_without_delay() {
    // Sanity check (no direct TS counterpart, but guards the happy path): a
    // function that succeeds immediately must not sleep.
    let counter = Arc::new(AtomicU32::new(0));
    let closure = failing_then_success(counter.clone(), |_| None);

    let result =
        retry_with_exponential_backoff_respecting_retry_headers(RetryConfig::default(), closure)
            .await;
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert_eq!(result.unwrap(), "success");
}

#[tokio::test(start_paused = true)]
async fn gives_up_after_max_retries() {
    // TS counterpart: the suite's `maxRetries` config bounds attempts. A
    // persistently-retryable error must surface after `max_retries + 1` tries.
    let counter = Arc::new(AtomicU32::new(0));
    let closure = failing_then_success(counter.clone(), |_| Some(server_error("always fails")));

    let config = RetryConfig {
        max_retries: 2,
        initial_delay: Duration::from_millis(10),
        backoff_factor: 2,
    };
    let handle = tokio::spawn(retry_with_exponential_backoff_respecting_retry_headers(
        config, closure,
    ));

    // max_retries=2 -> 3 attempts total (1 + 2 retries).
    // Delays: 10ms, 20ms. Advance past both.
    advance_and_yield(Duration::from_millis(10)).await;
    advance_and_yield(Duration::from_millis(20)).await;
    advance_and_yield(Duration::from_millis(50)).await;

    let result = handle.await.unwrap();
    assert!(result.is_err());
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}
