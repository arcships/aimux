//! A point in time as the AI SDK's JSON writes it.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Datelike, SecondsFormat, TimeZone, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
pub struct Timestamp(DateTime<Utc>);

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
        Self::from_datetime(Utc::now()).unwrap_or(Self(DateTime::UNIX_EPOCH))
    }

    /// The time, truncated to milliseconds, or `None` outside 0000-9999.
    #[must_use]
    pub fn from_datetime<Tz: TimeZone>(time: DateTime<Tz>) -> Option<Self> {
        let time = time.with_timezone(&Utc);
        if !(0..=9999).contains(&time.year()) {
            return None;
        }
        Self::from_unix_millis(time.timestamp_millis())
    }

    /// `new Date(millis)`: milliseconds since the Unix epoch, or `None`
    /// outside 0000-9999.
    #[must_use]
    pub fn from_unix_millis(millis: i64) -> Option<Self> {
        let time = DateTime::from_timestamp_millis(millis)?;
        (0..=9999).contains(&time.year()).then_some(Self(time))
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
        DateTime::parse_from_rfc2822(text.trim())
            .ok()
            .and_then(Self::from_datetime)
    }

    /// Read an RFC 3339 date-time.
    ///
    /// # Errors
    ///
    /// Fails when the text is not an RFC 3339 date-time or is outside
    /// 0000-9999.
    pub fn parse(text: &str) -> Result<Self, TimestampError> {
        DateTime::parse_from_rfc3339(text)
            .ok()
            .and_then(Self::from_datetime)
            .ok_or_else(|| TimestampError {
                text: text.to_owned(),
            })
    }

    /// The time as a `chrono` UTC date-time.
    #[must_use]
    pub fn to_datetime(self) -> DateTime<Utc> {
        self.0
    }

    /// Milliseconds since the Unix epoch (`Date.prototype.getTime()`).
    #[must_use]
    pub fn unix_millis(self) -> i64 {
        self.0.timestamp_millis()
    }
}

impl fmt::Display for Timestamp {
    /// `toISOString()`: `YYYY-MM-DDTHH:mm:ss.sssZ`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.to_rfc3339_opts(SecondsFormat::Millis, true))
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
}
