//! Temporal validators: Date, DateTime, Time, Uuid
//!
//! Pure-Rust implementations — no `chrono` or `uuid` dependencies.
//! Formats follow ISO 8601 / RFC 3339.

use crate::foundation::{Validate, ValidationError};

// ============================================================================
// Helpers
// ============================================================================

/// Parse an integer from a byte slice without allocation.
fn parse_u32(s: &[u8]) -> Option<u32> {
    if s.is_empty() || s.len() > 10 {
        return None;
    }
    let mut n: u32 = 0;
    for &b in s {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
}

fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Validates `YYYY-MM-DD` and returns `(year, month, day)` or an error.
fn parse_date_parts(s: &str) -> Result<(u32, u32, u32), ValidationError> {
    let b = s.as_bytes();
    // Must be exactly YYYY-MM-DD (10 chars)
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return Err(date_format_err(s));
    }
    let year = parse_u32(&b[0..4]).ok_or_else(|| date_format_err(s))?;
    let month = parse_u32(&b[5..7]).ok_or_else(|| date_format_err(s))?;
    let day = parse_u32(&b[8..10]).ok_or_else(|| date_format_err(s))?;

    if !(1..=12).contains(&month) {
        return Err(ValidationError::new(
            "date",
            format!("Month {month} is out of range (1-12)"),
        ));
    }
    if day < 1 || day > days_in_month(year, month) {
        return Err(ValidationError::new(
            "date",
            format!("Day {day} is out of range for {year}-{month:02}"),
        ));
    }
    Ok((year, month, day))
}

fn date_format_err(s: &str) -> ValidationError {
    ValidationError::new(
        "date",
        format!("'{s}' is not a valid date (expected YYYY-MM-DD)"),
    )
    .with_param("actual", s.to_string())
}

fn time_format_err(s: &str) -> ValidationError {
    ValidationError::new(
        "time",
        format!("'{s}' is not a valid time (expected HH:MM:SS or HH:MM:SS.sss)"),
    )
    .with_param("actual", s.to_string())
}

/// Validates `HH:MM:SS` or `HH:MM:SS.sss` and returns remaining bytes after seconds.
fn parse_time_parts(s: &str) -> Result<&str, ValidationError> {
    let b = s.as_bytes();
    if b.len() < 8 || b[2] != b':' || b[5] != b':' {
        return Err(time_format_err(s));
    }
    let hour = parse_u32(&b[0..2]).ok_or_else(|| time_format_err(s))?;
    let minute = parse_u32(&b[3..5]).ok_or_else(|| time_format_err(s))?;
    let second = parse_u32(&b[6..8]).ok_or_else(|| time_format_err(s))?;

    if hour > 23 {
        return Err(ValidationError::new(
            "time",
            format!("Hour {hour} is out of range (0-23)"),
        ));
    }
    if minute > 59 {
        return Err(ValidationError::new(
            "time",
            format!("Minute {minute} is out of range (0-59)"),
        ));
    }
    if second > 60 {
        // 60 allowed for leap seconds
        return Err(ValidationError::new(
            "time",
            format!("Second {second} is out of range (0-60)"),
        ));
    }

    let rest = &s[8..];
    // Optional fractional seconds
    let rest = if let Some(frac) = rest.strip_prefix('.') {
        let frac_end = frac
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(frac.len());
        if frac_end == 0 {
            return Err(time_format_err(s));
        }
        &frac[frac_end..]
    } else {
        rest
    };

    Ok(rest)
}

// ============================================================================
// Date
// ============================================================================

/// Validates that a string is a valid date in `YYYY-MM-DD` format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Date;

impl Validate for Date {
    type Input = str;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        parse_date_parts(input).map(|_| ())
    }
}

/// Creates a date validator (YYYY-MM-DD).
#[must_use]
pub fn date() -> Date {
    Date
}

// ============================================================================
// Time
// ============================================================================

/// Validates that a string is a valid time in `HH:MM:SS` or `HH:MM:SS.sss` format.
///
/// Does **not** require a timezone suffix. Use `.and(has_timezone())` for that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Time;

impl Validate for Time {
    type Input = str;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        parse_time_parts(input)?;
        Ok(())
    }
}

/// Creates a time validator (`HH:MM:SS` or `HH:MM:SS.sss`).
#[must_use]
pub fn time() -> Time {
    Time
}

// ============================================================================
// DateTime (RFC 3339 / ISO 8601)
// ============================================================================

/// Validates that a string is a valid date-time per RFC 3339.
///
/// Accepted formats:
/// - `YYYY-MM-DDTHH:MM:SSZ`
/// - `YYYY-MM-DDTHH:MM:SS+HH:MM`
/// - `YYYY-MM-DDTHH:MM:SS.sssZ`
/// - Separator may be `T` or `t` or space ` `
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DateTime;

impl Validate for DateTime {
    type Input = str;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let err = || {
            ValidationError::new(
                "datetime",
                format!("'{input}' is not a valid RFC 3339 date-time"),
            )
            .with_param("actual", input.to_string())
        };

        if input.len() < 19 {
            return Err(err());
        }

        // Date part
        parse_date_parts(&input[..10]).map_err(|_| err())?;

        // Separator T/t/space
        let sep = input.as_bytes().get(10).copied();
        if !matches!(sep, Some(b'T') | Some(b't') | Some(b' ')) {
            return Err(err());
        }

        // Time part + optional timezone
        let time_str = &input[11..];
        let rest = parse_time_parts(time_str).map_err(|_| err())?;

        // Timezone: Z, +HH:MM, -HH:MM
        validate_timezone_suffix(rest).map_err(|_| err())?;

        Ok(())
    }
}

fn validate_timezone_suffix(s: &str) -> Result<(), ()> {
    match s {
        "Z" | "z" => Ok(()),
        _ if s.len() == 6 => {
            let b = s.as_bytes();
            if (b[0] == b'+' || b[0] == b'-') && b[3] == b':' {
                let h = parse_u32(&b[1..3]).ok_or(())?;
                let m = parse_u32(&b[4..6]).ok_or(())?;
                if h <= 23 && m <= 59 {
                    return Ok(());
                }
            }
            Err(())
        }
        _ => Err(()),
    }
}

/// Creates a date-time validator (RFC 3339).
#[must_use]
pub fn date_time() -> DateTime {
    DateTime
}

// ============================================================================
// Uuid (RFC 4122)
// ============================================================================

/// Validates that a string is a valid UUID in the standard
/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` format.
///
/// Case-insensitive. Does not enforce version/variant bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uuid;

impl Validate for Uuid {
    type Input = str;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let err = || {
            ValidationError::new(
                "uuid",
                format!(
                    "'{input}' is not a valid UUID (expected xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)"
                ),
            )
            .with_param("actual", input.to_string())
        };

        let b = input.as_bytes();
        // 32 hex + 4 hyphens = 36 chars
        if b.len() != 36 {
            return Err(err());
        }
        // Hyphens at positions 8, 13, 18, 23
        for &pos in &[8usize, 13, 18, 23] {
            if b[pos] != b'-' {
                return Err(err());
            }
        }
        // All other chars must be hex
        for (i, &byte) in b.iter().enumerate() {
            if i == 8 || i == 13 || i == 18 || i == 23 {
                continue;
            }
            if !byte.is_ascii_hexdigit() {
                return Err(err());
            }
        }
        Ok(())
    }
}

/// Creates a UUID validator (RFC 4122, `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`).
#[must_use]
pub fn uuid() -> Uuid {
    Uuid
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- Date --

    #[test]
    fn date_valid() {
        assert!(Date.validate("2024-01-01").is_ok());
        assert!(Date.validate("2024-02-29").is_ok()); // leap year
        assert!(Date.validate("2000-12-31").is_ok());
    }

    #[test]
    fn date_invalid() {
        assert!(Date.validate("2024-13-01").is_err()); // bad month
        assert!(Date.validate("2023-02-29").is_err()); // not leap
        assert!(Date.validate("2024-00-01").is_err()); // month 0
        assert!(Date.validate("2024-1-1").is_err()); // wrong format
        assert!(Date.validate("not-a-date").is_err());
        assert!(Date.validate("").is_err());
    }

    // -- Time --

    #[test]
    fn time_valid() {
        assert!(Time.validate("00:00:00").is_ok());
        assert!(Time.validate("23:59:59").is_ok());
        assert!(Time.validate("12:30:45.123").is_ok());
        assert!(Time.validate("12:30:60").is_ok()); // leap second
    }

    #[test]
    fn time_invalid() {
        assert!(Time.validate("24:00:00").is_err()); // hour out of range
        assert!(Time.validate("12:60:00").is_err()); // minute out of range
        assert!(Time.validate("12:30").is_err()); // missing seconds
        assert!(Time.validate("not-a-time").is_err());
    }

    // -- DateTime --

    #[test]
    fn datetime_valid() {
        assert!(DateTime.validate("2024-01-01T00:00:00Z").is_ok());
        assert!(DateTime.validate("2024-06-15T12:30:45+05:30").is_ok());
        assert!(DateTime.validate("2024-06-15T12:30:45.999Z").is_ok());
        assert!(DateTime.validate("2024-06-15 12:30:45Z").is_ok()); // space separator
    }

    #[test]
    fn datetime_invalid() {
        assert!(DateTime.validate("2024-01-01").is_err()); // date only
        assert!(DateTime.validate("2024-13-01T00:00:00Z").is_err()); // bad month
        assert!(DateTime.validate("2024-01-01T25:00:00Z").is_err()); // bad hour
        assert!(DateTime.validate("2024-01-01T00:00:00").is_err()); // no timezone
        assert!(DateTime.validate("not-a-datetime").is_err());
    }

    // -- Uuid --

    #[test]
    fn uuid_valid() {
        assert!(
            Uuid.validate("550e8400-e29b-41d4-a716-446655440000")
                .is_ok()
        );
        assert!(
            Uuid.validate("00000000-0000-0000-0000-000000000000")
                .is_ok()
        );
        assert!(
            Uuid.validate("FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF")
                .is_ok()
        ); // uppercase
    }

    #[test]
    fn uuid_invalid() {
        assert!(
            Uuid.validate("550e8400-e29b-41d4-a716-44665544000g")
                .is_err()
        ); // 'g' not hex
        assert!(Uuid.validate("550e8400-e29b-41d4-a716-4466554400").is_err()); // too short
        assert!(Uuid.validate("550e8400e29b41d4a716446655440000").is_err()); // no hyphens
        assert!(Uuid.validate("").is_err());
    }
}
