//! A point in time as the AI SDK's JSON writes it.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

// `0000-01-01T00:00:00.000Z ..= 9999-12-30T22:00:00.999Z` (`jiff::Timestamp::MAX` floored to ms).
const MIN_UNIX_MILLIS: i64 = -62_167_219_200_000;
const MAX_UNIX_MILLIS: i64 = 253_402_207_200_999;

/// A point in time: a `Date` in the AI SDK, whose JSON is
/// `Date.prototype.toISOString()` (`2024-03-01T12:00:00.000Z`).
///
/// It holds a UTC time at millisecond precision, as a JavaScript `Date` does,
/// in the years 0000 to 9999 that `toISOString` writes in four digits.
/// Serializing writes that form; parsing reads any RFC 3339 date-time (any
/// offset, any fraction) and fails on anything else. Parsing and writing back
/// therefore gives `new Date(text).toISOString()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, JsonSchema)]
#[schemars(with = "String")]
#[schemars(extend("x-aimux-codec" = "timestamp", "format" = "date-time"))]
pub struct Timestamp(jiff::Timestamp);

/// The text is not an RFC 3339 date-time, or its time is outside the supported
/// range (`0000-01-01T00:00:00.000Z..=9999-12-30T22:00:00.999Z`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "invalid timestamp {text:?}: expected an RFC 3339 date-time such as 2024-03-01T12:00:00.000Z"
)]
pub struct TimestampError {
    text: String,
}

impl Timestamp {
    /// The current time.
    #[must_use]
    pub fn now() -> Self {
        Self::from_timestamp(jiff::Timestamp::now()).unwrap_or(Self(jiff::Timestamp::UNIX_EPOCH))
    }

    /// The time, truncated to milliseconds toward negative infinity, or `None`
    /// before `0000-01-01T00:00:00.000Z` (note [`jiff::Timestamp::MAX`] is
    /// `9999-12-30T22:00:00.999999999Z`).
    ///
    /// `jiff::Timestamp` extends down to year `-9999`, whereas `Timestamp`
    /// enforces 4-digit years (`>= 0000`) for JS `toISOString()` output, so
    /// this conversion is fallible while [`Timestamp::to_timestamp`] (and
    /// `From<Timestamp> for jiff::Timestamp`) is infallible.
    #[must_use]
    pub fn from_timestamp(time: jiff::Timestamp) -> Option<Self> {
        let millis = i64::try_from(time.as_nanosecond().div_euclid(1_000_000)).ok()?;
        Self::from_unix_millis(millis)
    }

    /// `new Date(millis)`: milliseconds since the Unix epoch, or `None`
    /// outside `0000-01-01T00:00:00.000Z..=9999-12-30T22:00:00.999Z`.
    #[must_use]
    pub fn from_unix_millis(millis: i64) -> Option<Self> {
        if !(MIN_UNIX_MILLIS..=MAX_UNIX_MILLIS).contains(&millis) {
            return None;
        }
        let secs = millis.div_euclid(1_000);
        let nanos = (millis.rem_euclid(1_000) as i32) * 1_000_000;
        jiff::Timestamp::new(secs, nanos).ok().map(Self)
    }

    /// `new Date(seconds * 1000)`, the form providers use for a Unix
    /// `created` field: a fraction below a millisecond is dropped, and a
    /// value that is not finite (an invalid `Date` upstream) or is outside
    /// `0000-01-01T00:00:00.000Z..=9999-12-30T22:00:00.999Z` is `None`.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn from_unix_seconds(seconds: f64) -> Option<Self> {
        let millis = (seconds * 1000.0).trunc();
        if !millis.is_finite() || millis.abs() > 8.64e15 {
            return None;
        }
        Self::from_unix_millis(millis as i64)
    }

    /// `new Date(text)` for an HTTP `Date` header (RFC 7231 IMF-fixdate, as
    /// RFC 2822 reads it), or `None` when it is not one.
    #[must_use]
    pub fn from_http_date(text: &str) -> Option<Self> {
        let without_comments = strip_trailing_rfc2822_comments(text.trim())?;
        let parsed = jiff::fmt::rfc2822::DateTimeParser::new()
            .parse_timestamp(without_comments)
            .ok()
            .and_then(Self::from_timestamp)?;
        if rfc2822_has_leap_second(without_comments) {
            parsed
                .unix_millis()
                .checked_add(1_000)
                .and_then(Self::from_unix_millis)
        } else {
            Some(parsed)
        }
    }

    /// Read an RFC 3339 date-time.
    ///
    /// # Errors
    ///
    /// Fails when the text is not an RFC 3339 date-time or is outside
    /// `0000-01-01T00:00:00.000Z..=9999-12-30T22:00:00.999Z`.
    pub fn parse(text: &str) -> Result<Self, TimestampError> {
        let err = || TimestampError {
            text: text.to_owned(),
        };
        let (normalized, leap_second) = prepare_strict_rfc3339(text).ok_or_else(err)?;
        let parsed = normalized
            .parse::<jiff::Timestamp>()
            .ok()
            .and_then(Self::from_timestamp)
            .ok_or_else(err)?;
        if leap_second {
            parsed
                .unix_millis()
                .checked_add(1_000)
                .and_then(Self::from_unix_millis)
                .ok_or_else(err)
        } else {
            Ok(parsed)
        }
    }

    /// The time as a `jiff::Timestamp` (UTC, millisecond precision).
    ///
    /// Infallible because `Timestamp` is bounded to
    /// `0000-01-01T00:00:00.000Z..=9999-12-30T22:00:00.999Z`, a subset of
    /// `jiff::Timestamp`'s `-9999..=9999` range.
    #[must_use]
    pub fn to_timestamp(self) -> jiff::Timestamp {
        self.0
    }

    /// Milliseconds since the Unix epoch (`Date.prototype.getTime()`).
    #[must_use]
    pub fn unix_millis(self) -> i64 {
        self.0.as_millisecond()
    }
}

/// Validates strict RFC 3339 syntax (`YYYY-MM-DD[T|t| ]HH:MM:SS(\.\d+)?(Z|z|[+-]HH:MM)`)
/// and normalizes two valid RFC 3339 constructs before `jiff::Timestamp` parsing:
/// - `:60` leap seconds are rewritten to `:59` with `leap_second = true` so the
///   caller advances by 1,000 ms (matching `chrono`'s `timestamp_millis()`).
/// - fractional seconds longer than 9 digits are truncated to 9 digits.
fn prepare_strict_rfc3339(text: &str) -> Option<(Cow<'_, str>, bool)> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    if !bytes[0..4].iter().all(u8::is_ascii_digit)
        || bytes[4] != b'-'
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
        || bytes[7] != b'-'
        || !bytes[8..10].iter().all(u8::is_ascii_digit)
        || !matches!(bytes[10], b'T' | b't' | b' ')
        || !bytes[11..13].iter().all(u8::is_ascii_digit)
        || bytes[13] != b':'
        || !bytes[14..16].iter().all(u8::is_ascii_digit)
        || bytes[16] != b':'
        || !bytes[17..19].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let mut rest = &bytes[19..];
    let mut frac_end = 19;
    let mut truncate_frac = false;
    if let Some(after_dot) = rest.strip_prefix(b".") {
        let digits = after_dot.iter().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 {
            return None;
        }
        if digits > 9 {
            truncate_frac = true;
        }
        frac_end = 20 + digits;
        rest = &after_dot[digits..];
    }
    let valid_offset = match rest {
        b"Z" | b"z" => true,
        [b'+' | b'-', h1, h2, b':', m1, m2]
            if h1.is_ascii_digit()
                && h2.is_ascii_digit()
                && m1.is_ascii_digit()
                && m2.is_ascii_digit() =>
        {
            let offset_hour = (h1 - b'0') * 10 + (h2 - b'0');
            let offset_minute = (m1 - b'0') * 10 + (m2 - b'0');
            offset_hour <= 23 && offset_minute <= 59
        }
        _ => false,
    };
    if !valid_offset {
        return None;
    }

    let leap_second = &bytes[17..19] == b"60";
    if !leap_second && !truncate_frac {
        return Some((Cow::Borrowed(text), false));
    }

    let mut normalized = String::with_capacity(text.len());
    normalized.push_str(&text[..17]);
    normalized.push_str(if leap_second { "59" } else { &text[17..19] });
    if truncate_frac {
        normalized.push_str(&text[19..29]);
        normalized.push_str(&text[frac_end..]);
    } else {
        normalized.push_str(&text[19..]);
    }
    Some((Cow::Owned(normalized), leap_second))
}

/// Validates and strips one or more trailing RFC 2822 comments (`(...)`) from
/// `text`, because `jiff::fmt::rfc2822::DateTimeParser` consumes at most one
/// trailing comment. Returns `None` if a comment is malformed or followed by
/// non-whitespace bytes.
fn strip_trailing_rfc2822_comments(text: &str) -> Option<&str> {
    let Some(first_paren) = text.find('(') else {
        return Some(text);
    };
    let mut rest = &text.as_bytes()[first_paren..];
    while let Some(after_open) = rest.strip_prefix(b"(") {
        let mut depth = 1u32;
        let mut escaped = false;
        let mut consumed = 0usize;
        for &byte in after_open {
            consumed += 1;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'(' {
                depth = depth.checked_add(1)?;
            } else if byte == b')' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
        }
        if depth != 0 {
            return None;
        }
        rest = &after_open[consumed..];
        while let Some((&b, tail)) = rest.split_first() {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                rest = tail;
            } else {
                break;
            }
        }
    }
    rest.is_empty().then_some(&text[..first_paren])
}

fn rfc2822_has_leap_second(text: &str) -> bool {
    let stripped: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut colons = stripped.iter().enumerate().filter(|(_, b)| **b == b':');
    let _hour_colon = colons.next();
    if let Some((idx, _)) = colons.next() {
        return stripped.get(idx + 1..idx + 3) == Some(b"60")
            && !stripped.get(idx + 3).is_some_and(u8::is_ascii_digit);
    }
    false
}

impl TryFrom<jiff::Timestamp> for Timestamp {
    type Error = TimestampError;

    fn try_from(time: jiff::Timestamp) -> Result<Self, Self::Error> {
        Self::from_timestamp(time).ok_or_else(|| TimestampError {
            text: time.to_string(),
        })
    }
}

impl From<Timestamp> for jiff::Timestamp {
    fn from(timestamp: Timestamp) -> Self {
        timestamp.to_timestamp()
    }
}

impl fmt::Display for Timestamp {
    /// `toISOString()`: `YYYY-MM-DDTHH:mm:ss.sssZ`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dt = jiff::tz::Offset::UTC.to_datetime(self.0);
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            dt.year(),
            dt.month(),
            dt.day(),
            dt.hour(),
            dt.minute(),
            dt.second(),
            dt.millisecond(),
        )
    }
}

impl FromStr for Timestamp {
    type Err = TimestampError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_to_iso_string() {
        // `new Date(0).toISOString()`, `new Date(1741269019123).toISOString()`.
        assert_eq!(
            Timestamp::from_unix_millis(0).unwrap().to_string(),
            "1970-01-01T00:00:00.000Z"
        );
        assert_eq!(
            Timestamp::from_unix_millis(1_741_269_019_123)
                .unwrap()
                .to_string(),
            "2025-03-06T13:50:19.123Z"
        );
        // `new Date(-1).toISOString()`.
        assert_eq!(
            Timestamp::from_unix_millis(-1).unwrap().to_string(),
            "1969-12-31T23:59:59.999Z"
        );
    }

    #[test]
    fn unix_seconds_are_read_as_new_date_seconds_times_1000() {
        // `new Date(1741269019.9999 * 1000)` keeps whole milliseconds.
        assert_eq!(
            Timestamp::from_unix_seconds(1_741_269_019.999_9)
                .unwrap()
                .to_string(),
            "2025-03-06T13:50:19.999Z"
        );
        assert_eq!(Timestamp::from_unix_seconds(f64::NAN), None);
        assert_eq!(Timestamp::from_unix_seconds(f64::INFINITY), None);
        // Year 10000 does not fit `toISOString`'s four digits.
        assert_eq!(Timestamp::from_unix_seconds(253_402_300_800.0), None);
    }

    #[test]
    fn parses_rfc_3339_only() {
        let parsed = Timestamp::parse("2025-03-06T13:50:19.123456789-01:30").unwrap();
        assert_eq!(parsed.to_string(), "2025-03-06T15:20:19.123Z");
        assert_eq!(
            "2025-03-06T13:50:19Z"
                .parse::<Timestamp>()
                .unwrap()
                .unix_millis(),
            1_741_269_019_000
        );
        for invalid in ["", "now", "2025-03-06", "2025-03-06 13:50:19", "1741269019"] {
            assert!(Timestamp::parse(invalid).is_err(), "{invalid:?}");
        }
        assert_eq!(
            Timestamp::parse("nope").unwrap_err().to_string(),
            "invalid timestamp \"nope\": expected an RFC 3339 date-time such as 2024-03-01T12:00:00.000Z"
        );
    }

    #[test]
    fn reads_http_dates() {
        assert_eq!(
            Timestamp::from_http_date("Thu, 06 Mar 2025 13:50:19 GMT")
                .unwrap()
                .to_string(),
            "2025-03-06T13:50:19.000Z"
        );
        assert_eq!(
            Timestamp::from_http_date("Thu, 06 Mar 2025 13:50:19 +0000 (first) (second)")
                .unwrap()
                .to_string(),
            "2025-03-06T13:50:19.000Z"
        );
        assert_eq!(
            Timestamp::from_http_date(r"Thu, 06 Mar 2025 13:50:19 +0000 (a (b\)c)) (second)")
                .unwrap()
                .to_string(),
            "2025-03-06T13:50:19.000Z"
        );
        assert_eq!(Timestamp::from_http_date("yesterday"), None);
        assert_eq!(
            Timestamp::from_http_date("Thu, 06 Mar 2025 13:50:19 +0000 (first) (unclosed"),
            None
        );
        assert_eq!(
            Timestamp::from_http_date("Thu, 06 Mar 2025 13:50:19 +0000 (first) junk"),
            None
        );
        assert_eq!(
            Timestamp::from_http_date("Thu, 06 Mar 2025 13:50:19 +0000 (first))"),
            None
        );
    }

    #[test]
    fn converts_jiff_timestamp_and_truncates_pre_1970_toward_floor() {
        let jiff_ts: jiff::Timestamp = "2025-03-06T13:50:19.123456789Z".parse().unwrap();
        let ts = Timestamp::from_timestamp(jiff_ts).unwrap();
        assert_eq!(ts.unix_millis(), 1_741_269_019_123);
        assert_eq!(
            ts.to_timestamp(),
            jiff::Timestamp::from_millisecond(1_741_269_019_123).unwrap()
        );
        assert_eq!(Timestamp::try_from(jiff_ts).unwrap(), ts);
        assert_eq!(jiff::Timestamp::from(ts), ts.to_timestamp());

        // Sub-millisecond pre-1970 instants must truncate toward -inf, not 0.
        let pre_epoch: jiff::Timestamp = "1969-12-31T23:59:59.999999Z".parse().unwrap();
        let truncated = Timestamp::from_timestamp(pre_epoch).unwrap();
        assert_eq!(truncated.unix_millis(), -1);
        assert_eq!(truncated.to_string(), "1969-12-31T23:59:59.999Z");
        assert_eq!(
            Timestamp::parse("1969-12-31T23:59:59.999999Z")
                .unwrap()
                .unix_millis(),
            -1
        );
    }

    #[test]
    fn enforces_four_digit_year_bounds() {
        let min = "0000-01-01T00:00:00.000Z".parse::<Timestamp>().unwrap();
        assert_eq!(min.unix_millis(), MIN_UNIX_MILLIS);
        assert_eq!(min.to_string(), "0000-01-01T00:00:00.000Z");

        // `jiff::Timestamp::MAX` is `9999-12-30T22:00:00.999999999Z`.
        assert_eq!(
            MAX_UNIX_MILLIS,
            i64::try_from(jiff::Timestamp::MAX.as_nanosecond().div_euclid(1_000_000)).unwrap()
        );
        let max = "9999-12-30T22:00:00.999Z".parse::<Timestamp>().unwrap();
        assert_eq!(max.unix_millis(), MAX_UNIX_MILLIS);
        assert_eq!(max.to_string(), "9999-12-30T22:00:00.999Z");

        assert_eq!(Timestamp::from_unix_millis(MIN_UNIX_MILLIS).unwrap(), min);
        assert_eq!(Timestamp::from_unix_millis(MAX_UNIX_MILLIS).unwrap(), max);
        assert_eq!(
            Timestamp::from_unix_millis(MAX_UNIX_MILLIS - 1)
                .unwrap()
                .to_string(),
            "9999-12-30T22:00:00.998Z"
        );
        assert_eq!(Timestamp::from_unix_millis(MIN_UNIX_MILLIS - 1), None);
        assert_eq!(Timestamp::from_unix_millis(MAX_UNIX_MILLIS + 1), None);
        assert!(Timestamp::from_unix_seconds(253_402_207_200.999).is_some());
        assert_eq!(Timestamp::from_unix_seconds(253_402_207_201.0), None);

        let before_year_zero: jiff::Timestamp = "-000001-12-31T23:59:59Z".parse().unwrap();
        assert_eq!(Timestamp::from_timestamp(before_year_zero), None);
        assert!(Timestamp::try_from(before_year_zero).is_err());

        assert!(Timestamp::parse("-0001-12-31T23:59:59Z").is_err());
        assert!(Timestamp::parse("9999-12-30T22:00:01Z").is_err());
        assert!(Timestamp::parse("9999-12-31T23:59:59.999Z").is_err());
        assert!(Timestamp::parse("10000-01-01T00:00:00Z").is_err());
        assert!(Timestamp::parse("0000-01-01T00:00:00+01:00").is_err());
    }

    #[test]
    fn parses_long_fractions_and_leap_seconds() {
        // RFC 3339 allows any number of fractional digits (`"." 1*DIGIT`).
        assert_eq!(
            Timestamp::parse("2025-03-06T13:50:19.123456789123Z")
                .unwrap()
                .unix_millis(),
            1_741_269_019_123
        );
        assert_eq!(
            Timestamp::parse("2025-03-06T13:50:19.1234567890123-01:30")
                .unwrap()
                .to_string(),
            "2025-03-06T15:20:19.123Z"
        );

        // Leap second `:60` maps to the following second, matching `chrono` `timestamp_millis()`.
        assert_eq!(
            Timestamp::parse("2016-12-31T23:59:60Z")
                .unwrap()
                .to_string(),
            "2017-01-01T00:00:00.000Z"
        );
        assert_eq!(
            Timestamp::parse("2016-12-31T23:59:60.234Z")
                .unwrap()
                .to_string(),
            "2017-01-01T00:00:00.234Z"
        );
        assert_eq!(
            Timestamp::parse("2015-02-18T23:59:60.234567890123+05:00")
                .unwrap()
                .to_string(),
            "2015-02-18T19:00:00.234Z"
        );
        assert_eq!(
            Timestamp::from_http_date("Thu, 06 Mar 2025 13:50:60 GMT")
                .unwrap()
                .to_string(),
            "2025-03-06T13:51:00.000Z"
        );
        assert_eq!(
            Timestamp::from_http_date("Thu, 06 Mar 2025 13:50:60 GMT (13:50:59)")
                .unwrap()
                .to_string(),
            "2025-03-06T13:51:00.000Z"
        );
        assert!(Timestamp::parse("9999-12-30T22:00:60Z").is_err());
        assert!(Timestamp::parse("9999-12-30T21:59:60.999Z").is_ok());
    }

    #[test]
    fn rejects_non_rfc3339_iso_extensions() {
        // RFC 3339 §5.6 permits a space date-time separator when an offset is present.
        let space_sep: Timestamp = serde_json::from_str("\"2025-03-06 13:50:19Z\"").unwrap();
        assert_eq!(space_sep.to_string(), "2025-03-06T13:50:19.000Z");

        for invalid in [
            "2025-03-06T13:50:19Z[UTC]",
            "2025-03-06T13:50:19-05:00[America/New_York]",
            "2025-03-06T13:50:19,123Z",
            "2025-03-06T13:50:19+0200",
            "2025-03-06T13:50:19+02",
            "2025-03-06T13:50:19+24:00",
            "2025-03-06T13:50:19.Z",
            "+002025-03-06T13:50:19Z",
        ] {
            assert!(Timestamp::parse(invalid).is_err(), "{invalid:?}");
        }
    }
}
