//! Timestamp conversion between `chrono` and storage formats.
//!
//! SQLite stores timestamps as ISO 8601 text; Postgres uses native
//! `TIMESTAMPTZ`. These helpers normalize around RFC 3339 strings
//! for the text path.

use chrono::{DateTime, Utc};

use crate::error::StorageError;

/// Format a `DateTime<Utc>` as an RFC 3339 / ISO 8601 string.
///
/// Suitable for SQLite `TEXT` columns.
pub fn to_iso8601(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339()
}

/// Parse an RFC 3339 / ISO 8601 string back to `DateTime<Utc>`.
///
/// # Errors
///
/// Returns [`StorageError::Serialization`] if the string is not valid RFC 3339.
pub fn from_iso8601(s: &str) -> Result<DateTime<Utc>, StorageError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| StorageError::Serialization(format!("invalid ISO 8601 timestamp: {e}")))
}

/// Current UTC timestamp.
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Timelike};

    use super::*;

    #[test]
    fn roundtrip() {
        let ts = now();
        let s = to_iso8601(&ts);
        let parsed = from_iso8601(&s).unwrap();
        assert_eq!(ts, parsed);
    }

    #[test]
    fn known_value() {
        let s = "2024-06-15T12:30:00+00:00";
        let dt = from_iso8601(s).unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 6);
        assert_eq!(dt.day(), 15);
    }

    #[test]
    fn with_offset() {
        // Should convert to UTC.
        let s = "2024-01-01T00:00:00+05:00";
        let dt = from_iso8601(s).unwrap();
        assert_eq!(dt.hour(), 19); // 00:00 +05:00 = 19:00 UTC (previous day)
    }

    #[test]
    fn invalid_string() {
        let err = from_iso8601("not-a-timestamp").unwrap_err();
        assert!(err.to_string().contains("invalid ISO 8601"));
    }

    #[test]
    fn empty_string() {
        assert!(from_iso8601("").is_err());
    }
}
