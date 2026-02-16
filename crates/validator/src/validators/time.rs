//! Time-only validator for ISO 8601 time strings.
//!
//! Validates time strings in `HH:MM:SS` format with optional fractional
//! seconds and timezone offset.

use crate::foundation::{Validate, ValidationComplexity, ValidationError, ValidatorMetadata};

// ============================================================================
// TIMEONLY VALIDATOR
// ============================================================================

/// Validates ISO 8601 time-only strings.
///
/// Supports:
/// - `HH:MM:SS`
/// - `HH:MM:SS.sss` (1-3 fractional digits)
/// - `HH:MM:SSZ`
/// - `HH:MM:SS+HH:MM` / `HH:MM:SS-HH:MM`
///
/// Hours: 0..=23, Minutes: 0..=59, Seconds: 0..=60 (60 for leap second).
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::TimeOnly;
/// use nebula_validator::foundation::Validate;
///
/// let v = TimeOnly::new();
/// assert!(v.validate("14:30:00").is_ok());
/// assert!(v.validate("14:30:00Z").is_ok());
/// assert!(v.validate("14:30:00.123").is_ok());
/// assert!(v.validate("14:30:00+05:30").is_ok());
/// assert!(v.validate("25:00:00").is_err());
/// ```
#[derive(Debug, Clone, Copy)]
pub struct TimeOnly {
    require_timezone: bool,
}

impl TimeOnly {
    /// Creates a new `TimeOnly` validator with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            require_timezone: false,
        }
    }

    /// Require timezone information (Z or +/-HH:MM).
    #[must_use = "builder methods must be chained or built"]
    pub fn require_timezone(mut self) -> Self {
        self.require_timezone = true;
        self
    }
}

impl Default for TimeOnly {
    fn default() -> Self {
        Self::new()
    }
}

/// Parses a two-digit numeric field from a byte slice at the given offset.
/// Returns the parsed value and expects exactly two ASCII digit bytes.
fn parse_two_digits(bytes: &[u8], offset: usize) -> Option<u8> {
    if offset + 2 > bytes.len() {
        return None;
    }
    let d1 = bytes[offset].wrapping_sub(b'0');
    let d2 = bytes[offset + 1].wrapping_sub(b'0');
    if d1 > 9 || d2 > 9 {
        return None;
    }
    Some(d1 * 10 + d2)
}

impl Validate for TimeOnly {
    type Input = str;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if input.is_empty() {
            return Err(ValidationError::new(
                "empty_time",
                "Time string cannot be empty",
            ));
        }

        let bytes = input.as_bytes();

        // Minimum valid: HH:MM:SS = 8 chars
        if bytes.len() < 8 {
            return Err(ValidationError::new(
                "invalid_time_format",
                "Time must be in HH:MM:SS format",
            ));
        }

        // Parse HH:MM:SS
        let hour = parse_two_digits(bytes, 0)
            .ok_or_else(|| ValidationError::new("invalid_hour", "Hour must be two digits"))?;

        if bytes[2] != b':' {
            return Err(ValidationError::new(
                "invalid_time_format",
                "Expected ':' after hours",
            ));
        }

        let minute = parse_two_digits(bytes, 3)
            .ok_or_else(|| ValidationError::new("invalid_minute", "Minute must be two digits"))?;

        if bytes[5] != b':' {
            return Err(ValidationError::new(
                "invalid_time_format",
                "Expected ':' after minutes",
            ));
        }

        let second = parse_two_digits(bytes, 6)
            .ok_or_else(|| ValidationError::new("invalid_second", "Second must be two digits"))?;

        // Validate ranges
        if hour > 23 {
            return Err(ValidationError::new(
                "invalid_hour",
                format!("Hour {hour} must be between 0 and 23"),
            ));
        }

        if minute > 59 {
            return Err(ValidationError::new(
                "invalid_minute",
                format!("Minute {minute} must be between 0 and 59"),
            ));
        }

        // 60 allowed for leap second
        if second > 60 {
            return Err(ValidationError::new(
                "invalid_second",
                format!("Second {second} must be between 0 and 60"),
            ));
        }

        // Parse remainder after HH:MM:SS (position 8)
        let mut pos = 8;

        // Optional fractional seconds
        if pos < bytes.len() && bytes[pos] == b'.' {
            pos += 1;
            let frac_start = pos;

            while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                pos += 1;
            }

            let frac_len = pos - frac_start;
            if frac_len == 0 || frac_len > 3 {
                return Err(ValidationError::new(
                    "invalid_fractional_seconds",
                    "Fractional seconds must be 1 to 3 digits",
                ));
            }
        }

        // Optional timezone
        let has_timezone = if pos < bytes.len() {
            if bytes[pos] == b'Z' {
                pos += 1;
                true
            } else if bytes[pos] == b'+' || bytes[pos] == b'-' {
                // Expect +HH:MM or -HH:MM (6 chars)
                if pos + 6 != bytes.len() {
                    return Err(ValidationError::new(
                        "invalid_timezone_format",
                        "Timezone offset must be in +HH:MM or -HH:MM format",
                    ));
                }

                let tz_hour = parse_two_digits(bytes, pos + 1).ok_or_else(|| {
                    ValidationError::new(
                        "invalid_timezone_hours",
                        "Timezone hours must be two digits",
                    )
                })?;

                if bytes[pos + 3] != b':' {
                    return Err(ValidationError::new(
                        "invalid_timezone_format",
                        "Expected ':' in timezone offset",
                    ));
                }

                let tz_min = parse_two_digits(bytes, pos + 4).ok_or_else(|| {
                    ValidationError::new(
                        "invalid_timezone_minutes",
                        "Timezone minutes must be two digits",
                    )
                })?;

                if tz_hour > 23 {
                    return Err(ValidationError::new(
                        "invalid_timezone_hours",
                        format!("Timezone hours {tz_hour} must be between 0 and 23"),
                    ));
                }

                if tz_min > 59 {
                    return Err(ValidationError::new(
                        "invalid_timezone_minutes",
                        format!("Timezone minutes {tz_min} must be between 0 and 59"),
                    ));
                }

                pos += 6;
                true
            } else {
                return Err(ValidationError::new(
                    "invalid_time_format",
                    format!("Unexpected character '{}' after time", bytes[pos] as char),
                ));
            }
        } else {
            false
        };

        // Must have consumed all input
        if pos != bytes.len() {
            return Err(ValidationError::new(
                "invalid_time_format",
                "Unexpected trailing characters after time",
            ));
        }

        if self.require_timezone && !has_timezone {
            return Err(ValidationError::new(
                "timezone_required",
                "Timezone is required",
            ));
        }

        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "TimeOnly".into(),
            description: Some(
                format!(
                    "Validates ISO 8601 time strings (timezone: {})",
                    if self.require_timezone {
                        "required"
                    } else {
                        "optional"
                    }
                )
                .into(),
            ),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(5)),
            tags: vec!["text".into(), "time".into(), "iso8601".into()],
            version: Some("1.0.0".into()),
            custom: Vec::new(),
        }
    }
}

/// Creates a new [`TimeOnly`] validator with default settings.
#[must_use]
pub fn time_only() -> TimeOnly {
    TimeOnly::new()
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- Valid times ---

    #[test]
    fn valid_basic_time() {
        let v = time_only();
        assert!(v.validate("14:30:00").is_ok());
    }

    #[test]
    fn valid_midnight() {
        let v = time_only();
        assert!(v.validate("00:00:00").is_ok());
    }

    #[test]
    fn valid_end_of_day() {
        let v = time_only();
        assert!(v.validate("23:59:59").is_ok());
    }

    #[test]
    fn valid_with_milliseconds() {
        let v = time_only();
        assert!(v.validate("14:30:00.123").is_ok());
    }

    #[test]
    fn valid_with_single_frac_digit() {
        let v = time_only();
        assert!(v.validate("14:30:00.1").is_ok());
    }

    #[test]
    fn valid_with_two_frac_digits() {
        let v = time_only();
        assert!(v.validate("14:30:00.12").is_ok());
    }

    #[test]
    fn valid_with_zulu() {
        let v = time_only();
        assert!(v.validate("14:30:00Z").is_ok());
    }

    #[test]
    fn valid_with_positive_offset() {
        let v = time_only();
        assert!(v.validate("14:30:00+05:30").is_ok());
    }

    #[test]
    fn valid_with_negative_offset() {
        let v = time_only();
        assert!(v.validate("14:30:00-08:00").is_ok());
    }

    #[test]
    fn valid_leap_second() {
        let v = time_only();
        assert!(v.validate("23:59:60").is_ok());
    }

    #[test]
    fn valid_millis_and_timezone() {
        let v = time_only();
        assert!(v.validate("14:30:00.999Z").is_ok());
        assert!(v.validate("14:30:00.123+01:00").is_ok());
    }

    // --- Invalid times ---

    #[test]
    fn invalid_hour_too_large() {
        let v = time_only();
        assert!(v.validate("25:00:00").is_err());
    }

    #[test]
    fn invalid_minute_too_large() {
        let v = time_only();
        assert!(v.validate("00:60:00").is_err());
    }

    #[test]
    fn invalid_empty() {
        let v = time_only();
        assert!(v.validate("").is_err());
    }

    #[test]
    fn invalid_no_seconds() {
        let v = time_only();
        assert!(v.validate("14:30").is_err());
    }

    #[test]
    fn invalid_not_a_time() {
        let v = time_only();
        assert!(v.validate("abc").is_err());
    }

    #[test]
    fn invalid_four_digit_millis() {
        let v = time_only();
        assert!(v.validate("14:30:00.1234").is_err());
    }

    #[test]
    fn invalid_dot_no_digits() {
        let v = time_only();
        assert!(v.validate("14:30:00.").is_err());
    }

    #[test]
    fn invalid_second_61() {
        let v = time_only();
        assert!(v.validate("23:59:61").is_err());
    }

    // --- require_timezone ---

    #[test]
    fn require_timezone_rejects_bare_time() {
        let v = TimeOnly::new().require_timezone();
        assert!(v.validate("14:30:00").is_err());
    }

    #[test]
    fn require_timezone_accepts_zulu() {
        let v = TimeOnly::new().require_timezone();
        assert!(v.validate("14:30:00Z").is_ok());
    }

    #[test]
    fn require_timezone_accepts_offset() {
        let v = TimeOnly::new().require_timezone();
        assert!(v.validate("14:30:00+05:30").is_ok());
    }

    #[test]
    fn require_timezone_accepts_millis_with_tz() {
        let v = TimeOnly::new().require_timezone();
        assert!(v.validate("14:30:00.123Z").is_ok());
    }

    #[test]
    fn require_timezone_rejects_millis_without_tz() {
        let v = TimeOnly::new().require_timezone();
        assert!(v.validate("14:30:00.123").is_err());
    }
}
