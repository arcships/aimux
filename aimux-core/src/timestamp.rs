//! A point in time as the AI SDK's JSON writes it.

use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

// 0000-01-01T00:00:00.000Z ..= 9999-12-31T23:59:59.999Z
const MIN_UNIX_MILLIS: i64 = -62_167_219_200_000;
const MAX_UNIX_MILLIS: i64 = 253_402_300_799_999;

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

/// The text is not an RFC 3339 date-time, or its time is outside 0000-9999.
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
    /// outside 0000-9999.
    ///
    /// `jiff::Timestamp` spans years `-9999..=9999`, whereas `Timestamp`
    /// enforces years `0000..=9999` for 4-digit JS `toISOString()` output, so
    /// this conversion is fallible while [`Timestamp::to_timestamp`] (and
    /// `From<Timestamp> for jiff::Timestamp`) is infallible.
    #[must_use]
    pub fn from_timestamp(time: jiff::Timestamp) -> Option<Self> {
        let millis = i64::try_from(time.as_nanosecond().div_euclid(1_000_000)).ok()?;
        Self::from_unix_millis(millis)
    }

    /// `new Date(millis)`: milliseconds since the Unix epoch, or `None`
    /// outside 0000-9999.
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
    /// value that is not finite or is outside 0000-9999 is `None` (an invalid
    /// `Date` upstream).
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
        jiff::fmt::rfc2822::DateTimeParser::new()
            .parse_timestamp(text.trim())
            .ok()
            .and_then(Self::from_timestamp)
    }

    /// Read an RFC 3339 date-time.
    ///
    /// # Errors
    ///
    /// Fails when the text is not an RFC 3339 date-time or is outside
    /// 0000-9999.
    pub fn parse(text: &str) -> Result<Self, TimestampError> {
        if !is_strict_rfc3339(text) {
            return Err(TimestampError {
                text: text.to_owned(),
            });
        }
        text.parse::<jiff::Timestamp>()
            .ok()
            .and_then(Self::from_timestamp)
            .ok_or_else(|| TimestampError {
                text: text.to_owned(),
            })
    }

    /// The time as a `jiff::Timestamp` (UTC, millisecond precision).
    ///
    /// Infallible because `Timestamp` enforces years `0000..=9999`, a subset of
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

fn is_strict_rfc3339(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 20 {
        return false;
    }
    if !bytes[0..4].iter().all(u8::is_ascii_digit)
        || bytes[4] != b'-'
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
        || bytes[7] != b'-'
        || !bytes[8..10].iter().all(u8::is_ascii_digit)
        || !matches!(bytes[10], b'T' | b't')
        || !bytes[11..13].iter().all(u8::is_ascii_digit)
        || bytes[13] != b':'
        || !bytes[14..16].iter().all(u8::is_ascii_digit)
        || bytes[16] != b':'
        || !bytes[17..19].iter().all(u8::is_ascii_digit)
    {
        return false;
    }
    let mut rest = &bytes[19..];
    if let Some(after_dot) = rest.strip_prefix(b".") {
        let digits = after_dot.iter().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 {
            return false;
        }
        rest = &after_dot[digits..];
    }
    match rest {
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
    }
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
        assert_eq!(Timestamp::from_http_date("yesterday"), None);
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
        let max_jiff_ms =
            i64::try_from(jiff::Timestamp::MAX.as_nanosecond().div_euclid(1_000_000)).unwrap();
        let max = "9999-12-30T22:00:00.999Z".parse::<Timestamp>().unwrap();
        assert_eq!(max.unix_millis(), max_jiff_ms);
        assert_eq!(max.to_string(), "9999-12-30T22:00:00.999Z");

        assert_eq!(Timestamp::from_unix_millis(MIN_UNIX_MILLIS - 1), None);
        assert_eq!(Timestamp::from_unix_millis(MAX_UNIX_MILLIS + 1), None);

        let before_year_zero: jiff::Timestamp = "-000001-12-31T23:59:59Z".parse().unwrap();
        assert_eq!(Timestamp::from_timestamp(before_year_zero), None);
        assert!(Timestamp::try_from(before_year_zero).is_err());

        assert!(Timestamp::parse("-0001-12-31T23:59:59Z").is_err());
        assert!(Timestamp::parse("10000-01-01T00:00:00Z").is_err());
        assert!(Timestamp::parse("0000-01-01T00:00:00+01:00").is_err());
    }

    #[test]
    fn rejects_non_rfc3339_iso_extensions() {
        for invalid in [
            "2025-03-06T13:50:19Z[UTC]",
            "2025-03-06T13:50:19-05:00[America/New_York]",
            "2025-03-06 13:50:19Z",
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
