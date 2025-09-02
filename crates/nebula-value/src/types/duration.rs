use std::fmt;
use std::time::Duration as StdDuration;
use thiserror::Error;
// ==================== Duration Errors ====================

/// Result type for Duration operations
pub type DurationResult<T> = Result<T, DurationError>;

/// Errors that can occur with Duration operations
#[derive(Debug, Error, Clone, PartialEq)]
pub enum DurationError {
    /// Negative duration
    #[error("Duration cannot be negative: {value}")]
    NegativeDuration { value: String },

    /// Overflow in duration arithmetic
    #[error("Duration arithmetic overflow")]
    ArithmeticOverflow,

    /// Invalid duration value
    #[error("Invalid duration value: {value}")]
    InvalidValue { value: String },

    /// Parse error
    #[error("Failed to parse duration from '{input}'")]
    ParseError { input: String },

    /// Division by zero
    #[error("Cannot divide duration by zero")]
    DivisionByZero,

    /// Not finite
    #[error("Duration must be finite, got {value}")]
    NotFinite { value: String },
}

impl DurationError {
    /// Create a negative duration error
    pub fn negative_duration<S: Into<String>>(value: S) -> Self {
        Self::NegativeDuration {
            value: value.into(),
        }
    }

    /// Create an invalid value error
    pub fn invalid_value<S: Into<String>>(value: S) -> Self {
        Self::InvalidValue {
            value: value.into(),
        }
    }

    /// Create a parse error
    pub fn parse_error<S: Into<String>>(input: S) -> Self {
        Self::ParseError {
            input: input.into(),
        }
    }

    /// Create a not finite error
    pub fn not_finite<S: Into<String>>(value: S) -> Self {
        Self::NotFinite {
            value: value.into(),
        }
    }
}

/// Duration type for time intervals
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration(StdDuration);

impl Duration {
    // ==================== Constants ====================

    /// Zero duration
    pub const ZERO: Self = Self(StdDuration::ZERO);

    /// Maximum duration
    pub const MAX: Self = Self(StdDuration::MAX);

    // ==================== Constructors ====================

    /// Create a new duration
    #[inline]
    pub const fn new(duration: StdDuration) -> Self {
        Self(duration)
    }

    /// Create from seconds
    #[inline]
    pub const fn from_secs(secs: u64) -> Self {
        Self(StdDuration::from_secs(secs))
    }

    /// Create from milliseconds
    #[inline]
    pub const fn from_millis(millis: u64) -> Self {
        Self(StdDuration::from_millis(millis))
    }

    /// Create from microseconds
    #[inline]
    pub const fn from_micros(micros: u64) -> Self {
        Self(StdDuration::from_micros(micros))
    }

    /// Create from nanoseconds
    #[inline]
    pub const fn from_nanos(nanos: u64) -> Self {
        Self(StdDuration::from_nanos(nanos))
    }

    /// Create from floating-point seconds
    pub fn from_secs_f64(secs: f64) -> DurationResult<Self> {
        if !secs.is_finite() {
            return Err(DurationError::not_finite(secs.to_string()));
        }
        if secs < 0.0 {
            return Err(DurationError::negative_duration(secs.to_string()));
        }
        Ok(Self(StdDuration::from_secs_f64(secs)))
    }

    /// Create from floating-point seconds (32-bit)
    pub fn from_secs_f32(secs: f32) -> DurationResult<Self> {
        if !secs.is_finite() {
            return Err(DurationError::not_finite(secs.to_string()));
        }
        if secs < 0.0 {
            return Err(DurationError::negative_duration(secs.to_string()));
        }
        Ok(Self(StdDuration::from_secs_f32(secs)))
    }

    /// Create from minutes
    #[inline]
    pub const fn from_minutes(minutes: u64) -> Self {
        Self::from_secs(minutes * 60)
    }

    /// Create from hours
    #[inline]
    pub const fn from_hours(hours: u64) -> Self {
        Self::from_secs(hours * 3600)
    }

    /// Create from days
    #[inline]
    pub const fn from_days(days: u64) -> Self {
        Self::from_secs(days * 86400)
    }

    // ==================== Accessors ====================

    /// Get the underlying std::time::Duration
    #[inline]
    pub const fn inner(&self) -> &StdDuration {
        &self.0
    }

    /// Get the value in seconds
    #[inline]
    pub const fn as_secs(&self) -> u64 {
        self.0.as_secs()
    }

    /// Get the value in milliseconds
    #[inline]
    pub const fn as_millis(&self) -> u128 {
        self.0.as_millis()
    }

    /// Get the value in microseconds
    #[inline]
    pub const fn as_micros(&self) -> u128 {
        self.0.as_micros()
    }

    /// Get the value in nanoseconds
    #[inline]
    pub const fn as_nanos(&self) -> u128 {
        self.0.as_nanos()
    }

    /// Get as floating-point seconds
    #[inline]
    pub fn as_secs_f64(&self) -> f64 {
        self.0.as_secs_f64()
    }

    /// Get as floating-point seconds (32-bit)
    #[inline]
    pub fn as_secs_f32(&self) -> f32 {
        self.0.as_secs_f32()
    }

    /// Get the value in minutes (truncated)
    #[inline]
    pub const fn as_minutes(&self) -> u64 {
        self.as_secs() / 60
    }

    /// Get the value in hours (truncated)
    #[inline]
    pub const fn as_hours(&self) -> u64 {
        self.as_secs() / 3600
    }

    /// Get the value in days (truncated)
    #[inline]
    pub const fn as_days(&self) -> u64 {
        self.as_secs() / 86400
    }

    /// Get subsecond nanoseconds
    #[inline]
    pub const fn subsec_nanos(&self) -> u32 {
        self.0.subsec_nanos()
    }

    /// Get subsecond microseconds
    #[inline]
    pub const fn subsec_micros(&self) -> u32 {
        self.0.subsec_micros()
    }

    /// Get subsecond milliseconds
    #[inline]
    pub const fn subsec_millis(&self) -> u32 {
        self.0.subsec_millis()
    }

    // ==================== State Checks ====================

    /// Check if duration is zero
    #[inline]
    pub const fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    /// Check if duration is maximum
    #[inline]
    pub fn is_max(&self) -> bool {
        *self == Self::MAX
    }

    // ==================== Operations ====================

    /// Add two durations (saturating)
    #[inline]
    pub fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// Subtract two durations (saturating)
    #[inline]
    pub fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// Multiply by integer (saturating)
    #[inline]
    pub fn saturating_mul(self, rhs: u32) -> Self {
        Self(self.0.saturating_mul(rhs))
    }

    /// Add two durations (checked)
    #[inline]
    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }

    /// Subtract two durations (checked)
    #[inline]
    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self)
    }

    /// Multiply by integer (checked)
    #[inline]
    pub fn checked_mul(self, rhs: u32) -> Option<Self> {
        self.0.checked_mul(rhs).map(Self)
    }

    /// Divide by integer (checked)
    #[inline]
    pub fn checked_div(self, rhs: u32) -> Option<Self> {
        self.0.checked_div(rhs).map(Self)
    }

    // ==================== Comparisons ====================

    /// Get minimum of two durations
    #[inline]
    pub fn min(self, other: Self) -> Self {
        if self <= other { self } else { other }
    }

    /// Get maximum of two durations
    #[inline]
    pub fn max(self, other: Self) -> Self {
        if self >= other { self } else { other }
    }

    /// Clamp duration to range
    #[inline]
    pub fn clamp(self, min: Self, max: Self) -> Self {
        self.max(min).min(max)
    }

    /// Get absolute difference between two durations
    #[inline]
    pub fn abs_diff(self, other: Self) -> Self {
        if self > other {
            self.saturating_sub(other)
        } else {
            other.saturating_sub(self)
        }
    }

    // ==================== Formatting ====================

    /// Format as compact string (e.g., "1.5s", "30ms")
    pub fn to_compact_string(&self) -> String {
        if self.is_zero() {
            return "0s".to_string();
        }

        let nanos = self.as_nanos();

        if nanos >= 1_000_000_000 {
            // Seconds or more
            let secs = self.as_secs_f64();
            if secs >= 86400.0 {
                format!("{:.1}d", secs / 86400.0)
            } else if secs >= 3600.0 {
                format!("{:.1}h", secs / 3600.0)
            } else if secs >= 60.0 {
                format!("{:.1}m", secs / 60.0)
            } else {
                format!("{:.1}s", secs)
            }
        } else if nanos >= 1_000_000 {
            format!("{}ms", self.as_millis())
        } else if nanos >= 1_000 {
            format!("{}μs", self.as_micros())
        } else {
            format!("{}ns", nanos)
        }
    }

    /// Format as human readable string
    pub fn to_human_string(&self) -> String {
        if self.is_zero() {
            return "0 seconds".to_string();
        }

        let mut parts = Vec::new();
        let mut secs = self.as_secs();

        // Days
        let days = secs / 86400;
        if days > 0 {
            parts.push(format!("{} {}", days, if days == 1 { "day" } else { "days" }));
            secs %= 86400;
        }

        // Hours
        let hours = secs / 3600;
        if hours > 0 {
            parts.push(format!("{} {}", hours, if hours == 1 { "hour" } else { "hours" }));
            secs %= 3600;
        }

        // Minutes
        let minutes = secs / 60;
        if minutes > 0 {
            parts.push(format!("{} {}", minutes, if minutes == 1 { "minute" } else { "minutes" }));
            secs %= 60;
        }

        // Seconds
        if secs > 0 || parts.is_empty() {
            parts.push(format!("{} {}", secs, if secs == 1 { "second" } else { "seconds" }));
        }

        // Milliseconds if no other parts and we have them
        if parts.is_empty() {
            let millis = self.subsec_millis();
            if millis > 0 {
                parts.push(format!("{} milliseconds", millis));
            }
        }

        parts.join(" ")
    }
}

// ==================== Conversions ====================

impl From<StdDuration> for Duration {
    #[inline]
    fn from(d: StdDuration) -> Self {
        Self(d)
    }
}

impl From<Duration> for StdDuration {
    #[inline]
    fn from(d: Duration) -> StdDuration {
        d.0
    }
}

impl AsRef<StdDuration> for Duration {
    #[inline]
    fn as_ref(&self) -> &StdDuration {
        &self.0
    }
}

// ==================== Display ====================

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_compact_string())
    }
}

// ==================== Default ====================

impl Default for Duration {
    #[inline]
    fn default() -> Self {
        Self::ZERO
    }
}

// ==================== Arithmetic Operations ====================

impl std::ops::Add for Duration {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Duration {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl std::ops::Mul<u32> for Duration {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: u32) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl std::ops::Div<u32> for Duration {
    type Output = Self;

    #[inline]
    fn div(self, rhs: u32) -> Self::Output {
        Self(self.0 / rhs)
    }
}

impl std::ops::AddAssign for Duration {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl std::ops::SubAssign for Duration {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

impl std::ops::MulAssign<u32> for Duration {
    #[inline]
    fn mul_assign(&mut self, rhs: u32) {
        self.0 *= rhs;
    }
}

impl std::ops::DivAssign<u32> for Duration {
    #[inline]
    fn div_assign(&mut self, rhs: u32) {
        self.0 /= rhs;
    }
}

// ==================== Serde Support ====================

#[cfg(feature = "serde")]
impl serde::Serialize for Duration {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as milliseconds
        serializer.serialize_u64(self.as_millis() as u64)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Duration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{self, Visitor};

        struct DurationVisitor;

        impl<'de> Visitor<'de> for DurationVisitor {
            type Value = Duration;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a duration in milliseconds or a string like '5s'")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Duration, E>
            where
                E: de::Error,
            {
                Ok(Duration::from_millis(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Duration, E>
            where
                E: de::Error,
            {
                if value < 0 {
                    Err(E::custom("duration cannot be negative"))
                } else {
                    Ok(Duration::from_millis(value as u64))
                }
            }

            fn visit_f64<E>(self, value: f64) -> Result<Duration, E>
            where
                E: de::Error,
            {
                if value < 0.0 {
                    Err(E::custom("duration cannot be negative"))
                } else {
                    Ok(Duration::from_secs_f64(value / 1000.0))
                }
            }

            fn visit_str<E>(self, value: &str) -> Result<Duration, E>
            where
                E: de::Error,
            {
                // Parse strings like "5s", "100ms", "2h", "1d"
                let (num_str, unit) = value.split_at(
                    value.rfind(|c: char| c.is_ascii_digit() || c == '.').map_or(0, |i| i + 1)
                );

                let num: f64 = num_str.parse()
                    .map_err(|_| E::custom(format!("invalid number: {}", num_str)))?;

                let duration = match unit {
                    "ns" => Duration::from_nanos(num as u64),
                    "μs" | "us" => Duration::from_micros(num as u64),
                    "ms" => Duration::from_millis(num as u64),
                    "s" | "" => Duration::from_secs_f64(num),
                    "m" => Duration::from_secs_f64(num * 60.0),
                    "h" => Duration::from_secs_f64(num * 3600.0),
                    "d" => Duration::from_secs_f64(num * 86400.0),
                    _ => return Err(E::custom(format!("unknown time unit: {}", unit))),
                };

                Ok(duration)
            }
        }

        deserializer.deserialize_any(DurationVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_construction() {
        assert_eq!(Duration::ZERO.as_nanos(), 0);
        assert_eq!(Duration::from_secs(5).as_secs(), 5);
        assert_eq!(Duration::from_millis(1500).as_millis(), 1500);
        assert_eq!(Duration::from_minutes(2).as_secs(), 120);
        assert_eq!(Duration::from_hours(1).as_secs(), 3600);
        assert_eq!(Duration::from_days(1).as_secs(), 86400);
    }

    #[test]
    fn test_formatting() {
        assert_eq!(Duration::ZERO.to_compact_string(), "0s");
        assert_eq!(Duration::from_millis(500).to_compact_string(), "500ms");
        assert_eq!(Duration::from_secs(1).to_compact_string(), "1.0s");
        assert_eq!(Duration::from_secs(90).to_compact_string(), "1.5m");
        assert_eq!(Duration::from_hours(25).to_compact_string(), "1.0d");

        assert_eq!(Duration::from_secs(1).to_human_string(), "1 second");
        assert_eq!(Duration::from_secs(61).to_human_string(), "1 minute 1 second");
        assert_eq!(Duration::from_secs(3661).to_human_string(), "1 hour 1 minute 1 second");
    }

    #[test]
    fn test_operations() {
        let d1 = Duration::from_secs(10);
        let d2 = Duration::from_secs(5);

        assert_eq!((d1 + d2).as_secs(), 15);
        assert_eq!((d1 - d2).as_secs(), 5);
        assert_eq!((d1 * 2).as_secs(), 20);
        assert_eq!((d1 / 2).as_secs(), 5);

        assert_eq!(d1.min(d2), d2);
        assert_eq!(d1.max(d2), d1);
        assert_eq!(d1.abs_diff(d2).as_secs(), 5);
    }

    #[test]
    fn test_checked_operations() {
        let d = Duration::from_secs(10);

        assert_eq!(d.checked_add(Duration::from_secs(5)).unwrap().as_secs(), 15);
        assert_eq!(d.checked_sub(Duration::from_secs(5)).unwrap().as_secs(), 5);
        assert_eq!(d.checked_mul(2).unwrap().as_secs(), 20);
        assert_eq!(d.checked_div(2).unwrap().as_secs(), 5);

        assert!(d.checked_sub(Duration::from_secs(20)).is_none());
        assert!(Duration::MAX.checked_add(Duration::from_secs(1)).is_none());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_serde() {
        let d = Duration::from_millis(1500);
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(json, "1500");

        let d2: Duration = serde_json::from_str(&json).unwrap();
        assert_eq!(d, d2);

        let d3: Duration = serde_json::from_str("\"5s\"").unwrap();
        assert_eq!(d3.as_secs(), 5);

        let d4: Duration = serde_json::from_str("\"100ms\"").unwrap();
        assert_eq!(d4.as_millis(), 100);
    }
}