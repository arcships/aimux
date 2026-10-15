use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use aimux_core::error::{AiMuxError, ApiCallError, RetryError, RetryErrorReason};
use aimux_ops::retry_hint::retry_after_ms;

fn now() -> SystemTime {
    // Wed, 21 Oct 2015 07:28:00 GMT
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_445_412_480)
}

fn headers(values: &[(&str, &str)]) -> HashMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).into(), (*value).into()))
        .collect()
}

#[test]
fn numeric_golden_vectors() {
    let cases = [
        ("250", Some(250)),
        ("250.999", Some(250)),
        ("0.999", Some(0)),
        ("0", Some(0)),
        ("-0", Some(0)),
        (" 250.5 ", Some(250)),
        ("1e3", Some(1_000)),
        ("-1", None),
        ("-0.1", None),
        ("NaN", None),
        ("Infinity", None),
        ("-Infinity", None),
        ("inf", None),
        ("1e308", None),
        ("18446744073709551616", None),
        ("18446744073709551615", None),
        // The largest f64 below 2^64 is valid; the endpoint itself is not.
        ("18446744073709549568", Some(18_446_744_073_709_549_568)),
        ("", None),
        ("garbage", None),
        ("12ms", None),
    ];
    for (value, expected) in cases {
        assert_eq!(
            retry_after_ms(&headers(&[("retry-after-ms", value)]), now()),
            expected,
            "retry-after-ms: {value:?}"
        );
    }
}

#[test]
fn seconds_golden_vectors() {
    for (value, expected) in [
        ("1.5", Some(1_500)),
        ("1.2349", Some(1_234)),
        ("0.0009", Some(0)),
        ("0", Some(0)),
        ("-0.0001", None),
        ("-1", None),
        ("NaN", None),
        ("Infinity", None),
        ("-Infinity", None),
        ("1e308", None),
        ("18446744073709552", None),
        ("1.5 seconds", None),
        ("invalid", None),
    ] {
        assert_eq!(
            retry_after_ms(&headers(&[("retry-after", value)]), now()),
            expected,
            "retry-after: {value:?}"
        );
    }
}

#[test]
fn header_precedence_golden_vectors() {
    for (ms, seconds, expected) in [
        ("250", "10", Some(250)),
        ("0", "10", Some(0)),
        ("-1", "10", None),
        ("-0.1", "10", None),
        ("Infinity", "10", None),
        ("1e308", "10", None),
        ("invalid", "1.5", Some(1_500)),
        ("NaN", "1.5", None),
        ("", "1.5", Some(1_500)),
        ("invalid", "Wed, 21 Oct 2015 07:28:02 GMT", Some(2_000)),
    ] {
        assert_eq!(
            retry_after_ms(
                &headers(&[("retry-after-ms", ms), ("retry-after", seconds)]),
                now(),
            ),
            expected,
            "retry-after-ms: {ms:?}, retry-after: {seconds:?}"
        );
    }
}

#[test]
fn fixed_clock_http_date_golden_vectors() {
    for (date, clock, expected) in [
        ("Wed, 21 Oct 2015 07:28:02 GMT", now(), Some(2_000)),
        (
            "Wed, 21 Oct 2015 07:28:02 GMT",
            now() + Duration::from_micros(1_234_567),
            Some(765),
        ),
        ("Wed, 21 Oct 2015 07:28:00 GMT", now(), Some(0)),
        ("Wed, 21 Oct 2015 07:27:59 GMT", now(), None),
        (
            "Wed, 21 Oct 2015 07:28:00 GMT",
            now() + Duration::from_nanos(1),
            None,
        ),
        ("not a date", now(), None),
    ] {
        assert_eq!(
            retry_after_ms(&headers(&[("retry-after", date)]), clock),
            expected,
            "retry-after: {date:?}, clock: {clock:?}"
        );
    }
}

#[test]
fn header_names_are_case_insensitive_and_missing_headers_have_no_hint() {
    assert_eq!(retry_after_ms(&HashMap::new(), now()), None);
    assert_eq!(retry_after_ms(&headers(&[("other", "5")]), now()), None);
    assert_eq!(
        retry_after_ms(&headers(&[("ReTrY-AfTeR-Ms", "250")]), now()),
        Some(250)
    );
    assert_eq!(
        retry_after_ms(&headers(&[("Retry-After", "1.5")]), now()),
        Some(1_500)
    );
    assert_eq!(
        retry_after_ms(
            &headers(&[("RETRY-AFTER-MS", "250"), ("Retry-After", "10")]),
            now(),
        ),
        Some(250)
    );
}

fn api_error(values: &[(&str, &str)]) -> AiMuxError {
    let mut api = ApiCallError::new("rate limited", "https://example.com", Default::default());
    api.response_headers = Some(headers(values));
    AiMuxError::ApiCall(Box::new(api))
}

#[test]
fn core_error_uses_shared_parser_with_checked_legacy_return_type() {
    assert_eq!(
        api_error(&[("Retry-After", "1.5")]).retry_after_hint(),
        Some(1_500)
    );
    assert_eq!(
        api_error(&[("retry-after-ms", "-0.1")]).retry_after_hint(),
        None
    );
    assert_eq!(
        api_error(&[("retry-after-ms", "9223372036854775808")]).retry_after_hint(),
        None
    );
    assert_eq!(
        AiMuxError::Timeout("budget spent".into()).retry_after_hint(),
        None
    );
}

#[test]
fn retry_nesting_has_no_top_level_hint_and_retains_attempt_hints() {
    let attempt = api_error(&[("retry-after-ms", "250")]);
    let nested = AiMuxError::Retry(RetryError {
        reason: RetryErrorReason::MaxRetriesExceeded,
        errors: vec![attempt.clone()],
    });
    let aggregate = AiMuxError::Retry(RetryError {
        reason: RetryErrorReason::MaxRetriesExceeded,
        errors: vec![attempt, nested],
    });
    assert_eq!(aggregate.retry_after_hint(), None);
    let AiMuxError::Retry(retry) = &aggregate else {
        panic!("expected retry aggregate");
    };
    assert_eq!(retry.errors[0].retry_after_hint(), Some(250));
    assert_eq!(retry.errors[1].retry_after_hint(), None);
    let AiMuxError::Retry(nested) = &retry.errors[1] else {
        panic!("expected nested retry aggregate");
    };
    assert_eq!(nested.errors[0].retry_after_hint(), Some(250));
}
