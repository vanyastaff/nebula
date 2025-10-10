//! DateTime validator for ISO 8601 date/time strings.
//!
//! Validates date and time strings in ISO 8601 format.

use crate::core::{TypedValidator, ValidationComplexity, ValidationError, ValidatorMetadata};

// ============================================================================
// DATETIME VALIDATOR
// ============================================================================

/// Validates ISO 8601 date/time strings.
///
/// Supports various ISO 8601 formats:
/// - Date: `YYYY-MM-DD`
/// - DateTime: `YYYY-MM-DDTHH:MM:SS`
/// - DateTime with timezone: `YYYY-MM-DDTHH:MM:SS+00:00`
/// - DateTime with milliseconds: `YYYY-MM-DDTHH:MM:SS.sss`
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::DateTime;
/// use nebula_validator::core::TypedValidator;
///
/// let validator = DateTime::new();
///
/// // Valid ISO 8601 strings
/// assert!(validator.validate("2023-12-25").is_ok());
/// assert!(validator.validate("2023-12-25T14:30:00").is_ok());
/// assert!(validator.validate("2023-12-25T14:30:00Z").is_ok());
/// assert!(validator.validate("2023-12-25T14:30:00+03:00").is_ok());
/// assert!(validator.validate("2023-12-25T14:30:00.123Z").is_ok());
///
/// // Invalid
/// assert!(validator.validate("2023-13-01").is_err()); // invalid month
/// assert!(validator.validate("2023-12-32").is_err()); // invalid day
/// assert!(validator.validate("not-a-date").is_err());
/// ```
#[derive(Debug, Clone, Copy)]
pub struct DateTime {
    allow_date_only: bool,
    require_timezone: bool,
    allow_milliseconds: bool,
}

impl DateTime {
    /// Creates a new DateTime validator with default settings.
    ///
    /// Default settings:
    /// - allow_date_only: true (accepts YYYY-MM-DD)
    /// - require_timezone: false
    /// - allow_milliseconds: true
    pub fn new() -> Self {
        Self {
            allow_date_only: true,
            require_timezone: false,
            allow_milliseconds: true,
        }
    }

    /// Require full date-time format (no date-only).
    pub fn require_time(mut self) -> Self {
        self.allow_date_only = false;
        self
    }

    /// Require timezone information (Z or ±HH:MM).
    pub fn require_timezone(mut self) -> Self {
        self.require_timezone = true;
        self
    }

    /// Disallow milliseconds in time.
    pub fn no_milliseconds(mut self) -> Self {
        self.allow_milliseconds = false;
        self
    }

    fn parse_date(date: &str) -> Result<(i32, u8, u8), ValidationError> {
        if date.len() != 10 {
            return Err(ValidationError::new(
                "invalid_date_format",
                "Date must be in YYYY-MM-DD format",
            ));
        }

        let parts: Vec<&str> = date.split('-').collect();
        if parts.len() != 3 {
            return Err(ValidationError::new(
                "invalid_date_format",
                "Date must be in YYYY-MM-DD format",
            ));
        }

        let year = parts[0]
            .parse::<i32>()
            .map_err(|_| ValidationError::new("invalid_year", "Year must be a valid number"))?;

        let month = parts[1]
            .parse::<u8>()
            .map_err(|_| ValidationError::new("invalid_month", "Month must be a valid number"))?;

        let day = parts[2]
            .parse::<u8>()
            .map_err(|_| ValidationError::new("invalid_day", "Day must be a valid number"))?;

        // Validate ranges
        if !(1..=9999).contains(&year) {
            return Err(ValidationError::new(
                "invalid_year",
                "Year must be between 1 and 9999",
            ));
        }

        if !(1..=12).contains(&month) {
            return Err(ValidationError::new(
                "invalid_month",
                "Month must be between 1 and 12",
            ));
        }

        let max_day = Self::days_in_month(year, month);
        if day < 1 || day > max_day {
            return Err(ValidationError::new(
                "invalid_day",
                format!("Day must be between 1 and {}", max_day),
            ));
        }

        Ok((year, month, day))
    }

    fn days_in_month(year: i32, month: u8) -> u8 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if Self::is_leap_year(year) {
                    29
                } else {
                    28
                }
            }
            _ => 0,
        }
    }

    fn is_leap_year(year: i32) -> bool {
        (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
    }

    fn parse_time(time: &str) -> Result<(u8, u8, u8, Option<u16>), ValidationError> {
        // Time format: HH:MM:SS or HH:MM:SS.sss
        let (time_part, millis) = if let Some(dot_pos) = time.find('.') {
            let millis_str = &time[dot_pos + 1..];
            if millis_str.len() > 3 {
                return Err(ValidationError::new(
                    "invalid_milliseconds",
                    "Milliseconds must be 3 digits or less",
                ));
            }
            let millis = millis_str.parse::<u16>().map_err(|_| {
                ValidationError::new(
                    "invalid_milliseconds",
                    "Milliseconds must be a valid number",
                )
            })?;
            (&time[..dot_pos], Some(millis))
        } else {
            (time, None)
        };

        let parts: Vec<&str> = time_part.split(':').collect();
        if parts.len() != 3 {
            return Err(ValidationError::new(
                "invalid_time_format",
                "Time must be in HH:MM:SS format",
            ));
        }

        let hour = parts[0]
            .parse::<u8>()
            .map_err(|_| ValidationError::new("invalid_hour", "Hour must be a valid number"))?;

        let minute = parts[1]
            .parse::<u8>()
            .map_err(|_| ValidationError::new("invalid_minute", "Minute must be a valid number"))?;

        let second = parts[2]
            .parse::<u8>()
            .map_err(|_| ValidationError::new("invalid_second", "Second must be a valid number"))?;

        if hour > 23 {
            return Err(ValidationError::new(
                "invalid_hour",
                "Hour must be between 0 and 23",
            ));
        }

        if minute > 59 {
            return Err(ValidationError::new(
                "invalid_minute",
                "Minute must be between 0 and 59",
            ));
        }

        if second > 59 {
            return Err(ValidationError::new(
                "invalid_second",
                "Second must be between 0 and 59",
            ));
        }

        Ok((hour, minute, second, millis))
    }

    fn parse_timezone(tz: &str) -> Result<(), ValidationError> {
        if tz == "Z" {
            return Ok(());
        }

        if tz.len() != 6 {
            return Err(ValidationError::new(
                "invalid_timezone_format",
                "Timezone must be Z or ±HH:MM",
            ));
        }

        let sign = tz
            .chars()
            .next()
            .expect("tz.len() == 6 guarantees chars().next() succeeds");
        if sign != '+' && sign != '-' {
            return Err(ValidationError::new(
                "invalid_timezone_format",
                "Timezone must start with + or -",
            ));
        }

        let parts: Vec<&str> = tz[1..].split(':').collect();
        if parts.len() != 2 {
            return Err(ValidationError::new(
                "invalid_timezone_format",
                "Timezone must be ±HH:MM",
            ));
        }

        let hours = parts[0].parse::<u8>().map_err(|_| {
            ValidationError::new(
                "invalid_timezone_hours",
                "Timezone hours must be a valid number",
            )
        })?;

        let minutes = parts[1].parse::<u8>().map_err(|_| {
            ValidationError::new(
                "invalid_timezone_minutes",
                "Timezone minutes must be a valid number",
            )
        })?;

        if hours > 23 {
            return Err(ValidationError::new(
                "invalid_timezone_hours",
                "Timezone hours must be between 0 and 23",
            ));
        }

        if minutes > 59 {
            return Err(ValidationError::new(
                "invalid_timezone_minutes",
                "Timezone minutes must be between 0 and 59",
            ));
        }

        Ok(())
    }
}

impl Default for DateTime {
    fn default() -> Self {
        Self::new()
    }
}

impl TypedValidator for DateTime {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &str) -> Result<Self::Output, Self::Error> {
        if input.is_empty() {
            return Err(ValidationError::new(
                "empty_datetime",
                "DateTime string cannot be empty",
            ));
        }

        // Check if it's date-only format
        if !input.contains('T') {
            if !self.allow_date_only {
                return Err(ValidationError::new(
                    "date_only_not_allowed",
                    "Date-only format not allowed, time is required",
                ));
            }
            Self::parse_date(input)?;
            return Ok(());
        }

        // Split date and time parts
        let parts: Vec<&str> = input.split('T').collect();
        if parts.len() != 2 {
            return Err(ValidationError::new(
                "invalid_datetime_format",
                "DateTime must be in YYYY-MM-DDTHH:MM:SS format",
            ));
        }

        // Parse date
        Self::parse_date(parts[0])?;

        // Parse time and timezone
        let time_part = parts[1];

        // Check for timezone
        let (time_str, has_timezone) = if time_part.ends_with('Z') {
            (&time_part[..time_part.len() - 1], true)
        } else if let Some(pos) = time_part.rfind('+').or_else(|| time_part.rfind('-')) {
            if pos > 0 {
                // Make sure it's not at the start
                let tz = &time_part[pos..];
                Self::parse_timezone(tz)?;
                (&time_part[..pos], true)
            } else {
                (time_part, false)
            }
        } else {
            (time_part, false)
        };

        if self.require_timezone && !has_timezone {
            return Err(ValidationError::new(
                "timezone_required",
                "Timezone is required",
            ));
        }

        // Parse time
        let (_, _, _, millis) = Self::parse_time(time_str)?;

        if !self.allow_milliseconds && millis.is_some() {
            return Err(ValidationError::new(
                "milliseconds_not_allowed",
                "Milliseconds are not allowed",
            ));
        }

        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "DateTime".to_string(),
            description: Some(format!(
                "Validates ISO 8601 date/time strings (date-only: {}, timezone: {}, milliseconds: {})",
                if self.allow_date_only {
                    "allowed"
                } else {
                    "not allowed"
                },
                if self.require_timezone {
                    "required"
                } else {
                    "optional"
                },
                if self.allow_milliseconds {
                    "allowed"
                } else {
                    "not allowed"
                }
            )),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(10)),
            tags: vec![
                "text".to_string(),
                "datetime".to_string(),
                "iso8601".to_string(),
            ],
            version: Some("1.0.0".to_string()),
            custom: std::collections::HashMap::new(),
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_dates() {
        let validator = DateTime::new();
        assert!(validator.validate("2023-12-25").is_ok());
        assert!(validator.validate("2024-02-29").is_ok()); // leap year
        assert!(validator.validate("2023-01-01").is_ok());
        assert!(validator.validate("2023-12-31").is_ok());
    }

    #[test]
    fn test_invalid_dates() {
        let validator = DateTime::new();
        assert!(validator.validate("2023-13-01").is_err()); // invalid month
        assert!(validator.validate("2023-12-32").is_err()); // invalid day
        assert!(validator.validate("2023-02-30").is_err()); // invalid day for Feb
        assert!(validator.validate("2023-02-29").is_err()); // not a leap year
    }

    #[test]
    fn test_valid_datetime() {
        let validator = DateTime::new();
        assert!(validator.validate("2023-12-25T14:30:00").is_ok());
        assert!(validator.validate("2023-12-25T00:00:00").is_ok());
        assert!(validator.validate("2023-12-25T23:59:59").is_ok());
    }

    #[test]
    fn test_datetime_with_timezone() {
        let validator = DateTime::new();
        assert!(validator.validate("2023-12-25T14:30:00Z").is_ok());
        assert!(validator.validate("2023-12-25T14:30:00+00:00").is_ok());
        assert!(validator.validate("2023-12-25T14:30:00+03:00").is_ok());
        assert!(validator.validate("2023-12-25T14:30:00-05:00").is_ok());
    }

    #[test]
    fn test_datetime_with_milliseconds() {
        let validator = DateTime::new();
        assert!(validator.validate("2023-12-25T14:30:00.123").is_ok());
        assert!(validator.validate("2023-12-25T14:30:00.123Z").is_ok());
        assert!(validator.validate("2023-12-25T14:30:00.999+03:00").is_ok());
    }

    #[test]
    fn test_require_time() {
        let validator = DateTime::new().require_time();
        assert!(validator.validate("2023-12-25T14:30:00").is_ok());
        assert!(validator.validate("2023-12-25").is_err());
    }

    #[test]
    fn test_require_timezone() {
        let validator = DateTime::new().require_timezone();
        assert!(validator.validate("2023-12-25T14:30:00Z").is_ok());
        assert!(validator.validate("2023-12-25T14:30:00+03:00").is_ok());
        assert!(validator.validate("2023-12-25T14:30:00").is_err());
    }

    #[test]
    fn test_no_milliseconds() {
        let validator = DateTime::new().no_milliseconds();
        assert!(validator.validate("2023-12-25T14:30:00").is_ok());
        assert!(validator.validate("2023-12-25T14:30:00.123").is_err());
    }

    #[test]
    fn test_leap_year() {
        let validator = DateTime::new();
        assert!(validator.validate("2024-02-29").is_ok()); // 2024 is leap
        assert!(validator.validate("2023-02-29").is_err()); // 2023 is not
        assert!(validator.validate("2000-02-29").is_ok()); // 2000 is leap
        assert!(validator.validate("1900-02-29").is_err()); // 1900 is not
    }

    #[test]
    fn test_invalid_time() {
        let validator = DateTime::new();
        assert!(validator.validate("2023-12-25T24:00:00").is_err()); // invalid hour
        assert!(validator.validate("2023-12-25T14:60:00").is_err()); // invalid minute
        assert!(validator.validate("2023-12-25T14:30:60").is_err()); // invalid second
    }

    #[test]
    fn test_empty_string() {
        let validator = DateTime::new();
        assert!(validator.validate("").is_err());
    }

    #[test]
    fn test_real_world_examples() {
        let validator = DateTime::new();
        assert!(validator.validate("2024-01-15T09:30:00Z").is_ok()); // UTC timestamp
        assert!(validator.validate("2024-01-15T12:30:00+03:00").is_ok()); // Moscow time
        assert!(validator.validate("2024-01-15T08:30:00.000Z").is_ok()); // With milliseconds
    }
}
