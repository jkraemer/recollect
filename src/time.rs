//! Timestamps in the stored format and parsing of `--since` / `--until`.

use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, Utc};

use crate::error::{Error, Result};

/// `YYYY-MM-DDTHH:MM:SS.sssZ`, the format of every stored timestamp; such
/// strings compare correctly as text.
pub fn format_timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn now_timestamp() -> String {
    format_timestamp(Utc::now())
}

/// Inclusive lower bound: a date means its first millisecond (UTC).
pub fn parse_since(raw: &str) -> Result<String> {
    parse_bound(raw, |date| {
        date.and_hms_opt(0, 0, 0).expect("midnight exists")
    })
}

/// Inclusive upper bound: a date means its last millisecond (UTC).
pub fn parse_until(raw: &str) -> Result<String> {
    parse_bound(raw, |date| {
        date.and_hms_milli_opt(23, 59, 59, 999)
            .expect("end of day exists")
    })
}

/// A date becomes the moment `moment_of` picks within it; anything else must be RFC 3339.
fn parse_bound(raw: &str, moment_of: impl Fn(NaiveDate) -> NaiveDateTime) -> Result<String> {
    match NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        Ok(date) => Ok(format_timestamp(moment_of(date).and_utc())),
        Err(_) => parse_rfc3339(raw),
    }
}

/// Days between a stored timestamp and `now`; unparseable timestamps count as new.
pub fn age_days(created_at: &str, now: DateTime<Utc>) -> f64 {
    match DateTime::parse_from_rfc3339(created_at) {
        Ok(created) => (now - created.with_timezone(&Utc)).num_milliseconds() as f64 / 86_400_000.0,
        Err(_) => 0.0,
    }
}

fn parse_rfc3339(raw: &str) -> Result<String> {
    DateTime::parse_from_rfc3339(raw)
        .map(|at| format_timestamp(at.with_timezone(&Utc)))
        .map_err(|_| Error::InvalidDate(raw.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeDelta, TimeZone};

    #[test]
    fn timestamps_have_milliseconds_and_a_z_suffix() {
        let at = Utc.with_ymd_and_hms(2026, 9, 29, 8, 5, 3).unwrap() + TimeDelta::milliseconds(45);
        assert_eq!(format_timestamp(at), "2026-09-29T08:05:03.045Z");
        assert_eq!(now_timestamp().len(), "2026-09-29T08:05:03.045Z".len());
    }

    #[test]
    fn a_date_bound_covers_the_whole_day() {
        assert_eq!(
            parse_since("2026-09-01").unwrap(),
            "2026-09-01T00:00:00.000Z"
        );
        assert_eq!(
            parse_until("2026-09-01").unwrap(),
            "2026-09-01T23:59:59.999Z"
        );
    }

    #[test]
    fn rfc3339_bounds_are_normalized_to_utc() {
        assert_eq!(
            parse_since("2026-09-01T10:00:00+02:00").unwrap(),
            "2026-09-01T08:00:00.000Z"
        );
        assert_eq!(
            parse_until("2026-09-01T10:00:00Z").unwrap(),
            "2026-09-01T10:00:00.000Z"
        );
    }

    #[test]
    fn unparseable_dates_are_rejected() {
        assert!(matches!(parse_since("yesterday"), Err(Error::InvalidDate(d)) if d == "yesterday"));
    }

    #[test]
    fn age_is_measured_in_days_and_unparseable_timestamps_count_as_new() {
        let now = Utc.with_ymd_and_hms(2026, 9, 11, 0, 0, 0).unwrap();
        assert!((age_days("2026-09-01T00:00:00.000Z", now) - 10.0).abs() < 1e-9);
        assert_eq!(age_days("not a timestamp", now), 0.0);
    }
}
