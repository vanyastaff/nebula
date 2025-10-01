extern crate alloc;

use core::borrow::Borrow;
use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::{Add, Sub};
use core::str::FromStr;
use alloc::sync::Arc;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use once_cell::sync::OnceCell;

use chrono::{Datelike, Duration, Local, NaiveDate, Utc, Weekday};

use crate::core::{NebulaError, ValueResult};

/// Internal date storage
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DateInner {
    pub(crate) year: i32,
    pub(crate) month: u8,
    pub(crate) day: u8,
}

impl DateInner {
    /// Creates a new DateInner with validation
    pub fn new(year: i32, month: u32, day: u32) -> ValueResult<Self> {
        // Validate month
        if month == 0 || month > 12 {
            return Err(NebulaError::validation(format!("Invalid month: {}", month)));
        }

        // Validate day
        let max_day = Self::days_in_month(year, month);
        if day == 0 || day > max_day {
            return Err(NebulaError::validation(format!("Invalid date: year={}, month={}, day={}", year, month, day)));
        }

        Ok(Self {
            year,
            month: month as u8,
            day: day as u8,
        })
    }

    /// Returns the number of days in a month
    pub fn days_in_month(year: i32, month: u32) -> u32 {
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

    /// Checks if a year is a leap year
    pub fn is_leap_year(year: i32) -> bool {
        (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
    }

    /// Converts to chrono NaiveDate
    pub fn to_naive(&self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.year, self.month as u32, self.day as u32).expect("Valid date")
    }

    /// Creates from chrono NaiveDate
    pub fn from_naive(date: NaiveDate) -> Self {
        Self {
            year: date.year(),
            month: date.month() as u8,
            day: date.day() as u8,
        }
    }

    /// Calculates day of year (1-366)
    pub fn day_of_year(&self) -> u16 {
        let mut days = 0u16;
        for m in 1..self.month {
            days += Self::days_in_month(self.year, m as u32) as u16;
        }
        days + self.day as u16
    }

    /// Calculates day of week (Monday = 0, Sunday = 6)
    pub fn day_of_week(&self) -> u8 {
        let date = self.to_naive();
        match date.weekday() {
            Weekday::Mon => 0,
            Weekday::Tue => 1,
            Weekday::Wed => 2,
            Weekday::Thu => 3,
            Weekday::Fri => 4,
            Weekday::Sat => 5,
            Weekday::Sun => 6,
        }
    }
}

/// A high-performance, feature-rich date type
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Date {
    /// Internal date storage
    inner: Arc<DateInner>,

    /// Cached ISO string for O(1) access
    #[cfg_attr(feature = "serde", serde(skip))]
    iso_string_cache: OnceCell<String>,

    /// Cached day of year for O(1) access
    #[cfg_attr(feature = "serde", serde(skip))]
    day_of_year_cache: OnceCell<u16>,
}

impl Date {
    // ==================== Constructors ====================

    /// Creates a new Date
    pub fn new(year: i32, month: u32, day: u32) -> ValueResult<Self> {
        let inner = DateInner::new(year, month, day)?;
        Ok(Self {
            inner: Arc::new(inner),
            iso_string_cache: OnceCell::new(),
            day_of_year_cache: OnceCell::new(),
        })
    }

    /// Creates Date for today (local timezone)
    #[cfg(feature = "std")]
    pub fn today() -> Self {
        let today = Local::now().naive_local().date();
        Self::from_naive_date(today)
    }

    /// Creates Date for today (UTC)
    #[cfg(feature = "std")]
    pub fn today_utc() -> Self {
        let today = Utc::now().naive_utc().date();
        Self::from_naive_date(today)
    }

    /// Creates from Julian day number
    pub fn from_julian_day(jd: i32) -> ValueResult<Self> {
        NaiveDate::from_num_days_from_ce_opt(jd - 1721425)
            .map(|d| Self::from_naive_date(d))
            .ok_or_else(|| NebulaError::validation(format!("Invalid Julian day: {}", jd)))
    }

    /// Creates from day of year
    pub fn from_year_day(year: i32, day_of_year: u16) -> ValueResult<Self> {
        if day_of_year == 0 || day_of_year > 366 {
            return Err(NebulaError::validation(format!("Invalid day_of_year: {}", day_of_year)));
        }

        let is_leap = DateInner::is_leap_year(year);
        if !is_leap && day_of_year > 365 {
            return Err(NebulaError::validation(format!("Invalid day_of_year: {}", day_of_year)));
        }

        let mut remaining = day_of_year;
        for month in 1..=12 {
            let days_in_month = DateInner::days_in_month(year, month) as u16;
            if remaining <= days_in_month {
                return Self::new(year, month, remaining as u32);
            }
            remaining -= days_in_month;
        }

        Err(NebulaError::validation(format!("Invalid day_of_year: {}", day_of_year)))
    }

    /// Creates from ISO week date (year, week, day)
    pub fn from_iso_week(year: i32, week: u32, weekday: u32) -> ValueResult<Self> {
        if week == 0 || week > 53 || weekday == 0 || weekday > 7 {
            return Err(NebulaError::validation(format!("Invalid ISO week date: {}-W{:02}-{}", year, week, weekday)));
        }

        NaiveDate::from_isoywd_opt(year, week, Weekday::try_from((weekday - 1) as u8).unwrap())
            .map(|d| Self::from_naive_date(d))
            .ok_or_else(|| NebulaError::validation(format!("Invalid ISO week date: {}-W{:02}-{}", year, week, weekday)))
    }

    /// Creates from chrono NaiveDate
    pub fn from_naive_date(date: NaiveDate) -> Self {
        Self {
            inner: Arc::new(DateInner::from_naive(date)),
            iso_string_cache: OnceCell::new(),
            day_of_year_cache: OnceCell::new(),
        }
    }

    /// Parses from a string in format YYYY-MM-DD
    pub fn parse_iso(s: &str) -> ValueResult<Self> {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() != 3 {
            return Err(NebulaError::validation(format!("Invalid ISO date format: {}", s)));
        }

        let year = parts[0].parse::<i32>().map_err(|_| NebulaError::validation(format!("Invalid year: {}", parts[0])))?;

        let month = parts[1].parse::<u32>().map_err(|_| NebulaError::validation(format!("Invalid month: {}", parts[1])))?;

        let day = parts[2].parse::<u32>().map_err(|_| NebulaError::validation(format!("Invalid day: {}", parts[2])))?;

        Self::new(year, month, day)
    }

    // ==================== Basic Properties ====================

    /// Returns the year
    #[inline]
    pub fn year(&self) -> i32 {
        self.inner.year
    }

    /// Returns the month (1-12)
    #[inline]
    pub fn month(&self) -> u32 {
        self.inner.month as u32
    }

    /// Returns the day of month (1-31)
    #[inline]
    pub fn day(&self) -> u32 {
        self.inner.day as u32
    }

    /// Returns the day of year (1-366)
    #[inline]
    pub fn day_of_year(&self) -> u16 {
        *self
            .day_of_year_cache
            .get_or_init(|| self.inner.day_of_year())
    }

    /// Returns the day of week (Monday = 0, Sunday = 6)
    #[inline]
    pub fn day_of_week(&self) -> u8 {
        self.inner.day_of_week()
    }

    /// Returns the weekday name
    pub fn weekday_name(&self) -> &'static str {
        match self.day_of_week() {
            0 => "Monday",
            1 => "Tuesday",
            2 => "Wednesday",
            3 => "Thursday",
            4 => "Friday",
            5 => "Saturday",
            6 => "Sunday",
            _ => unreachable!(),
        }
    }

    /// Returns the month name
    pub fn month_name(&self) -> &'static str {
        match self.month() {
            1 => "January",
            2 => "February",
            3 => "March",
            4 => "April",
            5 => "May",
            6 => "June",
            7 => "July",
            8 => "August",
            9 => "September",
            10 => "October",
            11 => "November",
            12 => "December",
            _ => unreachable!(),
        }
    }

    /// Returns the quarter (1-4)
    #[inline]
    pub fn quarter(&self) -> u8 {
        ((self.month() - 1) / 3 + 1) as u8
    }

    /// Returns the ISO week number
    pub fn iso_week(&self) -> u32 {
        self.inner.to_naive().iso_week().week()
    }

    /// Checks if this is a leap year
    #[inline]
    pub fn is_leap_year(&self) -> bool {
        DateInner::is_leap_year(self.year())
    }

    /// Checks if this is a weekend (Saturday or Sunday)
    #[inline]
    pub fn is_weekend(&self) -> bool {
        let dow = self.day_of_week();
        dow == 5 || dow == 6
    }

    /// Checks if this is a weekday (Monday-Friday)
    #[inline]
    pub fn is_weekday(&self) -> bool {
        !self.is_weekend()
    }

    /// Returns the Julian day number
    pub fn julian_day(&self) -> i32 {
        self.inner.to_naive().num_days_from_ce() + 1721425
    }

    /// Gets the internal representation
    #[inline]
    pub(crate) fn as_inner(&self) -> &DateInner {
        &self.inner
    }

    // ==================== Date Arithmetic ====================

    /// Adds days to the date
    pub fn add_days(&self, days: i64) -> ValueResult<Self> {
        let date = self.inner.to_naive();
        date.checked_add_signed(Duration::days(days))
            .map(|d| Self::from_naive_date(d))
            .ok_or_else(|| NebulaError::validation("Date arithmetic overflow"))
    }

    /// Adds weeks to the date
    #[inline]
    pub fn add_weeks(&self, weeks: i64) -> ValueResult<Self> {
        self.add_days(weeks * 7)
    }

    /// Adds months to the date
    pub fn add_months(&self, months: i32) -> ValueResult<Self> {
        let total_months = self.year() * 12 + self.month() as i32 - 1 + months;
        let new_year = total_months.div_euclid(12);
        let new_month = (total_months.rem_euclid(12) + 1) as u32;

        let max_day = DateInner::days_in_month(new_year, new_month);
        let new_day = self.day().min(max_day);

        Self::new(new_year, new_month, new_day)
    }

    /// Adds years to the date
    pub fn add_years(&self, years: i32) -> ValueResult<Self> {
        let new_year = self.year() + years;

        // Handle leap year edge case for Feb 29
        let new_day = if self.month() == 2 && self.day() == 29 && !DateInner::is_leap_year(new_year)
        {
            28
        } else {
            self.day()
        };

        Self::new(new_year, self.month(), new_day)
    }

    /// Subtracts another date, returning the number of days between them
    pub fn days_between(&self, other: &Date) -> i64 {
        let jd1 = self.julian_day() as i64;
        let jd2 = other.julian_day() as i64;
        jd1 - jd2
    }

    /// Returns the first day of the month
    pub fn first_of_month(&self) -> ValueResult<Self> {
        Self::new(self.year(), self.month(), 1)
    }

    /// Returns the last day of the month
    pub fn last_of_month(&self) -> ValueResult<Self> {
        let last_day = DateInner::days_in_month(self.year(), self.month());
        Self::new(self.year(), self.month(), last_day)
    }

    /// Returns the first day of the year
    pub fn first_of_year(&self) -> ValueResult<Self> {
        Self::new(self.year(), 1, 1)
    }

    /// Returns the last day of the year
    pub fn last_of_year(&self) -> ValueResult<Self> {
        Self::new(self.year(), 12, 31)
    }

    /// Returns the next weekday
    pub fn next_weekday(&self) -> ValueResult<Self> {
        let dow = self.day_of_week();
        let days_to_add = match dow {
            5 => 3, // Saturday -> Monday
            6 => 2, // Sunday -> Monday
            _ => 1,
        };
        self.add_days(days_to_add)
    }

    /// Returns the previous weekday
    pub fn prev_weekday(&self) -> ValueResult<Self> {
        let dow = self.day_of_week();
        let days_to_sub = match dow {
            0 => 3, // Monday -> Friday
            6 => 2, // Sunday -> Friday
            _ => 1,
        };
        self.add_days(-days_to_sub)
    }

    // ==================== Formatting ====================

    /// Returns ISO 8601 date string (YYYY-MM-DD)
    pub fn to_iso_string(&self) -> &str {
        self.iso_string_cache
            .get_or_init(|| format!("{:04}-{:02}-{:02}", self.year(), self.month(), self.day()))
    }

    /// Formats the date using a custom format string
    pub fn format(&self, fmt: &str) -> String {
        let chars: Vec<char> = fmt.chars().collect();
        let mut i = 0;
        let mut out = String::with_capacity(fmt.len() + 8);
        while i < chars.len() {
            // Helper to check a token at current position
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

            if starts_with("dddd") {
                out.push_str(self.weekday_name());
                i += 4;
            } else if starts_with("MMMM") {
                out.push_str(self.month_name());
                i += 4;
            } else if starts_with("YYYY") {
                out.push_str(&format!("{:04}", self.year()));
                i += 4;
            } else if starts_with("ddd") {
                out.push_str(&self.weekday_name()[..3]);
                i += 3;
            } else if starts_with("MMM") {
                out.push_str(&self.month_name()[..3]);
                i += 3;
            } else if starts_with("DD") {
                out.push_str(&format!("{:02}", self.day()));
                i += 2;
            } else if starts_with("MM") {
                out.push_str(&format!("{:02}", self.month()));
                i += 2;
            } else if starts_with("YY") {
                out.push_str(&format!("{:02}", self.year() % 100));
                i += 2;
            } else if starts_with("D") {
                out.push_str(&self.day().to_string());
                i += 1;
            } else if starts_with("M") {
                out.push_str(&self.month().to_string());
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
        format!("{} {}, {}", self.month_name(), self.day(), self.year())
    }

    /// Returns relative date string (e.g., "today", "yesterday", "3 days ago")
    #[cfg(feature = "std")]
    pub fn to_relative_string(&self) -> String {
        let today = Date::today();
        let days = today.days_between(self);

        match days {
            0 => "today".to_string(),
            1 => "yesterday".to_string(),
            -1 => "tomorrow".to_string(),
            2..=6 => format!("{} days ago", days),
            -6..=-2 => format!("in {} days", -days),
            7..=13 => "last week".to_string(),
            -13..=-7 => "next week".to_string(),
            14..=27 => format!("{} weeks ago", days / 7),
            -27..=-14 => format!("in {} weeks", -days / 7),
            28..=364 => format!("{} months ago", days / 30),
            -364..=-28 => format!("in {} months", -days / 30),
            _ => {
                let years = days / 365;
                if years > 0 {
                    format!("{} years ago", years)
                } else {
                    format!("in {} years", -years)
                }
            }
        }
    }

    // ==================== Conversions ====================

    /// Converts to chrono NaiveDate
    #[inline]
    pub fn to_naive(&self) -> NaiveDate {
        self.inner.to_naive()
    }

    /// Creates a Date at the start of the Unix epoch
    pub fn unix_epoch() -> ValueResult<Self> {
        Self::new(1970, 1, 1)
    }

    /// Returns the number of days since Unix epoch
    pub fn days_since_epoch(&self) -> i64 {
        self.days_between(&Self::unix_epoch().unwrap())
    }

    // ==================== Validation ====================

    /// Checks if a date is valid
    pub fn is_valid(year: i32, month: u32, day: u32) -> bool {
        DateInner::new(year, month, day).is_ok()
    }

    /// Checks if the date is in the past
    #[cfg(feature = "std")]
    pub fn is_past(&self) -> bool {
        let today = Date::today();
        self < &today
    }

    /// Checks if the date is in the future
    #[cfg(feature = "std")]
    pub fn is_future(&self) -> bool {
        let today = Date::today();
        self > &today
    }

    /// Checks if the date is today
    #[cfg(feature = "std")]
    pub fn is_today(&self) -> bool {
        let today = Date::today();
        self == &today
    }
}

// ==================== Trait Implementations ====================

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_iso_string())
    }
}

#[cfg(feature = "std")]
impl Default for Date {
    fn default() -> Self {
        Self::today()
    }
}

impl FromStr for Date {
    type Err = NebulaError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_iso(s)
    }
}

impl PartialEq for Date {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for Date {}

impl PartialOrd for Date {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Date {
    fn cmp(&self, other: &Self) -> Ordering {
        self.inner.cmp(&other.inner)
    }
}

impl Hash for Date {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}

impl Borrow<DateInner> for Date {
    fn borrow(&self) -> &DateInner {
        &self.inner
    }
}

// ==================== Arithmetic Operations ====================

impl Add<i64> for Date {
    type Output = ValueResult<Date>;

    fn add(self, days: i64) -> Self::Output {
        self.add_days(days)
    }
}

impl Add<i64> for &Date {
    type Output = ValueResult<Date>;

    fn add(self, days: i64) -> Self::Output {
        self.add_days(days)
    }
}

impl Sub<i64> for Date {
    type Output = ValueResult<Date>;

    fn sub(self, days: i64) -> Self::Output {
        self.add_days(-days)
    }
}

impl Sub<i64> for &Date {
    type Output = ValueResult<Date>;

    fn sub(self, days: i64) -> Self::Output {
        self.add_days(-days)
    }
}

impl Sub<Date> for Date {
    type Output = i64;

    fn sub(self, other: Date) -> Self::Output {
        self.days_between(&other)
    }
}

impl Sub<&Date> for Date {
    type Output = i64;

    fn sub(self, other: &Date) -> Self::Output {
        self.days_between(other)
    }
}

impl Sub<Date> for &Date {
    type Output = i64;

    fn sub(self, other: Date) -> Self::Output {
        self.days_between(&other)
    }
}

impl Sub<&Date> for &Date {
    type Output = i64;

    fn sub(self, other: &Date) -> Self::Output {
        self.days_between(other)
    }
}

// ==================== Send + Sync ====================

unsafe impl Send for Date {}
unsafe impl Sync for Date {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_creation() {
        let date = Date::new(2024, 12, 25).unwrap();
        assert_eq!(date.year(), 2024);
        assert_eq!(date.month(), 12);
        assert_eq!(date.day(), 25);
    }

    #[test]
    fn test_leap_year() {
        assert!(Date::new(2024, 2, 29).is_ok());
        assert!(Date::new(2023, 2, 29).is_err());
        assert!(Date::new(2000, 2, 29).is_ok());
        assert!(Date::new(1900, 2, 29).is_err());
    }

    #[test]
    fn test_day_of_week() {
        let date = Date::new(2024, 12, 25).unwrap(); // Wednesday
        assert_eq!(date.day_of_week(), 2);
        assert_eq!(date.weekday_name(), "Wednesday");
    }

    #[test]
    fn test_arithmetic() {
        let date = Date::new(2024, 12, 25).unwrap();

        let next_week = date.add_days(7).unwrap();
        assert_eq!(next_week.year(), 2025);
        assert_eq!(next_week.month(), 1);
        assert_eq!(next_week.day(), 1);

        let next_month = date.add_months(1).unwrap();
        assert_eq!(next_month.year(), 2025);
        assert_eq!(next_month.month(), 1);
        assert_eq!(next_month.day(), 25);
    }

    #[test]
    fn test_formatting() {
        let date = Date::new(2024, 12, 25).unwrap();

        assert_eq!(date.to_iso_string(), "2024-12-25");
        assert_eq!(date.format("YYYY/MM/DD"), "2024/12/25");
        assert_eq!(date.format("MMM D, YYYY"), "Dec 25, 2024");
        assert_eq!(date.to_human_string(), "December 25, 2024");
    }

    #[test]
    fn test_parsing() {
        let date = Date::parse_iso("2024-12-25").unwrap();
        assert_eq!(date.year(), 2024);
        assert_eq!(date.month(), 12);
        assert_eq!(date.day(), 25);

        let parsed: Date = "2024-01-01".parse().unwrap();
        assert_eq!(parsed.year(), 2024);
    }
}
