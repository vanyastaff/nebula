extern crate alloc;

use alloc::sync::Arc;
use core::borrow::Borrow;
use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::{Add, AddAssign, Sub, SubAssign};
use core::str::FromStr;
use core::time::Duration as StdDuration;
#[cfg(feature = "std")]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use once_cell::sync::OnceCell;

use chrono::{Local, NaiveDateTime, TimeZone, Utc};

use super::date::{Date, DateInner};
use super::time::{Time, TimeInner};

use crate::core::{ValueError, ValueResult};

/// Internal datetime storage
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DateTimeInner {
    /// Date component
    pub date: DateInner,
    /// Time component
    pub time: TimeInner,
}

impl DateTimeInner {
    /// Creates a new DateTimeInner
    pub fn new(date: DateInner, time: TimeInner) -> Self {
        Self { date, time }
    }

    /// Converts to chrono NaiveDateTime
    pub fn to_naive(&self) -> NaiveDateTime {
        NaiveDateTime::new(self.date.to_naive(), self.time.to_naive())
    }

    /// Creates from chrono NaiveDateTime
    pub fn from_naive(dt: NaiveDateTime) -> Self {
        Self {
            date: DateInner::from_naive(dt.date()),
            time: TimeInner::from_naive(dt.time()),
        }
    }
}

/// A high-performance, feature-rich datetime type
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DateTime {
    /// Internal datetime storage
    inner: Arc<DateTimeInner>,

    /// Cached Unix timestamp for O(1) access
    #[cfg_attr(feature = "serde", serde(skip))]
    timestamp_cache: OnceCell<i64>,

    /// Cached ISO string for O(1) access
    #[cfg_attr(feature = "serde", serde(skip))]
    iso_string_cache: OnceCell<String>,
}

impl DateTime {
    // ==================== Constructors ====================

    /// Creates a new DateTime from components
    pub fn new(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> ValueResult<Self> {
        let date = Date::new(year, month, day)?;
        let time = Time::new(hour, minute, second)?;
        Ok(Self::from_date_time(date, time))
    }

    /// Creates a new DateTime with nanoseconds
    pub fn with_nanos(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        nanos: u32,
    ) -> ValueResult<Self> {
        let date = Date::new(year, month, day)?;
        let time = Time::with_nanos(hour, minute, second, nanos)?;
        Ok(Self::from_date_time(date, time))
    }

    /// Creates from Date and Time
    pub fn from_date_time(date: Date, time: Time) -> Self {
        let inner = DateTimeInner::new(date.as_inner().clone(), *time.as_inner());

        Self {
            inner: Arc::new(inner),
            timestamp_cache: OnceCell::new(),
            iso_string_cache: OnceCell::new(),
        }
    }

    /// Creates DateTime for current moment (local timezone)
    #[cfg(feature = "std")]
    pub fn now() -> Self {
        let now = Local::now().naive_local();
        Self::from_naive_datetime(now)
    }

    /// Creates DateTime for current moment (UTC)
    #[cfg(feature = "std")]
    pub fn now_utc() -> Self {
        let now = Utc::now().naive_utc();
        Self::from_naive_datetime(now)
    }

    /// Creates from Unix timestamp (seconds since epoch)
    pub fn from_timestamp(timestamp: i64) -> ValueResult<Self> {
        Self::from_timestamp_nanos(timestamp * 1_000_000_000)
    }

    /// Creates from Unix timestamp in milliseconds
    pub fn from_timestamp_millis(millis: i64) -> ValueResult<Self> {
        Self::from_timestamp_nanos(millis * 1_000_000)
    }

    /// Creates from Unix timestamp in microseconds
    pub fn from_timestamp_micros(micros: i64) -> ValueResult<Self> {
        Self::from_timestamp_nanos(micros * 1000)
    }

    /// Creates from Unix timestamp in nanoseconds
    pub fn from_timestamp_nanos(nanos: i64) -> ValueResult<Self> {
        let seconds = nanos / 1_000_000_000;
        let nano_part = (nanos % 1_000_000_000) as u32;

        chrono::DateTime::from_timestamp(seconds, nano_part)
            .map(|dt| Self::from_naive_datetime(dt.naive_utc()))
            .ok_or_else(|| {
                ValueError::out_of_range(
                    format!("{} nanos", nanos),
                    "valid timestamp range",
                    "valid timestamp range",
                )
            })
    }

    /// Creates from SystemTime
    #[cfg(feature = "std")]
    pub fn from_system_time(st: SystemTime) -> ValueResult<Self> {
        let duration = st
            .duration_since(UNIX_EPOCH)
            .map_err(|e| ValueError::validation(format!("System time error: {}", e)))?;

        let nanos = duration.as_nanos();
        if nanos > i64::MAX as u128 {
            return Err(ValueError::out_of_range(
                "SystemTime",
                "valid range",
                format!("{} (too far in the future)", i64::MAX),
            ));
        }

        Self::from_timestamp_nanos(nanos as i64)
    }

    /// Creates from chrono NaiveDateTime
    pub fn from_naive_datetime(dt: NaiveDateTime) -> Self {
        Self {
            inner: Arc::new(DateTimeInner::from_naive(dt)),
            timestamp_cache: OnceCell::new(),
            iso_string_cache: OnceCell::new(),
        }
    }

    /// Creates DateTime at Unix epoch
    pub fn unix_epoch() -> Self {
        Self::new(1970, 1, 1, 0, 0, 0).expect("1970-01-01 00:00:00 is always a valid datetime")
    }

    /// Parses from ISO 8601 string
    pub fn parse_iso(s: &str) -> ValueResult<Self> {
        // Handle various ISO formats
        let s = s.trim();

        // Split by 'T' or space
        let (date_part, time_part) = if let Some(t_pos) = s.find('T') {
            (&s[..t_pos], &s[t_pos + 1..])
        } else if let Some(space_pos) = s.find(' ') {
            (&s[..space_pos], &s[space_pos + 1..])
        } else {
            // Date only, assume midnight
            let date = Date::parse_iso(s)?;
            return Ok(Self::from_date_time(date, Time::midnight()));
        };

        // Remove timezone info if present
        let time_part = if let Some(plus_pos) = time_part.find('+') {
            &time_part[..plus_pos]
        } else if let Some(minus_pos) = time_part.rfind('-') {
            &time_part[..minus_pos]
        } else if let Some(z_pos) = time_part.find('Z') {
            &time_part[..z_pos]
        } else {
            time_part
        };

        let date = Date::parse_iso(date_part)?;
        let time = Time::parse_iso(time_part)?;

        Ok(Self::from_date_time(date, time))
    }

    /// Parses from RFC 3339 string
    pub fn parse_rfc3339(s: &str) -> ValueResult<Self> {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| Self::from_naive_datetime(dt.naive_utc()))
            .map_err(|e| ValueError::parse_error("RFC 3339 datetime", format!("{}: {}", s, e)))
    }

    /// Parses from RFC 2822 string
    pub fn parse_rfc2822(s: &str) -> ValueResult<Self> {
        chrono::DateTime::parse_from_rfc2822(s)
            .map(|dt| Self::from_naive_datetime(dt.naive_utc()))
            .map_err(|e| ValueError::parse_error("RFC 2822 datetime", format!("{}: {}", s, e)))
    }

    /// Parses with custom format
    pub fn parse_from_str(s: &str, fmt: &str) -> ValueResult<Self> {
        NaiveDateTime::parse_from_str(s, fmt)
            .map(Self::from_naive_datetime)
            .map_err(|e| {
                ValueError::parse_error(
                    format!("datetime with format '{}'", fmt),
                    format!("{}: {}", s, e),
                )
            })
    }

    // ==================== Basic Properties ====================

    /// Returns the date component
    pub fn date(&self) -> Date {
        Date::new(
            self.inner.date.year,
            self.inner.date.month as u32,
            self.inner.date.day as u32,
        )
        .expect("datetime date component is always valid")
    }

    /// Returns the time component
    pub fn time(&self) -> Time {
        Time::from_nanos(self.inner.time.nanos).expect("datetime time component is always valid")
    }

    /// Returns the year
    #[inline]
    pub fn year(&self) -> i32 {
        self.inner.date.year
    }

    /// Returns the month (1-12)
    #[inline]
    pub fn month(&self) -> u32 {
        self.inner.date.month as u32
    }

    /// Returns the day of month (1-31)
    #[inline]
    pub fn day(&self) -> u32 {
        self.inner.date.day as u32
    }

    /// Returns the hour (0-23)
    #[inline]
    pub fn hour(&self) -> u32 {
        self.inner.time.hour() as u32
    }

    /// Returns the minute (0-59)
    #[inline]
    pub fn minute(&self) -> u32 {
        self.inner.time.minute() as u32
    }

    /// Returns the second (0-59)
    #[inline]
    pub fn second(&self) -> u32 {
        self.inner.time.second() as u32
    }

    /// Returns the nanosecond (0-999_999_999)
    #[inline]
    pub fn nanosecond(&self) -> u32 {
        self.inner.time.nanosecond()
    }

    /// Returns the millisecond (0-999)
    #[inline]
    pub fn millisecond(&self) -> u32 {
        self.inner.time.millisecond()
    }

    /// Returns the microsecond (0-999_999)
    #[inline]
    pub fn microsecond(&self) -> u32 {
        self.inner.time.microsecond()
    }

    /// Returns the day of week (Monday = 0, Sunday = 6)
    #[inline]
    pub fn day_of_week(&self) -> u8 {
        self.inner.date.day_of_week()
    }

    /// Returns the day of year (1-366)
    #[inline]
    pub fn day_of_year(&self) -> u16 {
        self.inner.date.day_of_year()
    }

    /// Returns Unix timestamp in seconds
    pub fn timestamp(&self) -> i64 {
        *self
            .timestamp_cache
            .get_or_init(|| self.inner.to_naive().and_utc().timestamp())
    }

    /// Returns Unix timestamp in milliseconds
    #[inline]
    pub fn timestamp_millis(&self) -> i64 {
        self.timestamp() * 1000 + self.millisecond() as i64
    }

    /// Returns Unix timestamp in microseconds
    #[inline]
    pub fn timestamp_micros(&self) -> i64 {
        self.timestamp() * 1_000_000 + self.microsecond() as i64
    }

    /// Returns Unix timestamp in nanoseconds
    #[inline]
    pub fn timestamp_nanos(&self) -> i64 {
        self.timestamp() * 1_000_000_000 + self.nanosecond() as i64
    }

    // ==================== DateTime Arithmetic ====================

    /// Adds a duration
    pub fn add_duration(&self, duration: StdDuration) -> ValueResult<Self> {
        let nanos = self.timestamp_nanos() + duration.as_nanos() as i64;
        Self::from_timestamp_nanos(nanos)
    }

    /// Subtracts a duration
    pub fn sub_duration(&self, duration: StdDuration) -> ValueResult<Self> {
        let nanos = self.timestamp_nanos() - duration.as_nanos() as i64;
        Self::from_timestamp_nanos(nanos)
    }

    /// Adds days
    pub fn add_days(&self, days: i64) -> ValueResult<Self> {
        let new_date = self.date().add_days(days)?;
        Ok(Self::from_date_time(new_date, self.time()))
    }

    /// Adds months
    pub fn add_months(&self, months: i32) -> ValueResult<Self> {
        let new_date = self.date().add_months(months)?;
        Ok(Self::from_date_time(new_date, self.time()))
    }

    /// Adds years
    pub fn add_years(&self, years: i32) -> ValueResult<Self> {
        let new_date = self.date().add_years(years)?;
        Ok(Self::from_date_time(new_date, self.time()))
    }

    /// Adds hours
    pub fn add_hours(&self, hours: i64) -> ValueResult<Self> {
        self.add_duration(StdDuration::from_secs((hours * 3600) as u64))
    }

    /// Adds minutes
    pub fn add_minutes(&self, minutes: i64) -> ValueResult<Self> {
        self.add_duration(StdDuration::from_secs((minutes * 60) as u64))
    }

    /// Adds seconds
    pub fn add_seconds(&self, seconds: i64) -> ValueResult<Self> {
        self.add_duration(StdDuration::from_secs(seconds as u64))
    }

    /// Returns duration between two datetimes
    pub fn duration_between(&self, other: &DateTime) -> StdDuration {
        let diff_nanos = (self.timestamp_nanos() - other.timestamp_nanos()).abs();
        StdDuration::from_nanos(diff_nanos as u64)
    }

    /// Returns signed duration to another datetime
    pub fn signed_duration_to(&self, other: &DateTime) -> i64 {
        other.timestamp_nanos() - self.timestamp_nanos()
    }

    // ==================== Component Updates ====================

    /// Returns a new DateTime with the specified date
    pub fn with_date(&self, date: Date) -> Self {
        Self::from_date_time(date, self.time())
    }

    /// Returns a new DateTime with the specified time
    pub fn with_time(&self, time: Time) -> Self {
        Self::from_date_time(self.date(), time)
    }

    /// Returns a new DateTime with the specified year
    pub fn with_year(&self, year: i32) -> ValueResult<Self> {
        let new_date = Date::new(year, self.month(), self.day())?;
        Ok(self.with_date(new_date))
    }

    /// Returns a new DateTime with the specified month
    pub fn with_month(&self, month: u32) -> ValueResult<Self> {
        let new_date = Date::new(self.year(), month, self.day())?;
        Ok(self.with_date(new_date))
    }

    /// Returns a new DateTime with the specified day
    pub fn with_day(&self, day: u32) -> ValueResult<Self> {
        let new_date = Date::new(self.year(), self.month(), day)?;
        Ok(self.with_date(new_date))
    }

    /// Returns a new DateTime with the specified hour
    pub fn with_hour(&self, hour: u32) -> ValueResult<Self> {
        let new_time = self.time().with_hour(hour)?;
        Ok(self.with_time(new_time))
    }

    /// Returns a new DateTime with the specified minute
    pub fn with_minute(&self, minute: u32) -> ValueResult<Self> {
        let new_time = self.time().with_minute(minute)?;
        Ok(self.with_time(new_time))
    }

    /// Returns a new DateTime with the specified second
    pub fn with_second(&self, second: u32) -> ValueResult<Self> {
        let new_time = self.time().with_second(second)?;
        Ok(self.with_time(new_time))
    }

    /// Returns a new DateTime with the specified nanosecond
    pub fn with_nanosecond(&self, nano: u32) -> ValueResult<Self> {
        let new_time = self.time().with_nanosecond(nano)?;
        Ok(self.with_time(new_time))
    }

    // ==================== Truncation ====================

    /// Truncates to day precision (midnight)
    pub fn truncate_to_day(&self) -> Self {
        Self::from_date_time(self.date(), Time::midnight())
    }

    /// Truncates to hour precision
    pub fn truncate_to_hour(&self) -> Self {
        Self::from_date_time(self.date(), self.time().truncate_to_hour())
    }

    /// Truncates to minute precision
    pub fn truncate_to_minute(&self) -> Self {
        Self::from_date_time(self.date(), self.time().truncate_to_minute())
    }

    /// Truncates to second precision
    pub fn truncate_to_second(&self) -> Self {
        Self::from_date_time(self.date(), self.time().truncate_to_second())
    }

    // ==================== Formatting ====================

    /// Returns ISO 8601 datetime string
    pub fn to_iso_string(&self) -> &str {
        self.iso_string_cache.get_or_init(|| {
            format!(
                "{}T{}",
                self.date().to_iso_string(),
                self.time().to_iso_string()
            )
        })
    }

    /// Returns RFC 3339 string
    pub fn to_rfc3339(&self) -> String {
        format!(
            "{}T{}Z",
            self.date().to_iso_string(),
            self.time().to_iso_string()
        )
    }

    /// Returns RFC 2822 string
    #[cfg(feature = "std")]
    pub fn to_rfc2822(&self) -> String {
        let dt = Utc
            .timestamp_opt(self.timestamp(), self.nanosecond())
            .unwrap();
        dt.to_rfc2822()
    }

    /// Formats the datetime using a custom format string
    pub fn format(&self, fmt: &str) -> String {
        let mut result = fmt.to_string();

        // Date components
        result = self.date().format(&result);

        // Time components
        result = self.time().format(&result);

        // DateTime specific formats
        result = result.replace("ISO", self.to_iso_string());
        result = result.replace("RFC3339", &self.to_rfc3339());

        result
    }

    /// Returns a human-readable string
    pub fn to_human_string(&self) -> String {
        format!(
            "{} at {}",
            self.date().to_human_string(),
            self.time().to_human_string()
        )
    }

    /// Returns relative datetime string
    #[cfg(feature = "std")]
    pub fn to_relative_string(&self) -> String {
        let now = DateTime::now();
        let diff_seconds = self.signed_duration_to(&now) / 1_000_000_000;
        let abs_seconds = diff_seconds.abs();

        if abs_seconds < 60 {
            return "just now".to_string();
        }

        let minutes = abs_seconds / 60;
        let hours = minutes / 60;
        let days = hours / 24;

        let (amount, unit) = if days > 0 {
            if days == 1 {
                return if diff_seconds > 0 {
                    "tomorrow".to_string()
                } else {
                    "yesterday".to_string()
                };
            }
            (days, "day")
        } else if hours > 0 {
            (hours, "hour")
        } else {
            (minutes, "minute")
        };

        let plural = if amount == 1 { "" } else { "s" };

        if diff_seconds > 0 {
            format!("in {} {}{}", amount, unit, plural)
        } else {
            format!("{} {}{} ago", amount, unit, plural)
        }
    }

    // ==================== Conversions ====================

    /// Converts to chrono NaiveDateTime
    #[inline]
    pub fn to_naive(&self) -> NaiveDateTime {
        self.inner.to_naive()
    }

    /// Converts to SystemTime
    #[cfg(feature = "std")]
    pub fn to_system_time(&self) -> SystemTime {
        UNIX_EPOCH + StdDuration::from_nanos(self.timestamp_nanos() as u64)
    }

    // ==================== Validation ====================

    /// Checks if the datetime is in the past
    #[cfg(feature = "std")]
    pub fn is_past(&self) -> bool {
        let now = DateTime::now();
        self < &now
    }

    /// Checks if the datetime is in the future
    #[cfg(feature = "std")]
    pub fn is_future(&self) -> bool {
        let now = DateTime::now();
        self > &now
    }

    /// Checks if the datetime is now (within 1 second)
    #[cfg(feature = "std")]
    pub fn is_now(&self) -> bool {
        let now = DateTime::now();
        let diff = self.duration_between(&now);
        diff.as_secs() < 1
    }
}

// ==================== Trait Implementations ====================

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_iso_string())
    }
}

#[cfg(feature = "std")]
impl Default for DateTime {
    fn default() -> Self {
        Self::now()
    }
}

impl FromStr for DateTime {
    type Err = ValueError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_iso(s)
    }
}

impl PartialEq for DateTime {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for DateTime {}

impl PartialOrd for DateTime {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DateTime {
    fn cmp(&self, other: &Self) -> Ordering {
        self.inner.cmp(&other.inner)
    }
}

impl Hash for DateTime {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}

impl Borrow<DateTimeInner> for DateTime {
    fn borrow(&self) -> &DateTimeInner {
        &self.inner
    }
}

// ==================== From Implementations ====================

#[cfg(feature = "std")]
impl From<SystemTime> for DateTime {
    fn from(st: SystemTime) -> Self {
        Self::from_system_time(st).unwrap_or_else(|_| Self::unix_epoch())
    }
}

#[cfg(feature = "std")]
impl From<DateTime> for SystemTime {
    fn from(dt: DateTime) -> Self {
        dt.to_system_time()
    }
}

impl From<NaiveDateTime> for DateTime {
    fn from(dt: NaiveDateTime) -> Self {
        Self::from_naive_datetime(dt)
    }
}

impl From<DateTime> for NaiveDateTime {
    fn from(dt: DateTime) -> Self {
        dt.to_naive()
    }
}

impl<Tz: TimeZone> From<chrono::DateTime<Tz>> for DateTime {
    fn from(dt: chrono::DateTime<Tz>) -> Self {
        Self::from_naive_datetime(dt.naive_utc())
    }
}

// ==================== Arithmetic Operations ====================

impl Add<StdDuration> for DateTime {
    type Output = ValueResult<DateTime>;

    fn add(self, duration: StdDuration) -> Self::Output {
        self.add_duration(duration)
    }
}

impl Add<StdDuration> for &DateTime {
    type Output = ValueResult<DateTime>;

    fn add(self, duration: StdDuration) -> Self::Output {
        self.add_duration(duration)
    }
}

impl Sub<StdDuration> for DateTime {
    type Output = ValueResult<DateTime>;

    fn sub(self, duration: StdDuration) -> Self::Output {
        self.sub_duration(duration)
    }
}

impl Sub<StdDuration> for &DateTime {
    type Output = ValueResult<DateTime>;

    fn sub(self, duration: StdDuration) -> Self::Output {
        self.sub_duration(duration)
    }
}

impl Sub<DateTime> for DateTime {
    type Output = StdDuration;

    fn sub(self, other: DateTime) -> Self::Output {
        self.duration_between(&other)
    }
}

impl Sub<&DateTime> for DateTime {
    type Output = StdDuration;

    fn sub(self, other: &DateTime) -> Self::Output {
        self.duration_between(other)
    }
}

impl Sub<DateTime> for &DateTime {
    type Output = StdDuration;

    fn sub(self, other: DateTime) -> Self::Output {
        self.duration_between(&other)
    }
}

impl Sub<&DateTime> for &DateTime {
    type Output = StdDuration;

    fn sub(self, other: &DateTime) -> Self::Output {
        self.duration_between(other)
    }
}

impl AddAssign<StdDuration> for DateTime {
    fn add_assign(&mut self, duration: StdDuration) {
        if let Ok(new_dt) = self.add_duration(duration) {
            *self = new_dt;
        }
    }
}

impl SubAssign<StdDuration> for DateTime {
    fn sub_assign(&mut self, duration: StdDuration) {
        if let Ok(new_dt) = self.sub_duration(duration) {
            *self = new_dt;
        }
    }
}

// ==================== Send + Sync ====================

// Static assertions to ensure inner types are Send + Sync
// This catches issues at compile time if inner types change
static_assertions::assert_impl_all!(DateTimeInner: Send, Sync);
static_assertions::assert_impl_all!(std::sync::Arc<DateTimeInner>: Send, Sync);

unsafe impl Send for DateTime {}
unsafe impl Sync for DateTime {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_creation() {
        let dt = DateTime::new(2024, 12, 25, 14, 30, 45).unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 12);
        assert_eq!(dt.day(), 25);
        assert_eq!(dt.hour(), 14);
        assert_eq!(dt.minute(), 30);
        assert_eq!(dt.second(), 45);
    }

    #[test]
    fn test_from_components() {
        let date = Date::new(2024, 12, 25).unwrap();
        let time = Time::new(14, 30, 45).unwrap();
        let dt = DateTime::from_date_time(date.clone(), time.clone());

        assert_eq!(dt.date(), date);
        assert_eq!(dt.time(), time);
    }

    #[test]
    fn test_timestamp() {
        let dt = DateTime::new(1970, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(dt.timestamp(), 0);

        let dt2 = DateTime::new(1970, 1, 1, 0, 0, 1).unwrap();
        assert_eq!(dt2.timestamp(), 1);
    }

    #[test]
    fn test_parsing() {
        let dt = DateTime::parse_iso("2024-12-25T14:30:45").unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.hour(), 14);

        let dt2 = DateTime::parse_iso("2024-12-25 14:30:45").unwrap();
        assert_eq!(dt2, dt);

        let dt3 = DateTime::parse_iso("2024-12-25").unwrap();
        assert_eq!(dt3.hour(), 0);
    }

    #[test]
    fn test_arithmetic() {
        use std::time::Duration;

        let dt = DateTime::new(2024, 12, 25, 14, 30, 45).unwrap();

        let one_hour_later = dt.add_hours(1).unwrap();
        assert_eq!(one_hour_later.hour(), 15);

        let one_day_later = dt.add_days(1).unwrap();
        assert_eq!(one_day_later.day(), 26);

        let one_month_later = dt.add_months(1).unwrap();
        assert_eq!(one_month_later.month(), 1);
        assert_eq!(one_month_later.year(), 2025);

        let plus_duration = dt.add_duration(Duration::from_secs(3600)).unwrap();
        assert_eq!(plus_duration.hour(), 15);
    }

    #[test]
    fn test_formatting() {
        let dt = DateTime::new(2024, 12, 25, 14, 30, 45).unwrap();

        assert_eq!(dt.to_iso_string(), "2024-12-25T14:30:45");
        assert_eq!(dt.to_rfc3339(), "2024-12-25T14:30:45Z");

        assert_eq!(dt.format("YYYY-MM-DD HH:mm:ss"), "2024-12-25 14:30:45");
        assert_eq!(
            dt.format("MMM D, YYYY at h:mm A"),
            "Dec 25, 2024 at 2:30 PM"
        );
    }

    #[test]
    fn test_truncation() {
        let dt = DateTime::with_nanos(2024, 12, 25, 14, 35, 45, 123_456_789).unwrap();

        let day = dt.truncate_to_day();
        assert_eq!(day.hour(), 0);
        assert_eq!(day.minute(), 0);

        let hour = dt.truncate_to_hour();
        assert_eq!(hour.minute(), 0);
        assert_eq!(hour.second(), 0);

        let minute = dt.truncate_to_minute();
        assert_eq!(minute.second(), 0);
        assert_eq!(minute.nanosecond(), 0);

        let second = dt.truncate_to_second();
        assert_eq!(second.nanosecond(), 0);
    }

    #[test]
    fn test_component_updates() {
        let dt = DateTime::new(2024, 12, 25, 14, 30, 45).unwrap();

        let new_year = dt.with_year(2025).unwrap();
        assert_eq!(new_year.year(), 2025);

        let new_hour = dt.with_hour(16).unwrap();
        assert_eq!(new_hour.hour(), 16);

        let new_date = dt.with_date(Date::new(2025, 1, 1).unwrap());
        assert_eq!(new_date.year(), 2025);
        assert_eq!(new_date.month(), 1);
        assert_eq!(new_date.day(), 1);
    }
}
