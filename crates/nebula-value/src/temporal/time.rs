extern crate alloc;

use alloc::sync::Arc;
use core::borrow::Borrow;
use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::{Add, AddAssign, Sub, SubAssign};
use core::str::FromStr;
use core::time::Duration as StdDuration;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use once_cell::sync::OnceCell;

use chrono::{NaiveTime, Timelike};

use crate::core::{ValueError, ValueResult};

/// Internal time storage (nanoseconds since midnight)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TimeInner {
    pub nanos: u64, // Nanoseconds since midnight (0 to 86_399_999_999_999)
}

impl TimeInner {
    /// Maximum nanoseconds in a day
    const MAX_NANOS: u64 = 86_399_999_999_999;

    /// Nanoseconds per second
    const NANOS_PER_SECOND: u64 = 1_000_000_000;

    /// Nanoseconds per minute
    const NANOS_PER_MINUTE: u64 = 60 * Self::NANOS_PER_SECOND;

    /// Nanoseconds per hour
    const NANOS_PER_HOUR: u64 = 60 * Self::NANOS_PER_MINUTE;

    /// Creates a new TimeInner with validation
    pub fn new(hour: u32, minute: u32, second: u32, nanos: u32) -> ValueResult<Self> {
        // Validate components
        if hour >= 24 {
            return Err(ValueError::validation(format!("Invalid hour: {}", hour)));
        }

        if minute >= 60 {
            return Err(ValueError::validation(format!(
                "Invalid minute: {}",
                minute
            )));
        }

        if second >= 60 {
            return Err(ValueError::validation(format!(
                "Invalid second: {}",
                second
            )));
        }

        if nanos >= Self::NANOS_PER_SECOND as u32 {
            return Err(ValueError::validation(format!(
                "Invalid nanoseconds: {}",
                nanos
            )));
        }

        let total_nanos = hour as u64 * Self::NANOS_PER_HOUR
            + minute as u64 * Self::NANOS_PER_MINUTE
            + second as u64 * Self::NANOS_PER_SECOND
            + nanos as u64;

        Ok(Self { nanos: total_nanos })
    }

    /// Creates a new TimeInner without validation (for compile-time constants)
    ///
    /// # Safety
    ///
    /// Caller must guarantee that:
    /// - `hour < 24`
    /// - `minute < 60`
    /// - `second < 60`
    /// - `nanos < 1_000_000_000`
    #[inline]
    pub const unsafe fn new_unchecked(hour: u32, minute: u32, second: u32, nanos: u32) -> Self {
        // Debug assertions to catch invalid usage during development
        debug_assert!(hour < 24, "hour must be < 24");
        debug_assert!(minute < 60, "minute must be < 60");
        debug_assert!(second < 60, "second must be < 60");
        debug_assert!(
            nanos < Self::NANOS_PER_SECOND as u32,
            "nanos must be < 1_000_000_000"
        );

        let total_nanos = hour as u64 * Self::NANOS_PER_HOUR
            + minute as u64 * Self::NANOS_PER_MINUTE
            + second as u64 * Self::NANOS_PER_SECOND
            + nanos as u64;

        Self { nanos: total_nanos }
    }

    /// Creates from total nanoseconds since midnight
    pub fn from_nanos(nanos: u64) -> ValueResult<Self> {
        if nanos > Self::MAX_NANOS {
            return Err(ValueError::validation(format!(
                "Nanoseconds {} exceeds max {}",
                nanos,
                Self::MAX_NANOS
            )));
        }
        Ok(Self { nanos })
    }

    /// Creates from total nanoseconds without validation
    ///
    /// # Safety
    ///
    /// Caller must guarantee that `nanos <= 86_399_999_999_999`
    #[inline]
    pub const unsafe fn from_nanos_unchecked(nanos: u64) -> Self {
        debug_assert!(nanos <= Self::MAX_NANOS, "nanos must be <= MAX_NANOS");
        Self { nanos }
    }

    /// Returns the hour component (0-23)
    #[inline]
    pub fn hour(&self) -> u8 {
        (self.nanos / Self::NANOS_PER_HOUR) as u8
    }

    /// Returns the minute component (0-59)
    #[inline]
    pub fn minute(&self) -> u8 {
        ((self.nanos % Self::NANOS_PER_HOUR) / Self::NANOS_PER_MINUTE) as u8
    }

    /// Returns the second component (0-59)
    #[inline]
    pub fn second(&self) -> u8 {
        ((self.nanos % Self::NANOS_PER_MINUTE) / Self::NANOS_PER_SECOND) as u8
    }

    /// Returns the nanosecond component (0-999_999_999)
    #[inline]
    pub fn nanosecond(&self) -> u32 {
        (self.nanos % Self::NANOS_PER_SECOND) as u32
    }

    /// Returns the microsecond component (0-999_999)
    #[inline]
    pub fn microsecond(&self) -> u32 {
        self.nanosecond() / 1000
    }

    /// Returns the millisecond component (0-999)
    #[inline]
    pub fn millisecond(&self) -> u32 {
        self.nanosecond() / 1_000_000
    }

    /// Converts to chrono NaiveTime
    pub fn to_naive(&self) -> NaiveTime {
        // SAFETY: TimeInner guarantees nanos <= MAX_NANOS, which ensures
        // seconds < 86400 and nanos < 1_000_000_000, both valid for NaiveTime
        NaiveTime::from_num_seconds_from_midnight_opt(
            (self.nanos / Self::NANOS_PER_SECOND) as u32,
            (self.nanos % Self::NANOS_PER_SECOND) as u32,
        )
        .unwrap()
    }

    /// Creates from chrono NaiveTime
    pub fn from_naive(time: NaiveTime) -> Self {
        let nanos = time.num_seconds_from_midnight() as u64 * Self::NANOS_PER_SECOND
            + time.nanosecond() as u64;
        Self { nanos }
    }

    /// Adds seconds with wrapping
    pub fn add_seconds_wrapping(&self, seconds: i64) -> Self {
        let seconds_mod = seconds.rem_euclid(86400) as u64;
        let new_nanos = (self.nanos + seconds_mod * Self::NANOS_PER_SECOND) % (Self::MAX_NANOS + 1);
        Self { nanos: new_nanos }
    }
}

/// A high-performance, feature-rich time type
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Time {
    /// Internal time storage
    inner: Arc<TimeInner>,

    /// Cached ISO string for O(1) access
    #[cfg_attr(feature = "serde", serde(skip))]
    iso_string_cache: OnceCell<String>,

    /// Cached 12-hour format string
    #[cfg_attr(feature = "serde", serde(skip))]
    format_12h_cache: OnceCell<String>,
}

impl Time {
    // ==================== Constructors ====================

    /// Creates a new Time
    pub fn new(hour: u32, minute: u32, second: u32) -> ValueResult<Self> {
        Self::with_nanos(hour, minute, second, 0)
    }

    /// Creates a new Time with nanoseconds
    pub fn with_nanos(hour: u32, minute: u32, second: u32, nanos: u32) -> ValueResult<Self> {
        let inner = TimeInner::new(hour, minute, second, nanos)?;
        Ok(Self {
            inner: Arc::new(inner),
            iso_string_cache: OnceCell::new(),
            format_12h_cache: OnceCell::new(),
        })
    }

    /// Creates a new Time with milliseconds
    pub fn with_millis(hour: u32, minute: u32, second: u32, millis: u32) -> ValueResult<Self> {
        if millis >= 1000 {
            return Err(ValueError::validation(format!(
                "Invalid milliseconds: {}",
                millis
            )));
        }
        Self::with_nanos(hour, minute, second, millis * 1_000_000)
    }

    /// Creates a new Time with microseconds
    pub fn with_micros(hour: u32, minute: u32, second: u32, micros: u32) -> ValueResult<Self> {
        if micros >= 1_000_000 {
            return Err(ValueError::validation(format!(
                "Invalid microseconds: {}",
                micros
            )));
        }
        Self::with_nanos(hour, minute, second, micros * 1000)
    }

    /// Creates Time at midnight (00:00:00)
    pub fn midnight() -> Self {
        Self {
            inner: Arc::new(TimeInner { nanos: 0 }),
            iso_string_cache: OnceCell::new(),
            format_12h_cache: OnceCell::new(),
        }
    }

    /// Creates Time at noon (12:00:00)
    pub fn noon() -> Self {
        // SAFETY: 12:00:00 is always valid (hour=12 < 24, minute=0 < 60, second=0 < 60)
        Self {
            inner: Arc::new(unsafe { TimeInner::new_unchecked(12, 0, 0, 0) }),
            iso_string_cache: OnceCell::new(),
            format_12h_cache: OnceCell::new(),
        }
    }

    /// Creates Time at end of day (23:59:59.999999999)
    pub fn end_of_day() -> Self {
        Self {
            inner: Arc::new(TimeInner {
                nanos: TimeInner::MAX_NANOS,
            }),
            iso_string_cache: OnceCell::new(),
            format_12h_cache: OnceCell::new(),
        }
    }

    /// Creates Time for current time (local timezone)
    #[cfg(feature = "std")]
    pub fn now() -> Self {
        let now = chrono::Local::now().time();
        Self::from_naive_time(now)
    }

    /// Creates Time for current time (UTC)
    #[cfg(feature = "std")]
    pub fn now_utc() -> Self {
        let now = chrono::Utc::now().time();
        Self::from_naive_time(now)
    }

    /// Creates from seconds since midnight
    pub fn from_seconds(seconds: u32) -> ValueResult<Self> {
        if seconds >= 86400 {
            return Err(ValueError::validation(format!(
                "Seconds {} >= 86400",
                seconds
            )));
        }

        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        let secs = seconds % 60;

        Self::new(hours, minutes, secs)
    }

    /// Creates from milliseconds since midnight
    pub fn from_millis(millis: u64) -> ValueResult<Self> {
        if millis >= 86_400_000 {
            return Err(ValueError::validation(format!(
                "Milliseconds {} >= 86400000",
                millis
            )));
        }

        let nanos = millis * 1_000_000;
        Ok(Self {
            inner: Arc::new(TimeInner { nanos }),
            iso_string_cache: OnceCell::new(),
            format_12h_cache: OnceCell::new(),
        })
    }

    /// Creates from nanoseconds since midnight
    pub fn from_nanos(nanos: u64) -> ValueResult<Self> {
        let inner = TimeInner::from_nanos(nanos)?;
        Ok(Self {
            inner: Arc::new(inner),
            iso_string_cache: OnceCell::new(),
            format_12h_cache: OnceCell::new(),
        })
    }

    /// Creates from chrono NaiveTime
    pub fn from_naive_time(time: NaiveTime) -> Self {
        Self {
            inner: Arc::new(TimeInner::from_naive(time)),
            iso_string_cache: OnceCell::new(),
            format_12h_cache: OnceCell::new(),
        }
    }

    /// Parses from a string in format HH:MM:SS[.nanos]
    pub fn parse_iso(s: &str) -> ValueResult<Self> {
        // Split by '.' for fractional seconds
        let (time_part, nano_part) = if let Some(dot_pos) = s.find('.') {
            let nano_str = &s[dot_pos + 1..];
            let nanos = if nano_str.len() <= 9 {
                let padded = format!("{:0<9}", nano_str);
                padded.parse::<u32>().map_err(|_| {
                    ValueError::validation(format!("Invalid nanoseconds: {}", nano_str))
                })?
            } else {
                return Err(ValueError::validation(format!(
                    "Too many decimal places: {}",
                    nano_str
                )));
            };
            (&s[..dot_pos], nanos)
        } else {
            (s, 0)
        };

        // Parse HH:MM:SS
        let parts: Vec<_> = time_part.split(':').collect();
        if parts.len() < 2 || parts.len() > 3 {
            return Err(ValueError::validation(format!(
                "Invalid time format: {}",
                s
            )));
        }

        let hour = parts[0]
            .parse::<u32>()
            .map_err(|_| ValueError::validation(format!("Invalid hour: {}", parts[0])))?;

        let minute = parts[1]
            .parse::<u32>()
            .map_err(|_| ValueError::validation(format!("Invalid minute: {}", parts[1])))?;

        let second = if parts.len() == 3 {
            parts[2]
                .parse::<u32>()
                .map_err(|_| ValueError::validation(format!("Invalid second: {}", parts[2])))?
        } else {
            0
        };

        Self::with_nanos(hour, minute, second, nano_part)
    }

    /// Parses time in 12-hour format (e.g., "3:30 PM", "11:45:30 AM")
    pub fn parse_12h(s: &str) -> ValueResult<Self> {
        let s = s.trim();

        // Find AM/PM
        let (time_part, is_pm) = if s.to_uppercase().ends_with(" AM") {
            (&s[..s.len() - 3], false)
        } else if s.to_uppercase().ends_with(" PM") {
            (&s[..s.len() - 3], true)
        } else {
            return Err(ValueError::validation("Missing AM/PM indicator"));
        };

        // Parse the time part
        let parts: Vec<&str> = time_part.trim().split(':').collect();
        if parts.is_empty() || parts.len() > 3 {
            return Err(ValueError::validation(format!(
                "Invalid 12-hour format: {}",
                s
            )));
        }

        let hour12 = parts[0]
            .parse::<u32>()
            .map_err(|_| ValueError::validation(format!("Invalid hour: {}", parts[0])))?;

        if hour12 == 0 || hour12 > 12 {
            return Err(ValueError::validation(format!(
                "Invalid hour (12-hour format): {}",
                hour12
            )));
        }

        let minute = if parts.len() > 1 {
            parts[1]
                .parse::<u32>()
                .map_err(|_| ValueError::validation(format!("Invalid minute: {}", parts[1])))?
        } else {
            0
        };

        let second = if parts.len() > 2 {
            parts[2]
                .parse::<u32>()
                .map_err(|_| ValueError::validation(format!("Invalid second: {}", parts[2])))?
        } else {
            0
        };

        // Convert to 24-hour format
        let hour24 = if hour12 == 12 {
            if is_pm { 12 } else { 0 }
        } else if is_pm {
            hour12 + 12
        } else {
            hour12
        };

        Self::new(hour24, minute, second)
    }

    // ==================== Basic Properties ====================

    /// Returns the hour (0-23)
    #[inline]
    pub fn hour(&self) -> u32 {
        self.inner.hour() as u32
    }

    /// Returns the minute (0-59)
    #[inline]
    pub fn minute(&self) -> u32 {
        self.inner.minute() as u32
    }

    /// Returns the second (0-59)
    #[inline]
    pub fn second(&self) -> u32 {
        self.inner.second() as u32
    }

    /// Returns the nanosecond (0-999_999_999)
    #[inline]
    pub fn nanosecond(&self) -> u32 {
        self.inner.nanosecond()
    }

    /// Returns the millisecond (0-999)
    #[inline]
    pub fn millisecond(&self) -> u32 {
        self.inner.millisecond()
    }

    /// Returns the microsecond (0-999_999)
    #[inline]
    pub fn microsecond(&self) -> u32 {
        self.inner.microsecond()
    }

    /// Returns total seconds since midnight
    #[inline]
    pub fn total_seconds(&self) -> u32 {
        (self.inner.nanos / TimeInner::NANOS_PER_SECOND) as u32
    }

    /// Returns total milliseconds since midnight
    #[inline]
    pub fn total_millis(&self) -> u64 {
        self.inner.nanos / 1_000_000
    }

    /// Returns total microseconds since midnight
    #[inline]
    pub fn total_micros(&self) -> u64 {
        self.inner.nanos / 1000
    }

    /// Returns total nanoseconds since midnight
    #[inline]
    pub fn total_nanos(&self) -> u64 {
        self.inner.nanos
    }

    /// Returns hour in 12-hour format (1-12)
    pub fn hour_12(&self) -> u32 {
        let h = self.hour();
        if h == 0 {
            12
        } else if h > 12 {
            h - 12
        } else {
            h
        }
    }

    /// Returns true if time is AM
    #[inline]
    pub fn is_am(&self) -> bool {
        self.hour() < 12
    }

    /// Returns true if time is PM
    #[inline]
    pub fn is_pm(&self) -> bool {
        !self.is_am()
    }

    /// Returns AM/PM string
    #[inline]
    pub fn am_pm(&self) -> &'static str {
        if self.is_am() { "AM" } else { "PM" }
    }

    /// Gets the internal representation
    #[inline]
    pub(crate) fn as_inner(&self) -> &TimeInner {
        &self.inner
    }

    // ==================== Time Arithmetic ====================

    /// Adds a duration to the time
    pub fn add_duration(&self, duration: StdDuration) -> ValueResult<Self> {
        let total_nanos = duration.as_nanos();
        if total_nanos > TimeInner::MAX_NANOS as u128 {
            return Err(ValueError::validation(
                "Duration too large for time operation",
            ));
        }

        let new_nanos = (self.inner.nanos + total_nanos as u64) % (TimeInner::MAX_NANOS + 1);
        Self::from_nanos(new_nanos)
    }

    /// Subtracts a duration from the time
    pub fn sub_duration(&self, duration: StdDuration) -> ValueResult<Self> {
        let total_nanos = duration.as_nanos();
        if total_nanos > TimeInner::MAX_NANOS as u128 {
            return Err(ValueError::validation(
                "Duration too large for time operation",
            ));
        }

        let sub_nanos = total_nanos as u64 % (TimeInner::MAX_NANOS + 1);
        let new_nanos = if self.inner.nanos >= sub_nanos {
            self.inner.nanos - sub_nanos
        } else {
            TimeInner::MAX_NANOS + 1 - (sub_nanos - self.inner.nanos)
        };

        Self::from_nanos(new_nanos)
    }

    /// Adds hours to the time (with wrapping)
    pub fn add_hours(&self, hours: i32) -> Self {
        let total_hours = (self.hour() as i32 + hours).rem_euclid(24);
        Self::new(total_hours as u32, self.minute(), self.second())
            .expect("rem_euclid(24) guarantees hour is 0-23")
    }

    /// Adds minutes to the time (with wrapping)
    pub fn add_minutes(&self, minutes: i32) -> Self {
        let total_minutes = self.hour() as i32 * 60 + self.minute() as i32 + minutes;
        let new_minutes = total_minutes.rem_euclid(1440);
        let new_hours = (new_minutes / 60) as u32;
        let new_mins = (new_minutes % 60) as u32;
        Self::new(new_hours, new_mins, self.second())
            .expect("rem_euclid(1440) and modulo arithmetic guarantee valid time components")
    }

    /// Adds seconds to the time (with wrapping)
    pub fn add_seconds(&self, seconds: i64) -> Self {
        let inner = self.inner.add_seconds_wrapping(seconds);
        Self {
            inner: Arc::new(inner),
            iso_string_cache: OnceCell::new(),
            format_12h_cache: OnceCell::new(),
        }
    }

    /// Returns the duration between two times
    pub fn duration_between(&self, other: &Time) -> StdDuration {
        if self.inner.nanos >= other.inner.nanos {
            StdDuration::from_nanos(self.inner.nanos - other.inner.nanos)
        } else {
            StdDuration::from_nanos(other.inner.nanos - self.inner.nanos)
        }
    }

    /// Returns signed duration to another time
    pub fn signed_duration_to(&self, other: &Time) -> i64 {
        other.inner.nanos as i64 - self.inner.nanos as i64
    }

    /// Rounds to the nearest hour
    pub fn round_to_hour(&self) -> Self {
        if self.minute() >= 30 {
            self.add_hours(1)
                .with_minute(0)
                .expect("0 is always a valid minute")
                .with_second(0)
                .expect("0 is always a valid second")
        } else {
            self.with_minute(0)
                .expect("0 is always a valid minute")
                .with_second(0)
                .expect("0 is always a valid second")
        }
    }

    /// Rounds to the nearest minute
    pub fn round_to_minute(&self) -> Self {
        if self.second() >= 30 {
            self.add_minutes(1)
                .with_second(0)
                .expect("0 is always a valid second")
        } else {
            self.with_second(0).expect("0 is always a valid second")
        }
    }

    /// Rounds to the nearest second
    pub fn round_to_second(&self) -> Self {
        if self.nanosecond() >= 500_000_000 {
            self.add_seconds(1)
                .with_nanosecond(0)
                .expect("0 is always a valid nanosecond")
        } else {
            self.with_nanosecond(0)
                .expect("0 is always a valid nanosecond")
        }
    }

    /// Truncates to hour precision
    pub fn truncate_to_hour(&self) -> Self {
        Self::new(self.hour(), 0, 0)
            .expect("existing hour with 0 minute and 0 second is always valid")
    }

    /// Truncates to minute precision
    pub fn truncate_to_minute(&self) -> Self {
        Self::new(self.hour(), self.minute(), 0)
            .expect("existing hour and minute with 0 second is always valid")
    }

    /// Truncates to second precision
    pub fn truncate_to_second(&self) -> Self {
        Self::with_nanos(self.hour(), self.minute(), self.second(), 0)
            .expect("existing time components with 0 nanosecond is always valid")
    }

    // ==================== Component Updates ====================

    /// Returns a new Time with the specified hour
    pub fn with_hour(&self, hour: u32) -> ValueResult<Self> {
        Self::with_nanos(hour, self.minute(), self.second(), self.nanosecond())
    }

    /// Returns a new Time with the specified minute
    pub fn with_minute(&self, minute: u32) -> ValueResult<Self> {
        Self::with_nanos(self.hour(), minute, self.second(), self.nanosecond())
    }

    /// Returns a new Time with the specified second
    pub fn with_second(&self, second: u32) -> ValueResult<Self> {
        Self::with_nanos(self.hour(), self.minute(), second, self.nanosecond())
    }

    /// Returns a new Time with the specified nanosecond
    pub fn with_nanosecond(&self, nano: u32) -> ValueResult<Self> {
        Self::with_nanos(self.hour(), self.minute(), self.second(), nano)
    }

    // ==================== Formatting ====================

    /// Returns ISO 8601 time string (HH:MM:SS[.nanos])
    pub fn to_iso_string(&self) -> &str {
        self.iso_string_cache.get_or_init(|| {
            if self.nanosecond() == 0 {
                format!(
                    "{:02}:{:02}:{:02}",
                    self.hour(),
                    self.minute(),
                    self.second()
                )
            } else {
                let nanos = format!("{:09}", self.nanosecond());
                let trimmed = nanos.trim_end_matches('0');
                format!(
                    "{:02}:{:02}:{:02}.{}",
                    self.hour(),
                    self.minute(),
                    self.second(),
                    trimmed
                )
            }
        })
    }

    /// Returns 12-hour format string (e.g., "3:30:00 PM")
    pub fn to_12h_string(&self) -> &str {
        self.format_12h_cache.get_or_init(|| {
            format!(
                "{}:{:02}:{:02} {}",
                self.hour_12(),
                self.minute(),
                self.second(),
                self.am_pm()
            )
        })
    }

    /// Formats the time using a custom format string
    pub fn format(&self, fmt: &str) -> String {
        let chars: Vec<char> = fmt.chars().collect();
        let mut i = 0;
        let mut out = String::with_capacity(fmt.len() + 8);
        while i < chars.len() {
            #[allow(clippy::excessive_nesting)]
            let starts_with = |tok: &str| -> bool {
                let tchars: Vec<char> = tok.chars().collect();
                if i + tchars.len() > chars.len() {
                    return false;
                }
                for (k, ch) in tchars.iter().enumerate() {
                    if chars[i + k] != *ch {
                        return false;
                    }
                }
                true
            };
            // Helper to ensure single-letter tokens A/a are not inside words
            let at_word_boundary = |idx: usize| -> bool {
                let prev = if idx == 0 { None } else { Some(chars[idx - 1]) };
                let next = if idx + 1 >= chars.len() {
                    None
                } else {
                    Some(chars[idx + 1])
                };
                let is_alpha =
                    |c: Option<char>| c.map(|ch| ch.is_ascii_alphabetic()).unwrap_or(false);
                !is_alpha(prev) && !is_alpha(next)
            };

            if starts_with("SSSSSSSSS") {
                out.push_str(&format!("{:09}", self.nanosecond()));
                i += 9;
            } else if starts_with("SSSSSS") {
                out.push_str(&format!("{:06}", self.microsecond()));
                i += 6;
            } else if starts_with("SSS") {
                out.push_str(&format!("{:03}", self.millisecond()));
                i += 3;
            } else if starts_with("HH") {
                out.push_str(&format!("{:02}", self.hour()));
                i += 2;
            } else if starts_with("hh") {
                out.push_str(&format!("{:02}", self.hour_12()));
                i += 2;
            } else if starts_with("mm") {
                out.push_str(&format!("{:02}", self.minute()));
                i += 2;
            } else if starts_with("ss") {
                out.push_str(&format!("{:02}", self.second()));
                i += 2;
            } else if starts_with("H") {
                out.push_str(&self.hour().to_string());
                i += 1;
            } else if starts_with("h") {
                out.push_str(&self.hour_12().to_string());
                i += 1;
            } else if starts_with("m") {
                out.push_str(&self.minute().to_string());
                i += 1;
            } else if starts_with("s") {
                out.push_str(&self.second().to_string());
                i += 1;
            } else if starts_with("A") && at_word_boundary(i) {
                out.push_str(self.am_pm());
                i += 1;
            } else if starts_with("a") && at_word_boundary(i) {
                out.push_str(&self.am_pm().to_lowercase());
                i += 1;
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        out
    }

    /// Returns a human-readable string
    pub fn to_human_string(&self) -> String {
        if self.second() == 0 && self.nanosecond() == 0 {
            format!("{}:{:02} {}", self.hour_12(), self.minute(), self.am_pm())
        } else {
            self.to_12h_string().to_string()
        }
    }

    /// Returns relative time string (e.g., "in 5 minutes", "2 hours ago")
    #[cfg(feature = "std")]
    pub fn to_relative_string(&self) -> String {
        let now = Time::now();
        let diff_nanos = self.signed_duration_to(&now);
        let diff_seconds = diff_nanos / 1_000_000_000;
        let abs_seconds = diff_seconds.abs();

        let (amount, unit) = if abs_seconds < 60 {
            (abs_seconds, "second")
        } else if abs_seconds < 3600 {
            (abs_seconds / 60, "minute")
        } else {
            (abs_seconds / 3600, "hour")
        };

        let plural = if amount == 1 { "" } else { "s" };

        if diff_seconds > 0 {
            format!("in {} {}{}", amount, unit, plural)
        } else if diff_seconds < 0 {
            format!("{} {}{} ago", amount, unit, plural)
        } else {
            "now".to_string()
        }
    }

    // ==================== Conversions ====================

    /// Converts to chrono NaiveTime
    #[inline]
    pub fn to_naive(&self) -> NaiveTime {
        self.inner.to_naive()
    }

    /// Converts to std::time::Duration since midnight
    #[inline]
    pub fn to_duration(&self) -> StdDuration {
        StdDuration::from_nanos(self.inner.nanos)
    }

    // ==================== Validation ====================

    /// Checks if a time is valid
    pub fn is_valid(hour: u32, minute: u32, second: u32) -> bool {
        hour < 24 && minute < 60 && second < 60
    }

    /// Checks if the time is in the morning (00:00 - 11:59)
    #[inline]
    pub fn is_morning(&self) -> bool {
        self.hour() < 12
    }

    /// Checks if the time is in the afternoon (12:00 - 17:59)
    #[inline]
    pub fn is_afternoon(&self) -> bool {
        self.hour() >= 12 && self.hour() < 18
    }

    /// Checks if the time is in the evening (18:00 - 20:59)
    #[inline]
    pub fn is_evening(&self) -> bool {
        self.hour() >= 18 && self.hour() < 21
    }

    /// Checks if the time is at night (21:00 - 23:59 or 00:00 - 05:59)
    #[inline]
    pub fn is_night(&self) -> bool {
        self.hour() >= 21 || self.hour() < 6
    }

    /// Checks if this is exactly midnight
    #[inline]
    pub fn is_midnight(&self) -> bool {
        self.inner.nanos == 0
    }

    /// Checks if this is exactly noon
    #[inline]
    pub fn is_noon(&self) -> bool {
        self.hour() == 12 && self.minute() == 0 && self.second() == 0 && self.nanosecond() == 0
    }

    /// Checks if the time is between two other times
    pub fn is_between(&self, start: &Time, end: &Time) -> bool {
        if start <= end {
            self >= start && self <= end
        } else {
            // Handle wraparound (e.g., 23:00 to 01:00)
            self >= start || self <= end
        }
    }
}

// ==================== Trait Implementations ====================

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_iso_string())
    }
}

impl Default for Time {
    fn default() -> Self {
        Self::midnight()
    }
}

impl FromStr for Time {
    type Err = ValueError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Try ISO format first, then 12-hour format
        Self::parse_iso(s).or_else(|_| Self::parse_12h(s))
    }
}

impl PartialEq for Time {
    fn eq(&self, other: &Self) -> bool {
        self.inner.nanos == other.inner.nanos
    }
}

impl Eq for Time {}

impl PartialOrd for Time {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Time {
    fn cmp(&self, other: &Self) -> Ordering {
        self.inner.nanos.cmp(&other.inner.nanos)
    }
}

impl Hash for Time {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.nanos.hash(state);
    }
}

impl Borrow<TimeInner> for Time {
    fn borrow(&self) -> &TimeInner {
        &self.inner
    }
}

// ==================== From Implementations ====================

impl From<StdDuration> for Time {
    fn from(duration: StdDuration) -> Self {
        let nanos = duration.as_nanos() as u64;
        Self::from_nanos(nanos % (TimeInner::MAX_NANOS + 1))
            .expect("modulo MAX_NANOS+1 guarantees value is within valid time range")
    }
}

impl From<Time> for StdDuration {
    fn from(time: Time) -> Self {
        StdDuration::from_nanos(time.inner.nanos)
    }
}

impl From<NaiveTime> for Time {
    fn from(time: NaiveTime) -> Self {
        Self::from_naive_time(time)
    }
}

impl From<Time> for NaiveTime {
    fn from(time: Time) -> Self {
        time.to_naive()
    }
}

// ==================== Arithmetic Operations ====================

impl Add<StdDuration> for Time {
    type Output = ValueResult<Time>;

    fn add(self, duration: StdDuration) -> Self::Output {
        self.add_duration(duration)
    }
}

impl Add<StdDuration> for &Time {
    type Output = ValueResult<Time>;

    fn add(self, duration: StdDuration) -> Self::Output {
        self.add_duration(duration)
    }
}

impl Sub<StdDuration> for Time {
    type Output = ValueResult<Time>;

    fn sub(self, duration: StdDuration) -> Self::Output {
        self.sub_duration(duration)
    }
}

impl Sub<StdDuration> for &Time {
    type Output = ValueResult<Time>;

    fn sub(self, duration: StdDuration) -> Self::Output {
        self.sub_duration(duration)
    }
}

impl Sub<Time> for Time {
    type Output = StdDuration;

    fn sub(self, other: Time) -> Self::Output {
        self.duration_between(&other)
    }
}

impl Sub<&Time> for Time {
    type Output = StdDuration;

    fn sub(self, other: &Time) -> Self::Output {
        self.duration_between(other)
    }
}

impl Sub<Time> for &Time {
    type Output = StdDuration;

    fn sub(self, other: Time) -> Self::Output {
        self.duration_between(&other)
    }
}

impl Sub<&Time> for &Time {
    type Output = StdDuration;

    fn sub(self, other: &Time) -> Self::Output {
        self.duration_between(other)
    }
}

impl AddAssign<StdDuration> for Time {
    fn add_assign(&mut self, duration: StdDuration) {
        if let Ok(new_time) = self.add_duration(duration) {
            *self = new_time;
        }
    }
}

impl SubAssign<StdDuration> for Time {
    fn sub_assign(&mut self, duration: StdDuration) {
        if let Ok(new_time) = self.sub_duration(duration) {
            *self = new_time;
        }
    }
}

// ==================== Send + Sync ====================

// Static assertions to ensure inner types are Send + Sync
// This catches issues at compile time if inner types change
static_assertions::assert_impl_all!(TimeInner: Send, Sync);
static_assertions::assert_impl_all!(alloc::sync::Arc<TimeInner>: Send, Sync);

unsafe impl Send for Time {}
unsafe impl Sync for Time {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_creation() {
        let time = Time::new(14, 30, 45).unwrap();
        assert_eq!(time.hour(), 14);
        assert_eq!(time.minute(), 30);
        assert_eq!(time.second(), 45);

        let with_millis = Time::with_millis(10, 15, 20, 500).unwrap();
        assert_eq!(with_millis.millisecond(), 500);

        assert!(Time::new(24, 0, 0).is_err());
        assert!(Time::new(0, 60, 0).is_err());
        assert!(Time::new(0, 0, 60).is_err());
    }

    #[test]
    fn test_special_times() {
        let midnight = Time::midnight();
        assert_eq!(midnight.hour(), 0);
        assert_eq!(midnight.minute(), 0);
        assert_eq!(midnight.second(), 0);
        assert!(midnight.is_midnight());

        let noon = Time::noon();
        assert_eq!(noon.hour(), 12);
        assert!(noon.is_noon());

        let eod = Time::end_of_day();
        assert_eq!(eod.hour(), 23);
        assert_eq!(eod.minute(), 59);
        assert_eq!(eod.second(), 59);
        assert_eq!(eod.nanosecond(), 999_999_999);
    }

    #[test]
    fn test_12_hour_format() {
        let morning = Time::new(9, 30, 0).unwrap();
        assert_eq!(morning.hour_12(), 9);
        assert!(morning.is_am());
        assert_eq!(morning.am_pm(), "AM");

        let afternoon = Time::new(15, 45, 30).unwrap();
        assert_eq!(afternoon.hour_12(), 3);
        assert!(afternoon.is_pm());
        assert_eq!(afternoon.am_pm(), "PM");

        let midnight = Time::midnight();
        assert_eq!(midnight.hour_12(), 12);
        assert!(midnight.is_am());

        let noon = Time::noon();
        assert_eq!(noon.hour_12(), 12);
        assert!(noon.is_pm());
    }

    #[test]
    fn test_arithmetic() {
        let time = Time::new(10, 30, 45).unwrap();

        // Add hours
        let plus_2h = time.add_hours(2);
        assert_eq!(plus_2h.hour(), 12);

        // Add with wraparound
        let fifteen_hours_later = time.add_hours(15);
        assert_eq!(fifteen_hours_later.hour(), 1);

        // Add minutes
        let forty_five_minutes_later = time.add_minutes(45);
        assert_eq!(forty_five_minutes_later.hour(), 11);
        assert_eq!(forty_five_minutes_later.minute(), 15);

        // Add seconds
        let thirty_seconds_later = time.add_seconds(30);
        assert_eq!(thirty_seconds_later.second(), 15);

        // Add with wraparound
        let ninety_minutes_later = time.add_minutes(90);
        assert_eq!(ninety_minutes_later.hour(), 12);
        assert_eq!(ninety_minutes_later.minute(), 0);
    }

    #[test]
    fn test_duration_operations() {
        use std::time::Duration;

        let time1 = Time::new(10, 30, 0).unwrap();
        let time2 = Time::new(11, 45, 30).unwrap();

        let duration = time2 - time1.clone();
        assert_eq!(duration.as_secs(), 75 * 60 + 30);

        let new_time = time1.add_duration(Duration::from_secs(3600)).unwrap();
        assert_eq!(new_time.hour(), 11);
    }

    #[test]
    fn test_parsing() {
        // ISO format
        let time = Time::parse_iso("14:30:45").unwrap();
        assert_eq!(time.hour(), 14);
        assert_eq!(time.minute(), 30);
        assert_eq!(time.second(), 45);

        // With fractional seconds
        let time_millis = Time::parse_iso("10:15:20.500").unwrap();
        assert_eq!(time_millis.millisecond(), 500);

        // Short format
        let time_short = Time::parse_iso("09:45").unwrap();
        assert_eq!(time_short.hour(), 9);
        assert_eq!(time_short.minute(), 45);
        assert_eq!(time_short.second(), 0);

        // 12-hour format
        let time_12h = Time::parse_12h("3:30:00 PM").unwrap();
        assert_eq!(time_12h.hour(), 15);

        let midnight_12h = Time::parse_12h("12:00:00 AM").unwrap();
        assert_eq!(midnight_12h.hour(), 0);

        let noon_12h = Time::parse_12h("12:00:00 PM").unwrap();
        assert_eq!(noon_12h.hour(), 12);
    }

    #[test]
    fn test_formatting() {
        let time = Time::new(14, 30, 45).unwrap();

        assert_eq!(time.to_iso_string(), "14:30:45");
        assert_eq!(time.to_12h_string(), "2:30:45 PM");

        assert_eq!(time.format("HH:mm:ss"), "14:30:45");
        assert_eq!(time.format("h:mm A"), "2:30 PM");
        assert_eq!(time.format("HH:mm"), "14:30");

        let with_millis = Time::with_millis(10, 15, 20, 500).unwrap();
        assert_eq!(with_millis.to_iso_string(), "10:15:20.5");
        assert_eq!(with_millis.format("HH:mm:ss.SSS"), "10:15:20.500");
    }

    #[test]
    fn test_rounding() {
        let time = Time::with_millis(14, 35, 45, 600).unwrap();

        let rounded_hour = time.round_to_hour();
        assert_eq!(rounded_hour.hour(), 15);
        assert_eq!(rounded_hour.minute(), 0);

        let rounded_minute = time.round_to_minute();
        assert_eq!(rounded_minute.minute(), 36);
        assert_eq!(rounded_minute.second(), 0);

        let rounded_second = time.round_to_second();
        assert_eq!(rounded_second.second(), 46);
        assert_eq!(rounded_second.nanosecond(), 0);
    }

    #[test]
    fn test_truncation() {
        let time = Time::with_millis(14, 35, 45, 600).unwrap();

        let truncated_hour = time.truncate_to_hour();
        assert_eq!(truncated_hour.hour(), 14);
        assert_eq!(truncated_hour.minute(), 0);
        assert_eq!(truncated_hour.second(), 0);

        let truncated_minute = time.truncate_to_minute();
        assert_eq!(truncated_minute.minute(), 35);
        assert_eq!(truncated_minute.second(), 0);

        let truncated_second = time.truncate_to_second();
        assert_eq!(truncated_second.second(), 45);
        assert_eq!(truncated_second.nanosecond(), 0);
    }

    #[test]
    fn test_time_of_day() {
        assert!(Time::new(8, 0, 0).unwrap().is_morning());
        assert!(Time::new(14, 0, 0).unwrap().is_afternoon());
        assert!(Time::new(19, 0, 0).unwrap().is_evening());
        assert!(Time::new(23, 0, 0).unwrap().is_night());
        assert!(Time::new(3, 0, 0).unwrap().is_night());
    }

    #[test]
    fn test_is_between() {
        let time = Time::new(14, 0, 0).unwrap();
        let start = Time::new(13, 0, 0).unwrap();
        let end = Time::new(15, 0, 0).unwrap();

        assert!(time.is_between(&start, &end));

        // Test wraparound
        let night_time = Time::new(1, 0, 0).unwrap();
        let night_start = Time::new(23, 0, 0).unwrap();
        let night_end = Time::new(3, 0, 0).unwrap();

        assert!(night_time.is_between(&night_start, &night_end));
    }
}
